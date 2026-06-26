# Worktree Session Attribution Implementation Plan (Spec Fase B)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Model routing (deep-plan):** Plan = Opus xhigh | Exec atômica = Sonnet high | Exec complexa = Opus xhigh | Review por task = Opus xhigh | Revisão final = Opus xhigh
>
> Each task is tagged `[atômica]` (Sonnet high) or `[complexa]` (Opus xhigh).

**Goal:** Match each branch's AI sessions to the worktree where the branch is checked out, so `loops resume <branch>` recovers session excerpts in bare+worktree layouts.

**Architecture:** After Fase A discovers repos, `open_loops` runs `git worktree list --porcelain` once per repo, parses it into entries, and resolves each `OpenLoop.repo_path` to the branch's worktree when checked out (else the container/common-dir fallback). The session adapter (`ClaudeCode::excerpts`) already encodes `repo_path` into the `~/.claude/projects/<encoded>` lookup, so the correct cwd flows through unchanged. The `--porcelain` parser is extracted as a pure shared helper (Spec §4) and reused by `loops worktrees`.

**Tech Stack:** Rust (edition per repo), `anyhow`, `chrono`, git via shell-out (`std::process::Command`), `tempfile`/`assert_cmd`/`predicates` for tests.

## Global Constraints

- Tolerant degradation: a `git worktree list` failure → empty map + `eprintln!("warning: …")`, never abort (matches `scan`/`open_loops` existing tolerance). [Spec §4]
- No layout heuristics: branch→worktree mapping comes only from git, never from folder names. [Spec §1]
- Single field: keep one `OpenLoop.repo_path` (worktree-when-checked-out, container-else); do **not** add a `session_path`. `repo_path` never enters `key()`/cache. [Spec §3]
- Error/warning messages in English, actionable. [CLAUDE.md]
- Parsing is pure where testable: a bad/blank line is skipped, never panics. [CLAUDE.md]
- Tests build real git repos in tempdir via `src/testutil.rs`. [CLAUDE.md]
- Coverage gate 70% (core target 85%) stays green. [Spec §6]
- Dev env: `just` may be absent — run `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt` directly. [CLAUDE.md Cursor Cloud notes]

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/scanner.rs` | Modify | Add `WorktreeEntry`, `parse_worktree_porcelain` (pure), `worktree_map`; wire `open_loops` to resolve `repo_path` per branch. |
| `src/testutil.rs` | Modify | Add `add_worktree_with_commit` helper (open loop living in its own worktree). |
| `src/worktrees.rs` | Modify | Replace inline block-parse with the shared `parse_worktree_porcelain` (DRY, Spec §4). |
| `tests/cli.rs` | Modify | E2E: `loops resume <branch-in-worktree>` includes the session excerpt. |
| `docs/features.md` | Modify | Document per-worktree session matching. |
| `docs/decisions/0005-repo-discovery-via-git.md` | Modify | Note that Fase B is implemented. |
| `CHANGELOG.md` | Modify | Add unreleased entries. |
| `src/sessions/claude_code.rs` | **Verify only** | Confirm `excerpts` assumes nothing beyond the encoded cwd (it doesn't). No code change. |
| `src/sessions/mod.rs`, `src/cli.rs` | **No change** | `cli.rs` already passes `lp.repo_path` to `excerpts`/`git_log`/`diffstat`. |

### Interfaces produced (relied on by later tasks)

```rust
// src/scanner.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: std::path::PathBuf,
    pub branch: Option<String>, // short name (refs/heads/ stripped); None when detached/bare
    pub bare: bool,
    pub prunable: bool,
}
pub fn parse_worktree_porcelain(out: &str) -> Vec<WorktreeEntry>;
pub fn worktree_map(repo: &std::path::Path) -> anyhow::Result<std::collections::HashMap<String, std::path::PathBuf>>;

// src/testutil.rs
pub fn add_worktree_with_commit(container: &std::path::Path, dir_name: &str, branch: &str, file: &str);
```

---

## Task 1: Pure `--porcelain` parser  `[atômica]`

**Files:**
- Modify: `src/scanner.rs` (add `WorktreeEntry` + `parse_worktree_porcelain` after `git_common_dir`, ~line 119; add tests in the `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing (pure over a string).
- Produces: `WorktreeEntry`, `parse_worktree_porcelain(&str) -> Vec<WorktreeEntry>` (used by Task 2 and Task 5).

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `src/scanner.rs`:

```rust
    #[test]
    fn parse_worktree_porcelain_extracts_branches_and_flags() {
        let out = "\
worktree /home/u/app/main
HEAD aaaaaaaa
branch refs/heads/main

worktree /home/u/app/feat-x
HEAD bbbbbbbb
branch refs/heads/feat/x

worktree /home/u/app/detached
HEAD cccccccc
detached

worktree /home/u/app/.bare
bare
";
        let entries = parse_worktree_porcelain(out);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[0].path, std::path::PathBuf::from("/home/u/app/main"));
        assert_eq!(entries[1].branch.as_deref(), Some("feat/x")); // slash preserved
        assert_eq!(entries[2].branch, None); // detached
        assert!(entries[3].bare);
        assert_eq!(entries[3].branch, None);
    }

    #[test]
    fn parse_worktree_porcelain_marks_prunable_and_handles_empty() {
        assert!(parse_worktree_porcelain("").is_empty());
        let out = "worktree /gone\nprunable gitdir file points to non-existent location\n";
        let entries = parse_worktree_porcelain(out);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].prunable);
        assert_eq!(entries[0].branch, None);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib parse_worktree_porcelain`
Expected: FAIL — `cannot find function 'parse_worktree_porcelain'`.

- [ ] **Step 3: Write the implementation**

Insert in `src/scanner.rs` right after `git_common_dir` (after line 119):

```rust
/// One entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// Short branch name (`refs/heads/` stripped). `None` when detached or bare.
    pub branch: Option<String>,
    pub bare: bool,
    pub prunable: bool,
}

/// Parses `git worktree list --porcelain` into entries.
///
/// Pure over the git output: a new entry starts at each `worktree ` line; the
/// `HEAD`/`detached`/`locked` lines leave `branch` as `None`. Tolerant — unknown
/// or blank lines are ignored, never panics.
pub fn parse_worktree_porcelain(out: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            if let Some(e) = current.take() {
                entries.push(e);
            }
            current = Some(WorktreeEntry {
                path: PathBuf::from(p),
                branch: None,
                bare: false,
                prunable: false,
            });
        } else if let Some(e) = current.as_mut() {
            if let Some(b) = line.strip_prefix("branch ") {
                e.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line == "bare" {
                e.bare = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                e.prunable = true;
            }
        }
    }
    if let Some(e) = current.take() {
        entries.push(e);
    }
    entries
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib parse_worktree_porcelain`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): parse git worktree list --porcelain into entries"
```

---

## Task 2: `worktree_map` — branch → worktree path  `[atômica]`

**Files:**
- Modify: `src/scanner.rs` (add `worktree_map` after `parse_worktree_porcelain`; add one test)

**Interfaces:**
- Consumes: `parse_worktree_porcelain` (Task 1), `git` (existing), `testutil::init_bare_worktree_container`/`add_named_worktree` (existing).
- Produces: `worktree_map(&Path) -> Result<HashMap<String, PathBuf>>` (used by Task 3).

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/scanner.rs`:

```rust
    #[test]
    fn worktree_map_maps_checked_out_branches_to_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container); // main worktree at container/main
        testutil::add_named_worktree(&container, "dev", "dev"); // dev worktree at container/dev

        let map = worktree_map(&container).unwrap();
        assert_eq!(map.get("main"), Some(&container.join("main")));
        assert_eq!(map.get("dev"), Some(&container.join("dev")));
        // the `.bare` entry is filtered out (no branch / bare)
        assert!(!map.values().any(|p| p.ends_with(".bare")));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib worktree_map_maps_checked_out_branches_to_paths`
Expected: FAIL — `cannot find function 'worktree_map'`.

- [ ] **Step 3: Write the implementation**

Insert in `src/scanner.rs` right after `parse_worktree_porcelain`:

```rust
/// Maps each checked-out branch to the absolute path of its worktree.
///
/// Bare and detached entries are dropped (no branch to key on). git proscribes
/// the same branch in two worktrees, so the map is 1:1.
///
/// # Errors
///
/// Returns `Err` if `git worktree list` fails.
pub fn worktree_map(repo: &Path) -> Result<std::collections::HashMap<String, PathBuf>> {
    let raw = git(repo, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_porcelain(&raw)
        .into_iter()
        .filter(|e| !e.bare)
        .filter_map(|e| e.branch.map(|b| (b, e.path)))
        .collect())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib worktree_map_maps_checked_out_branches_to_paths`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): map branches to worktree paths"
```

---

## Task 3: Resolve `repo_path` per branch in `open_loops`  `[complexa]`

**Files:**
- Modify: `src/testutil.rs` (add `add_worktree_with_commit` after `add_named_worktree`, ~line 79)
- Modify: `src/scanner.rs` (`open_loops`, lines 177-234; add 3 tests)

**Interfaces:**
- Consumes: `worktree_map` (Task 2), existing `OpenLoop` struct.
- Produces: `OpenLoop.repo_path` = worktree path when the branch is checked out, else the repo/container path. `testutil::add_worktree_with_commit(container, dir_name, branch, file)`.

- [ ] **Step 1: Add the test helper**

Insert in `src/testutil.rs` after `add_named_worktree` (after line 79):

```rust
/// Adds a `container/<dir_name>/` worktree on a NEW unmerged branch carrying one
/// exclusive commit — an open loop living in its own worktree (author layout).
pub fn add_worktree_with_commit(container: &Path, dir_name: &str, branch: &str, file: &str) {
    add_named_worktree(container, dir_name, branch);
    let wt = container.join(dir_name);
    std::fs::write(wt.join(file), file).unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", &format!("wip {branch}")]);
}
```

- [ ] **Step 2: Write the failing tests**

Add to `mod tests` in `src/scanner.rs`:

```rust
    #[test]
    fn open_loops_sets_repo_path_to_worktree_when_branch_checked_out() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        testutil::add_worktree_with_commit(&container, "feat-x", "feat/x", "x.txt");

        let loops = open_loops(&container, "root").unwrap();
        let lp = loops
            .iter()
            .find(|l| l.branch == "feat/x")
            .expect("feat/x loop");
        assert_eq!(lp.repo_path, container.join("feat-x"));
    }

    #[test]
    fn open_loops_falls_back_to_container_when_branch_has_no_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        // feat/y exists in the store but is NOT checked out in any worktree
        testutil::add_branch_on_bare(&container.join(".bare"), "feat/y", "y.txt");

        let loops = open_loops(&container, "root").unwrap();
        let lp = loops
            .iter()
            .find(|l| l.branch == "feat/y")
            .expect("feat/y loop");
        assert_eq!(lp.repo_path, container);
    }

    #[test]
    fn open_loops_normal_repo_keeps_repo_path_as_repo_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt"); // checks out feat/x then back to main
        let loops = open_loops(&repo, "root").unwrap();
        assert_eq!(loops[0].branch, "feat/x");
        assert_eq!(loops[0].repo_path, repo); // not checked out in a worktree → fallback
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib open_loops_sets_repo_path open_loops_falls_back open_loops_normal_repo_keeps`
Expected: FAIL — `open_loops_sets_repo_path_to_worktree_when_branch_checked_out` asserts `repo_path == container.join("feat-x")` but current code always sets `repo_path = repo` (the container).

- [ ] **Step 4: Wire `open_loops`**

Replace the body of `open_loops` in `src/scanner.rs` (lines 177-234) with:

```rust
pub fn open_loops(repo: &Path, root_label: &str) -> Result<Vec<OpenLoop>> {
    let default = default_branch(repo)?;
    let common_dir = git_common_dir(repo)?;
    let repo_name = repo_name_from_common_dir(&common_dir);
    let worktrees = worktree_map(repo).unwrap_or_else(|e| {
        eprintln!(
            "warning: git worktree list failed in {}: {e:#}; session matching falls back to the repo path",
            repo.display()
        );
        std::collections::HashMap::new()
    });
    let merged: std::collections::HashSet<String> = git(
        repo,
        &["branch", "--merged", &default, "--format=%(refname:short)"],
    )?
    .lines()
    .map(|s| s.trim().to_string())
    .collect();
    let raw = git(
        repo,
        &[
            "for-each-ref",
            "refs/heads",
            "--format=%(refname:short)%09%(objectname)%09%(committerdate:iso8601-strict)",
        ],
    )?;
    let mut result = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split('\t');
        let (Some(branch), Some(sha), Some(date)) = (parts.next(), parts.next(), parts.next())
        else {
            eprintln!("warning: unexpected line from git for-each-ref ignored: {line:?}");
            continue;
        };
        if branch == default || merged.contains(branch) {
            continue;
        }
        let counts = git(
            repo,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{default}...{branch}"),
            ],
        )?;
        let mut c = counts.split_whitespace();
        let behind: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
        let ahead: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
        let last_commit = DateTime::parse_from_rfc3339(date)
            .with_context(|| format!("invalid date from git: {date}"))?
            .with_timezone(&Utc);
        let repo_path = worktrees
            .get(branch)
            .cloned()
            .unwrap_or_else(|| repo.to_path_buf());
        result.push(OpenLoop {
            root_label: root_label.to_string(),
            repo_name: repo_name.clone(),
            repo_path,
            branch: branch.to_string(),
            head_sha: sha.to_string(),
            last_commit,
            ahead,
            behind,
        });
    }
    Ok(result)
}
```

Notes:
- The only logical changes vs. the original are: (1) build `worktrees` once with a tolerant fallback, and (2) `repo_path` resolved from `worktrees.get(branch)` instead of the hard-coded `repo.to_path_buf()`.
- `worktrees.get(branch)` accepts `&str` (HashMap key `String: Borrow<str>`).
- The total-failure path (`worktree list` errors) and the no-worktree path converge on the same fallback expression, so the no-worktree test exercises the fallback branch.

- [ ] **Step 5: Run the full scanner suite to verify pass + no regression**

Run: `cargo test --lib scanner`
Expected: PASS — the 3 new tests plus the existing scanner tests (including `open_loops_uses_common_dir_repo_name_in_bare_layout`, which does not assert `repo_path` and stays green).

- [ ] **Step 6: Commit**

```bash
git add src/scanner.rs src/testutil.rs
git commit -m "feat(scanner): attribute repo_path to branch worktree"
```

---

## Task 4: E2E — `resume` matches sessions in the branch worktree  `[complexa]`

**Files:**
- Modify: `tests/cli.rs` (add one integration test at the end)
- Verify only: `src/sessions/claude_code.rs` — confirm `excerpts` only encodes the passed `repo_path` (`encode_project_path(repo_path)` at line 75); no code change.

**Interfaces:**
- Consumes: the full binary via `assert_cmd`, the `loops`/`git` helpers in `tests/cli.rs`, `repo_path` attribution (Task 3).
- Produces: end-to-end evidence that a session recorded under the worktree's encoded path reaches the resume output.

- [ ] **Step 1: Write the failing test**

Add at the end of `tests/cli.rs`:

```rust
#[test]
fn resume_includes_session_excerpt_for_branch_in_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    let container = root.join("my-app");

    // bare + worktree container (inline git: tests/cli.rs has its own git helper)
    let bare = container.join(".bare");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-b", "main"]);
    std::fs::write(container.join(".git"), "gitdir: ./.bare\n").unwrap();
    let main = container.join("main");
    git(
        &container,
        &["worktree", "add", "-b", "main", main.to_str().unwrap()],
    );
    std::fs::write(main.join("a.txt"), "a").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "init"]);
    // feature branch checked out in its OWN worktree directory
    let feat = container.join("feat-login");
    git(
        &container,
        &["worktree", "add", "-b", "feat/login", feat.to_str().unwrap()],
    );
    std::fs::write(feat.join("b.txt"), "b").unwrap();
    git(&feat, &["add", "."]);
    git(&feat, &["commit", "-m", "feat: login wip"]);

    // fake Claude Code session under the ENCODED WORKTREE path (not the container)
    let sessions = tmp.path().join("ai-sessions");
    let encoded = feat.to_string_lossy().replace(['/', '.'], "-");
    let proj = sessions.join(&encoded);
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("s.jsonl"),
        concat!(
            r#"{"type":"user","message":{"content":"resume the login feature please"}}"#,
            "\n",
        ),
    )
    .unwrap();

    loops(&home).arg("init").arg(&root).assert().success();

    // point llm_command at `cat` and sessions_dir at our fake projects dir (in place)
    let cfg_path = home.join("config.toml");
    let raw = std::fs::read_to_string(&cfg_path).unwrap();
    let rewritten: String = raw
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("sessions_dir") {
                format!("sessions_dir = \"{}\"", sessions.display())
            } else if l.trim_start().starts_with("llm_command") {
                "llm_command = \"cat\"".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&cfg_path, rewritten + "\n").unwrap();

    // resume distills (cat echoes the prompt) AND carries the worktree session excerpt
    loops(&home)
        .args(["resume", "feat/login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("resume the login feature please"));
}
```

- [ ] **Step 2: Run the test to verify it fails on `main` without Task 3**

Run: `cargo test --test cli resume_includes_session_excerpt_for_branch_in_worktree`
Expected on a tree WITHOUT Task 3: FAIL — `repo_path` would be the container, the session lives under the worktree's encoded path, so the excerpt text is absent from stdout. (With Task 3 already merged this proves the wiring; run it to confirm GREEN.)

- [ ] **Step 3: Confirm `excerpts` needs no change**

Read `src/sessions/claude_code.rs:75` — `self.projects_dir.join(encode_project_path(repo_path))`. It encodes only the passed `repo_path` and reads that directory; nothing else is assumed. No edit required. Record this in the commit body.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test cli resume_includes_session_excerpt_for_branch_in_worktree`
Expected: PASS.

If flaky (only if git reports a non-canonical worktree path on this platform): canonicalize before encoding — `let feat = std::fs::canonicalize(&feat).unwrap();` immediately after the worktree commit, before computing `encoded`. On Linux tempdirs the constructed path is already canonical, so the direct form is expected to pass.

- [ ] **Step 5: Commit**

```bash
git add tests/cli.rs
git commit -m "test(cli): resume matches sessions in branch worktree"
```

---

## Task 5: DRY — reuse the shared porcelain parser in `worktrees()`  `[complexa]`

Spec §4 sanctions extracting the `--porcelain` parse into a shared helper now that both features have landed. This removes the duplicate block-parser inside `worktrees()` (`src/worktrees.rs:105-127`) and routes it through `scanner::parse_worktree_porcelain`. Behavior-preserving; gated by the existing worktree suite.

**Files:**
- Modify: `src/worktrees.rs` (import + replace the block loop in `worktrees`)

**Interfaces:**
- Consumes: `scanner::parse_worktree_porcelain` (Task 1).
- Produces: no API change — `worktrees()` keeps its signature and behavior.

- [ ] **Step 1: Run the existing worktree suite as the regression baseline**

Run: `cargo test --lib worktrees && cargo test --test cli worktrees`
Expected: PASS (records the green baseline before refactor).

- [ ] **Step 2: Update the import**

In `src/worktrees.rs`, line 2, change:

```rust
use crate::scanner::{default_branch, find_repos, git};
```

to:

```rust
use crate::scanner::{default_branch, find_repos, git, parse_worktree_porcelain};
```

- [ ] **Step 3: Replace the block-parse loop**

In `src/worktrees.rs`, replace the parsing region — from `let mut out = Vec::new();` / `let mut first = true;` / `for block in raw.split("\n\n") {` down through the `let is_main = first; first = false;` lines (lines 105-133) — with the entry-driven version. The result is:

```rust
    let mut out = Vec::new();
    let mut first = true;
    for entry in parse_worktree_porcelain(&raw) {
        if entry.bare {
            continue;
        }
        let wt_path = entry.path;
        let branch = entry.branch;
        let prunable = entry.prunable;
        let is_main = first;
        first = false;

        let (last_commit, dirty) = if prunable {
            (None, false)
        } else {
            let lc = git(&wt_path, &["log", "-1", "--format=%cI"])
                .ok()
                .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
                .map(|d| d.with_timezone(&Utc));
            let status = git(&wt_path, &["status", "--porcelain"]).unwrap_or_default();
            (lc, !status.trim().is_empty())
        };
        let merged = branch
            .as_ref()
            .map(|b| merged_set.contains(b))
            .unwrap_or(false);

        out.push(Worktree {
            repo_name: repo_name.clone(),
            repo_path: repo.to_path_buf(),
            worktree_path: wt_path,
            branch,
            last_commit,
            merged,
            dirty,
            prunable,
            is_main,
        });
    }
    Ok(out)
```

Notes:
- `is_main` semantics preserved: bare entries `continue` **before** consuming `first`, so the first non-bare entry is main — identical to the original.
- The `let raw = git(repo, &["worktree", "list", "--porcelain"])?;` line above stays; only the parse loop changes.

- [ ] **Step 4: Run the worktree suite to verify no regression**

Run: `cargo test --lib worktrees && cargo test --test cli worktrees`
Expected: PASS — same set as the Step 1 baseline.

- [ ] **Step 5: Commit**

```bash
git add src/worktrees.rs
git commit -m "refactor(worktrees): reuse shared porcelain parser"
```

---

## Task 6: Docs + ADR + CHANGELOG  `[atômica]`

**Files:**
- Modify: `docs/features.md` (after the `loops resume` confidence/dry-run block, ~line 56)
- Modify: `docs/decisions/0005-repo-discovery-via-git.md` (Consequences bullet, line 34)
- Modify: `CHANGELOG.md` (under `## unreleased`)

**Interfaces:** none (documentation).

- [ ] **Step 1: Document session attribution in `docs/features.md`**

Insert after the `--dry-run` paragraph (after line 56, before `## loops ignore`):

```markdown
Sessions are matched against the **worktree where the branch is checked out**.
In bare+worktree layouts each branch lives in its own directory, so `loops resume`
looks up the AI sessions recorded for that directory — not the container. A branch
with no worktree falls back to the repo path: commits and diffstat still distill,
but session excerpts may be empty (the AI never ran there).
```

- [ ] **Step 2: Update ADR 0005**

In `docs/decisions/0005-repo-discovery-via-git.md`, replace the last Consequences bullet (line 34):

```markdown
- Spec Fase B builds on this for per-worktree session attribution.
```

with:

```markdown
- Spec Fase B (implemented) attributes AI sessions per worktree: `open_loops`
  resolves `OpenLoop.repo_path` to the branch's worktree via `git worktree list`
  (fallback: container/common-dir). The common-dir stays the dedup/identity
  anchor; `repo_path` never enters the canonical key.
```

- [ ] **Step 3: Update `CHANGELOG.md`**

Under `## unreleased`, add to the `### Features` list:

```markdown
- Attribute AI sessions to the branch's worktree
```

and add a `### Internals` entry (the section already exists under `## unreleased`):

```markdown
- Share git worktree --porcelain parser between scanner and worktrees
```

(Dev env lacks `git-cliff`; entries are added by hand to match the existing unreleased style. `just changelog` regenerates on release.)

- [ ] **Step 4: Commit**

```bash
git add docs/features.md docs/decisions/0005-repo-discovery-via-git.md CHANGELOG.md
git commit -m "docs: document per-worktree session attribution"
```

---

## Final Verification (Opus xhigh)

Run the full gate and confirm every Definition of Done item from the spec:

- [ ] `cargo test` — entire suite green (lib + `tests/cli.rs`).
- [ ] `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] `cargo fmt --check` — clean (run `cargo fmt` to fix).
- [ ] `cargo llvm-cov` (or `just cov`) — coverage at/above gate (70% overall, core 85%); the new pure parser + map are highly testable and should not regress core.
- [ ] **Manual validation** (author env, `~/repo/pigz`): `loops resume <branch-em-worktree>` brings the session excerpts (the `## Sources` section lists the worktree's sessions, confidence rises above `low` when a session matches). A branch without a worktree still resumes without error.
- [ ] Regression: a normal repo's `loops resume` matches sessions exactly as before (Task 4 + `open_loops_normal_repo_keeps_repo_path_as_repo_dir`).

### Spec coverage check (run with fresh eyes against `docs/superpowers/specs/2026-06-25-worktree-session-attribution.md`)

| Spec DoD item | Task |
|---|---|
| `worktree_map` parses `--porcelain` (pure helper tested) | Task 1 (parser) + Task 2 (map) |
| `open_loops` resolves `repo_path` per branch (worktree else fallback) | Task 3 |
| `worktree list` failure → empty map + warning, degrades | Task 3 (Step 4 fallback + warning) |
| `claude_code.rs` `excerpts` assumes only the encoded cwd | Task 4 (Step 3, verify-only) |
| Tests: porcelain parse, worktree vs no-worktree, session integration, normal-repo regression | Tasks 1-4 |
| Manual: `loops resume <branch-in-worktree>` brings excerpts in `~/repo/pigz` | Final Verification |
| `docs/features.md` + ADR 0005 updated | Task 6 |
| `just lint` + `just fmt`; coverage gate | Final Verification |
| CHANGELOG updated | Task 6 |
```

