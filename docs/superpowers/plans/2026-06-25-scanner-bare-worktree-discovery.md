# Scanner Bare + Worktree Discovery (Spec Fase A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `find_repos` discover git repositories regardless of physical layout (normal `.git` dir, worktree `.git` file, bare store) by interrogating git and deduplicating by `--git-common-dir`, so `loops` lists branches in bare+worktree environments like `~/repo/pigz`.

**Architecture:** Extend `walk` with a layout-agnostic candidate predicate (`.git` exists OR bare structural probe). After FS walk, resolve each candidate via `git rev-parse --path-format=absolute --git-common-dir`, deduplicate by that absolute path, and derive `repo_name` from the common-dir with a pure function. Replace fixed `MAX_DEPTH = 3` with configurable `scan_depth` (default 4) threaded from `Config` through `scan` and `scan_worktrees`. Session attribution per worktree stays out of scope (Spec Fase B).

**Tech Stack:** Rust 2021, `git` CLI shell-out (ADR 0002), `serde`/`toml` for config, real git repos in tempdirs (`src/testutil.rs`), `assert_cmd` for e2e.

**Source spec:** `docs/superpowers/specs/2026-06-25-scanner-bare-worktree-discovery.md`

## Problem Summary (why this blocks everything)

Today `walk` only recognizes a repo when `dir/.git` is a **directory**:

```98:101:src/scanner.rs
fn walk(dir: &Path, depth: usize, repos: &mut Vec<PathBuf>) {
    if dir.join(".git").is_dir() {
        repos.push(dir.to_path_buf());
        return;
```

In the author's bare+worktree layout, `.git` is always a **file** pointing at `.bare/`; worktrees also use `.git` files. No `.git` directory exists anywhere in the tree, so `find_repos` returns **zero** repos. `repo_name` today comes from `repo.file_name()`, which would yield `.bare`, `main`, or `dev` instead of `pigz-api`. `MAX_DEPTH = 3` is also too shallow for `~/repo/pigz/back/pigz-api` (depth 3 from `~/repo`, at the limit).

**Design rule (locked):** never match on folder-name heuristics (`.bare`, worktree names). Ask git; deduplicate by common-dir.

## Scope

**In scope:** detection, canonicalization, dedup, `repo_name` from common-dir, `scan_depth` config, ADR 0005, docs, tests, CHANGELOG.

**Out of scope (Spec Fase B):** `worktree_map`, resolving `repo_path` per branch for session excerpts. Until Fase B, `loops resume` on a branch checked out only in a worktree may distill without AI session excerpts (git evidence still works).

## Global Constraints

- User-facing errors/warnings in **English**, actionable.
- Tolerant git parsing: bad candidate → warning, never abort scan (mirror `scan` today).
- Conventional Commits; subjects in English.
- `just lint` (clippy `-D warnings`) and `just fmt` clean before each commit.
- Coverage gate 70% total (core target 85%) — `just cov` must pass.
- Docs are part of Definition of Done (`docs/features.md`, `docs/configuration.md` are source of truth).
- `label_for_repo` continues to operate on the **representative** `repo_path` (container), not the common-dir.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/scanner.rs` | Modify | `is_repo_candidate`, `looks_like_bare`, `git_common_dir`, `repo_name_from_common_dir`, dedup in `find_repos`, `scan_depth` param, `open_loops` uses common-dir name |
| `src/config.rs` | Modify | `scan_depth: usize` field, default 4, serde + test |
| `src/cli.rs` | Modify | Pass `cfg.scan_depth` to `scanner::scan` and `worktrees::scan_worktrees` |
| `src/worktrees.rs` | Modify | `scan_worktrees(roots, scan_depth)`; update `find_repos` call site |
| `src/testutil.rs` | Modify | `init_bare_repo`, `init_bare_worktree_container`, `add_named_worktree` |
| `docs/decisions/0005-repo-discovery-via-git.md` | Create | ADR: interrogate git + dedup by common-dir |
| `docs/features.md` | Modify | Document layout-agnostic discovery |
| `docs/configuration.md` | Modify | `scan_depth` + bare `.bare` edge case |
| `CHANGELOG.md` | Modify | `just changelog` entry |
| `ROADMAP.md` | Modify | Check off Fase A items when done |
| `tests/cli.rs` | Modify | E2E: `loops` lists branches in bare+worktree fixture |

**Signature changes (breaking in-crate only):**

```rust
// Before
pub fn find_repos(roots: &[PathBuf]) -> Vec<PathBuf>
pub fn scan(roots: &[PathBuf], labels: &[(PathBuf, String)]) -> (Vec<OpenLoop>, Vec<String>)
pub fn scan_worktrees(roots: &[PathBuf]) -> (Vec<Worktree>, Vec<String>)

// After
pub fn find_repos(roots: &[PathBuf], scan_depth: usize) -> (Vec<PathBuf>, Vec<String>)
pub fn scan(roots: &[PathBuf], labels: &[(PathBuf, String)], scan_depth: usize) -> (Vec<OpenLoop>, Vec<String>)
pub fn scan_worktrees(roots: &[PathBuf], scan_depth: usize) -> (Vec<Worktree>, Vec<String>)
```

`open_loops` gains an internal call to `git_common_dir` for `repo_name`; no public signature change required if `repo_name` is resolved inside `open_loops`.

---

### Task 1: `repo_name_from_common_dir` — pure naming rule

**Files:**
- Modify: `src/scanner.rs`
- Test: `src/scanner.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub fn repo_name_from_common_dir(common_dir: &Path) -> String` (consumed by Task 5).

- [ ] **Step 1: Write the failing table test**

Add to `src/scanner.rs` `mod tests`:

```rust
#[test]
fn repo_name_from_common_dir_table() {
    use std::path::Path;

    let cases: &[(&str, &str)] = &[
        ("/home/u/pigz-api/.bare", "pigz-api"),
        ("/home/u/app/.git", "app"),
        ("/srv/git/foo.git", "foo"),
        ("/srv/git/myproject", "myproject"),
    ];
    for (common, want) in cases {
        assert_eq!(
            repo_name_from_common_dir(Path::new(common)),
            *want,
            "common_dir={common}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test repo_name_from_common_dir_table -- --nocapture`
Expected: FAIL — `repo_name_from_common_dir` not found

- [ ] **Step 3: Implement the pure function**

Add before `find_repos` in `src/scanner.rs`:

```rust
/// Derives a stable repo name from the absolute git common-dir (§5 of Spec Fase A).
pub fn repo_name_from_common_dir(common_dir: &Path) -> String {
    let base = common_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if base == ".git" || base == ".bare" {
        return common_dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(base);
    }
    base.strip_suffix(".git")
        .map(str::to_owned)
        .unwrap_or(base)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test repo_name_from_common_dir_table -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): derive repo_name from git common-dir"
```

---

### Task 2: `git_common_dir` shell-out helper

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn git_common_dir_resolves_normal_and_bare_pointer() {
    let tmp = tempfile::tempdir().unwrap();
    let normal = tmp.path().join("app");
    testutil::init_repo(&normal);
    let normal_common = git_common_dir(&normal).unwrap();
    assert!(normal_common.ends_with(".git"));

    let container = tmp.path().join("container");
    testutil::init_bare_worktree_container(&container);
    let bare_common = git_common_dir(&container).unwrap();
    assert!(bare_common.ends_with(".bare"));
}
```

This test depends on `init_bare_worktree_container` (Task 8). **Implement Task 8 helpers first** if running strictly in order, or stub the layout inline for this test only. Recommended order: Task 8 before Task 2 test, or combine Task 2+8.

- [ ] **Step 2: Run test — expect FAIL** (function missing and/or helper missing)

Run: `cargo test git_common_dir_resolves_normal_and_bare_pointer -- --nocapture`

- [ ] **Step 3: Implement `git_common_dir`**

```rust
/// Absolute path of the git common-dir for `path` (bare store / `.git` dir).
///
/// # Errors
///
/// Returns `Err` when `path` is not inside a git repository.
pub fn git_common_dir(path: &Path) -> Result<PathBuf> {
    let raw = git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(raw))
}
```

- [ ] **Step 4: Run test — expect PASS** (after Task 8 helper exists)

- [ ] **Step 5: Commit**

```bash
git add src/scanner.rs src/testutil.rs
git commit -m "feat(scanner): resolve git common-dir via rev-parse"
```

---

### Task 3: Layout-agnostic `walk` candidate detection

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Add bare probe + candidate predicate**

```rust
fn looks_like_bare(dir: &Path) -> bool {
    dir.join("HEAD").is_file()
        && dir.join("objects").is_dir()
        && dir.join("refs").is_dir()
}

fn is_repo_candidate(dir: &Path) -> bool {
    dir.join(".git").exists() || looks_like_bare(dir)
}
```

- [ ] **Step 2: Replace `walk` detection**

Change `walk` signature to accept `scan_depth`:

```rust
fn walk(dir: &Path, depth: usize, scan_depth: usize, candidates: &mut Vec<PathBuf>) {
    if is_repo_candidate(dir) {
        candidates.push(dir.to_path_buf());
        return; // early-return: do not descend into repo innards
    }
    if depth >= scan_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !path.is_dir() || name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        walk(&path, depth + 1, scan_depth, candidates);
    }
}
```

Remove `const MAX_DEPTH: usize = 3;`.

- [ ] **Step 3: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): detect repos via .git file and bare probe"
```

---

### Task 4: Dedup by common-dir in `find_repos`

**Files:**
- Modify: `src/scanner.rs`, `src/worktrees.rs` (call site compile fix later in Task 7)

- [ ] **Step 1: Write failing dedup test**

```rust
#[test]
fn find_repos_dedups_container_and_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let container = tmp.path().join("pigz-api");
    testutil::init_bare_worktree_container(&container);
    testutil::add_named_worktree(&container, "dev", "dev");
    testutil::add_branch_with_commit(
        &container.join("dev"),
        "feat/x",
        "x.txt",
    );
    // Manually walk into a worktree path — should NOT create a second repo entry
    let (repos, warnings) = find_repos(&[tmp.path().to_path_buf()], 4);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], container);
}
```

- [ ] **Step 2: Run test — expect FAIL**

Run: `cargo test find_repos_dedups_container_and_worktrees -- --nocapture`

- [ ] **Step 3: Implement dedup**

```rust
fn dedup_candidates(candidates: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<String>) {
    use std::collections::HashMap;
    let mut by_common: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut warnings = Vec::new();
    for candidate in candidates {
        match git_common_dir(&candidate) {
            Ok(common) => {
                by_common.entry(common).or_insert(candidate);
            }
            Err(e) => {
                warnings.push(format!("{}: {e:#}", candidate.display()));
            }
        }
    }
    let mut repos: Vec<PathBuf> = by_common.into_values().collect();
    repos.sort();
    (repos, warnings)
}

/// Walks roots up to `scan_depth` looking for git repo candidates, then
/// deduplicates by absolute `--git-common-dir`.
pub fn find_repos(roots: &[PathBuf], scan_depth: usize) -> (Vec<PathBuf>, Vec<String>) {
    let mut candidates = Vec::new();
    for root in roots {
        walk(root, 0, scan_depth, &mut candidates);
    }
    dedup_candidates(candidates)
}
```

- [ ] **Step 4: Fix existing `find_repos_finds_repos_up_to_depth_3_and_skips_hidden`**

Update call and depth semantics:

```rust
#[test]
fn find_repos_respects_scan_depth_and_skips_hidden() {
    let tmp = tempfile::tempdir().unwrap();
    testutil::init_repo(&tmp.path().join("a/b/c/repo-deep")); // depth 4
    testutil::init_repo(&tmp.path().join("a/b/repo-mid")); // depth 3
    testutil::init_repo(&tmp.path().join("repo-shallow")); // depth 1
    testutil::init_repo(&tmp.path().join(".hidden/repo3"));

    let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
    let names: Vec<_> = repos
        .iter()
        .filter_map(|r| r.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"repo-deep".to_string()));
    assert!(names.contains(&"repo-mid".to_string()));
    assert!(names.contains(&"repo-shallow".to_string()));
    assert!(!names.contains(&"repo3".to_string()));

    let (shallow, _) = find_repos(&[tmp.path().to_path_buf()], 2);
    let shallow_names: Vec<_> = shallow
        .iter()
        .filter_map(|r| r.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    assert!(!shallow_names.contains(&"repo-deep".to_string()));
    assert!(shallow_names.contains(&"repo-shallow".to_string()));
}
```

- [ ] **Step 5: Run tests — expect PASS**

Run: `cargo test find_repos -- --nocapture`

- [ ] **Step 6: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): deduplicate repo candidates by git common-dir"
```

---

### Task 5: `open_loops` uses common-dir for `repo_name`

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn open_loops_uses_common_dir_repo_name_in_bare_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let container = tmp.path().join("pigz-api");
    testutil::init_bare_worktree_container(&container);
    let dev = container.join("dev");
    testutil::add_named_worktree(&container, "dev", "dev");
    testutil::add_branch_with_commit(&dev, "feat/x", "x.txt");

    let loops = open_loops(&container, "root").unwrap();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].repo_name, "pigz-api");
    assert_eq!(loops[0].branch, "feat/x");
    assert_eq!(loops[0].key(), "root/pigz-api/feat/x");
}
```

- [ ] **Step 2: Run test — expect FAIL** (`repo_name` still `pigz-api` from file_name? Actually container basename is already pigz-api — use worktree path to prove)

Better negative control: call `open_loops` from a bare root directly:

```rust
#[test]
fn open_loops_bare_root_repo_name_strips_dot_git_suffix() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("foo.git");
    testutil::init_bare_repo(&bare);
    // seed main via clone pattern — see Task 8 helper `seed_bare_main`
    testutil::seed_bare_main(&bare);
    testutil::add_branch_on_bare(&bare, "feat/y", "y.txt");

    let loops = open_loops(&bare, "r").unwrap();
    assert_eq!(loops[0].repo_name, "foo");
}
```

- [ ] **Step 3: Replace `file_name()` with common-dir lookup in `open_loops`**

```rust
pub fn open_loops(repo: &Path, root_label: &str) -> Result<Vec<OpenLoop>> {
    let default = default_branch(repo)?;
    let common_dir = git_common_dir(repo)?;
    let repo_name = repo_name_from_common_dir(&common_dir);
    // ... rest unchanged, remove old repo_name = repo.file_name() block
}
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test open_loops -- --nocapture`

- [ ] **Step 5: Commit**

```bash
git add src/scanner.rs src/testutil.rs
git commit -m "feat(scanner): name repos from git common-dir in open_loops"
```

---

### Task 6: Thread `scan_depth` through `scan` and merge discovery warnings

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Update `scan` signature and body**

```rust
pub fn scan(
    roots: &[PathBuf],
    labels: &[(PathBuf, String)],
    scan_depth: usize,
) -> (Vec<OpenLoop>, Vec<String>) {
    let (repos, mut warnings) = find_repos(roots, scan_depth);
    let results: Vec<Result<Vec<OpenLoop>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| {
                let label = crate::config::label_for_repo(labels, repo);
                s.spawn(move || open_loops(repo, &label))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("panic while scanning repository")))
            })
            .collect()
    });
    let mut all = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut loops) => all.append(&mut loops),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
        }
    }
    (all, warnings)
}
```

- [ ] **Step 2: Fix `scan_aggregates_repos_and_reports_warning_without_aborting`**

```rust
let (loops, warnings) = scan(&[tmp.path().to_path_buf()], &labels, 4);
```

- [ ] **Step 3: Run `cargo test scanner::tests` — expect PASS**

- [ ] **Step 4: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): pass scan_depth through scan and merge discovery warnings"
```

---

### Task 7: `scan_depth` in `Config`

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn config_scan_depth_defaults_to_four() {
    let cfg = Config::default();
    assert_eq!(cfg.scan_depth, 4);
}

#[test]
fn config_scan_depth_roundtrips_from_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join("state"));
    let cfg = Config {
        scan_depth: 6,
        ..Config::default()
    };
    store.save(&cfg).unwrap();
    assert_eq!(store.load().unwrap().scan_depth, 6);
}
```

- [ ] **Step 2: Add field + default**

```rust
/// Maximum directory depth (from each root) to search for git repositories.
#[serde(default = "default_scan_depth")]
pub scan_depth: usize,

fn default_scan_depth() -> usize {
    4
}
```

Add `scan_depth: default_scan_depth()` to `Config::default()`.

- [ ] **Step 3: Run `cargo test config::tests` — expect PASS**

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add configurable scan_depth (default 4)"
```

---

### Task 8: Testutil helpers for bare + worktree layout

**Files:**
- Modify: `src/testutil.rs`

- [ ] **Step 1: Add helpers**

```rust
/// Creates a bare repository at `bare` (`git init --bare`).
pub fn init_bare_repo(bare: &Path) {
    std::fs::create_dir_all(bare).unwrap();
    git(bare, &["init", "--bare", "-b", "main"]);
}

/// Author layout: `container/.bare` + `container/.git` pointer + `main/` worktree with init commit.
pub fn init_bare_worktree_container(container: &Path) {
    std::fs::create_dir_all(container).unwrap();
    let bare = container.join(".bare");
    init_bare_repo(&bare);
    std::fs::write(container.join(".git"), "gitdir: ./.bare\n").unwrap();
    let main = container.join("main");
    git(
        container,
        &["worktree", "add", "-b", "main", main.to_str().unwrap()],
    );
    std::fs::write(main.join("README"), "init").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "init"]);
}

/// Adds `container/<name>/` worktree on a new branch (author layout).
pub fn add_named_worktree(container: &Path, name: &str, branch: &str) {
    let wt = container.join(name);
    git(
        container,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            wt.to_str().unwrap(),
        ],
    );
}

/// Creates `main` with one commit on a bare repo (no container pointer).
pub fn seed_bare_main(bare: &Path) {
    let tmp = bare.parent().unwrap().join("_seed");
    git(
        bare.parent().unwrap(),
        &["clone", bare.to_str().unwrap(), tmp.to_str().unwrap()],
    );
    std::fs::write(tmp.join("README"), "init").unwrap();
    git(&tmp, &["add", "."]);
    git(&tmp, &["commit", "-m", "init"]);
    git(&tmp, &["push", "origin", "main"]);
    std::fs::remove_dir_all(&tmp).ok();
}

/// Feature branch with exclusive commit, using a throwaway clone of `bare`.
pub fn add_branch_on_bare(bare: &Path, branch: &str, file: &str) {
    let tmp = bare.parent().unwrap().join("_wt");
    git(
        bare.parent().unwrap(),
        &["clone", bare.to_str().unwrap(), tmp.to_str().unwrap()],
    );
    add_branch_with_commit(&tmp, branch, file);
    git(&tmp, &["push", "origin", branch]);
    std::fs::remove_dir_all(&tmp).ok();
}
```

Keep the existing `add_worktree(repo, path, branch)` for normal-repo worktree tests unchanged.

- [ ] **Step 2: Run Task 1–5 tests — expect PASS**

- [ ] **Step 3: Commit**

```bash
git add src/testutil.rs
git commit -m "test: add bare and bare+worktree git fixtures"
```

---

### Task 9: Wire `scan_depth` in CLI and worktrees

**Files:**
- Modify: `src/cli.rs`, `src/worktrees.rs`

- [ ] **Step 1: Update `scanner::scan` call sites in `cli.rs`**

In `resolve_loop` and `run_list`:

```rust
let (found, warnings) = scanner::scan(&cfg.roots, &labels, cfg.scan_depth);
```

- [ ] **Step 2: Update `worktrees::scan_worktrees`**

```rust
pub fn scan_worktrees(roots: &[PathBuf], scan_depth: usize) -> (Vec<Worktree>, Vec<String>) {
    let (repos, mut warnings) = find_repos(roots, scan_depth);
    // ... rest unchanged
}
```

In `run_worktrees`:

```rust
let (wts, warnings) = worktrees::scan_worktrees(&cfg.roots, cfg.scan_depth);
```

- [ ] **Step 3: Fix any remaining `find_repos` / `scan` call sites**

Run: `rg 'find_repos\(|scanner::scan\(' src tests`
Expected: all sites pass `scan_depth`.

- [ ] **Step 4: Run full test suite**

Run: `just test`
Expected: all green (fix compile errors in tests using old signatures).

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/worktrees.rs
git commit -m "feat: thread scan_depth from config through scan and worktrees"
```

---

### Task 10: Regression + discovery unit tests (scanner)

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Add remaining spec cases**

```rust
#[test]
fn find_repos_finds_normal_git_dir_repo() {
    let tmp = tempfile::tempdir().unwrap();
    testutil::init_repo(&tmp.path().join("app"));
    let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
    assert_eq!(repos.len(), 1);
}

#[test]
fn find_repos_finds_bare_worktree_container_via_git_file() {
    let tmp = tempfile::tempdir().unwrap();
    let container = tmp.path().join("pigz-api");
    testutil::init_bare_worktree_container(&container);
    let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], container);
}

#[test]
fn find_repos_finds_pure_bare_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("foo.git");
    testutil::init_bare_repo(&bare);
    testutil::seed_bare_main(&bare);
    let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0], bare);
}
```

- [ ] **Step 2: Run `cargo test` — expect PASS**

- [ ] **Step 3: Commit**

```bash
git add src/scanner.rs
git commit -m "test(scanner): cover bare, worktree, and normal discovery"
```

---

### Task 11: E2E — `loops` lists branches in bare+worktree fixture

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Step 1: Add e2e test**

```rust
#[test]
fn list_finds_branches_in_bare_worktree_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    let container = root.join("pigz-api");
    std::fs::create_dir_all(&container).unwrap();

    // inline git setup (tests/cli.rs has its own git helper)
    let bare = container.join(".bare");
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
    git(&main, &["checkout", "-b", "feat/login"]);
    std::fs::write(main.join("b.txt"), "b").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "feat"]);
    git(&main, &["checkout", "main"]);

    loops(&home)
        .arg("init")
        .arg(&root)
        .assert()
        .success();

    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("pigz-api/feat/login"));
}
```

- [ ] **Step 2: Run test — expect PASS**

Run: `cargo test list_finds_branches_in_bare_worktree_layout -- --nocapture`

- [ ] **Step 3: Commit**

```bash
git add tests/cli.rs
git commit -m "test(cli): list loops in bare+worktree layout"
```

---

### Task 12: ADR 0005

**Files:**
- Create: `docs/decisions/0005-repo-discovery-via-git.md`

- [ ] **Step 1: Write ADR**

```markdown
# ADR 0005: Repository discovery via git interrogation

Date: 2026-06-25 · Status: accepted

## Context

`find_repos` originally treated only `dir/.git` directories as repositories.
Bare stores and worktrees use `.git` files or no `.git` at all, so discovery
returned zero repos in bare+worktree layouts.

## Decision

1. Mark FS candidates when `.git` exists (file or directory) or a cheap bare
   probe matches (`HEAD` + `objects/` + `refs/`).
2. Resolve each candidate with `git rev-parse --path-format=absolute --git-common-dir`.
3. Deduplicate by that absolute common-dir. N worktrees → one logical repo.
4. Derive `repo_name` from the common-dir basename (`.git`/`.bare` → parent name;
   `foo.git` → `foo`).
5. Replace fixed walk depth with configurable `scan_depth` (default 4).

## Rationale

Layout-specific path heuristics (`.bare`, worktree folder names) would encode
one author's tree. Git already exposes the canonical store identity. Shell-out
matches ADR 0002. The 3-segment key (`root_label/repo/branch` per ADR 0003)
is unchanged — only the source of `repo_name` changes.

## Consequences

- `find_repos` returns `(repos, warnings)`; failed candidates warn, never abort.
- Future inventory cache (ADR 0003 phase 3) should hash the absolute common-dir.
- Isolated `.bare` dirs hidden under a dot-prefixed parent are not discovered
  unless a root points at them or the container `.git` pointer exists (documented).
- Spec Fase B builds on this for per-worktree session attribution.
```

- [ ] **Step 2: Commit**

```bash
git add docs/decisions/0005-repo-discovery-via-git.md
git commit -m "docs: add ADR 0005 for git-based repo discovery"
```

---

### Task 13: User docs + CHANGELOG

**Files:**
- Modify: `docs/features.md`, `docs/configuration.md`, `CHANGELOG.md`, `ROADMAP.md`

- [ ] **Step 1: Update `docs/configuration.md`**

Replace the `roots` row description and add `scan_depth`:

```markdown
| `roots` | list of paths | `[]` | Directories searched for git repositories (see `scan_depth`) |
| `scan_depth` | integer | `4` | Max directory depth from each root to search for repositories |
```

Add section **Repository discovery**:

```markdown
## Repository discovery

`loops` discovers repositories by asking git, not by folder naming. Supported
layouts include normal checkouts (`.git` directory), worktrees (`.git` file), and
bare stores. Multiple worktrees of the same repo are scanned once (deduplicated
by git common-dir).

**Edge case:** a bare directory named `.bare` hidden inside a dot-prefixed parent
is skipped during descent and will not be found unless you register a root that
points directly at the repository container or bare path.
```

- [ ] **Step 2: Update `docs/features.md`**

Under inventory, add:

```markdown
Discovery is layout-agnostic: normal repos, worktrees, and bare stores under your
configured roots are found automatically. Repo names come from git's common-dir,
not from worktree folder names.
```

- [ ] **Step 3: Update CHANGELOG**

Run: `just changelog`
Review the generated entry mentions bare+worktree discovery and `scan_depth`.

- [ ] **Step 4: Check off `ROADMAP.md` Fase A items**

- [ ] **Step 5: Commit**

```bash
git add docs/features.md docs/configuration.md CHANGELOG.md ROADMAP.md
git commit -m "docs: document layout-agnostic repo discovery and scan_depth"
```

---

### Task 14: Manual validation + quality gates

- [ ] **Step 1: Manual smoke test (author environment)**

If `~/repo/pigz` exists on the machine:

```bash
loops init ~/repo/pigz   # or ensure root already configured
loops                  # expect pigz-api/* branches
```

If not available, the e2e fixture from Task 11 is the acceptance substitute.

- [ ] **Step 2: Run quality gates**

```bash
just fmt
just lint
just test
just cov
```

Expected: all pass; coverage at or above gate.

- [ ] **Step 3: Final commit if fmt touched files**

```bash
git add -A
git commit -m "chore: fmt after Fase A implementation"  # only if needed
```

---

## Self-Review (spec coverage)

| Spec § | Requirement | Task |
|---|---|---|
| §3 | `.git` file/dir + bare probe; early-return; SKIP_DIRS | Task 3 |
| §4 | `git_common_dir` + dedup | Task 2, 4 |
| §5 | `repo_name_from_common_dir` | Task 1, 5 |
| §6 | `scan_depth` default 4 | Task 7, 9 |
| §7 | scanner/config/cli changes | Tasks 3–9 |
| §7 | ADR 0005 | Task 12 |
| §8 | Edge cases documented | Task 13 |
| §10 | testutil helpers + test matrix | Tasks 8, 10, 11 |
| §11 | DoD checklist | Task 14 |
| §2 | Fase B out of scope | Not in plan |
| §9 | `loops worktrees` benefits via `find_repos` | Task 9 (no porcelain work) |

**Placeholder scan:** no TBD/TODO steps; each task has concrete code and commands.

**Type consistency:** `find_repos(roots, scan_depth) -> (Vec<PathBuf>, Vec<String>)` used consistently in `scan`, `scan_worktrees`, and tests. `repo_name_from_common_dir(&Path) -> String` used in `open_loops`. `Config::scan_depth: usize` default 4 threaded through CLI.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-25-scanner-bare-worktree-discovery.md`.

**Two execution options:**

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration. REQUIRED SUB-SKILL: `superpowers:subagent-driven-development`.

2. **Inline Execution** — execute tasks in one session using `superpowers:executing-plans`, batch execution with checkpoints.

**Which approach?**
