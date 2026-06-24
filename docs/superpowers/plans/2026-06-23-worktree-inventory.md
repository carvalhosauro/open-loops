# Worktree Inventory + EN-first CLI + Completions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `loops worktrees` command that inventories git worktrees with a cleanup verdict and a copy-paste cleanup command, add `loops completions <shell>`, and migrate all user-facing CLI output to English.

**Architecture:** New `src/worktrees.rs` enumerates worktrees per repo via `git worktree list --porcelain`, reusing `scanner` helpers (`git`, `default_branch`, `find_repos`) and the `--merged` set. A pure `verdict()` function classifies each worktree from signals (merged/dirty/prunable/main). `output.rs` renders an ASCII table plus a cleanup-command block. CLI gains thin `worktrees`/`wt` and `completions` subcommands. A final pass translates every user-facing string to English.

**Tech Stack:** Rust 2021, `clap` v4 (derive), `clap_complete` v4, `anyhow`, `chrono`. Tests use real git repos in tempdirs (`testutil`) and `assert_cmd` for the binary.

## Global Constraints

- All user-facing output and error messages in **English** (replaces the prior "erros em PT" rule).
- The `worktrees` command output (table + verdict labels + command block) is **strictly ASCII** — no emoji, no box-drawing, no non-ASCII glyphs (guarded by `worktrees_output_is_ascii`). Elsewhere, prefer ASCII punctuation but existing `—` in prose error messages may stay.
- Tolerant parsing: a bad git line → skip + warning, never abort (mirror `scanner::scan`).
- Conventional Commits; commit message subjects in English.
- All `#[test]` function names in **English** (new and existing).
- All code comments in **English** and **minimal** — write a comment only when it explains *why*, not *what*; delete redundant/obvious ones. Migrate and trim existing comments as part of Task 7.
- Coverage gate 70% (core target 85%) must still pass (`just cov`).
- `just lint` (clippy `-D warnings`) and `just fmt` must be clean before final commit.

---

### Task 1: `completions` command + `clap_complete` dependency

**Files:**
- Modify: `Cargo.toml:18-25` (add dependency)
- Modify: `src/cli.rs` (add `Completions` variant + `run_completions`)
- Modify: `src/main.rs:8-13` (dispatch arm)
- Test: `tests/cli.rs` (new test)

**Interfaces:**
- Produces: `Command::Completions { shell: clap_complete::Shell }`, `cli::run_completions(shell: clap_complete::Shell) -> anyhow::Result<()>`

- [ ] **Step 1: Add the failing integration test**

In `tests/cli.rs`, append:

```rust
#[test]
fn completions_generates_script_for_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    loops(&home)
        .arg("completions")
        .arg("bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("loops"));
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --test cli completions_generates_script_for_shell`
Expected: FAIL — `completions` is not a recognized subcommand (clap exits non-zero).

- [ ] **Step 3: Add the dependency**

In `Cargo.toml`, under `[dependencies]`, add after the `clap` line:

```toml
clap_complete = "4"
```

- [ ] **Step 4: Add the command variant and handler in `src/cli.rs`**

Add to the `Command` enum (after `Ignore`):

```rust
    /// Generate a shell completion script (bash, zsh, fish, ...)
    Completions { shell: clap_complete::Shell },
```

Add the handler (top-level fn in `cli.rs`):

```rust
pub fn run_completions(shell: clap_complete::Shell) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "loops", &mut std::io::stdout());
    Ok(())
}
```

- [ ] **Step 5: Dispatch in `src/main.rs`**

Add an arm to the `match cli.command` block (after the `Ignore` arm):

```rust
        Some(Command::Completions { shell }) => cli::run_completions(shell),
```

- [ ] **Step 6: Run the test to confirm it passes**

Run: `cargo test --test cli completions_generates_script_for_shell`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat(cli): add completions command for shell autocomplete"
```

---

### Task 2: `testutil` worktree helper

**Files:**
- Modify: `src/testutil.rs` (append helper)

**Interfaces:**
- Produces: `testutil::add_worktree(repo: &Path, path: &Path, branch: &str)` — creates a worktree at `path` on a NEW branch `branch` off the current HEAD (so the branch is merged into the base by default).

- [ ] **Step 1: Add the helper**

Append to `src/testutil.rs`:

```rust
/// Creates a worktree at `path` on a new branch off the current HEAD (counts as merged).
pub fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    git(repo, &["worktree", "add", path.to_str().unwrap(), "-b", branch]);
}
```

- [ ] **Step 2: Confirm the crate still builds**

Run: `cargo test --no-run`
Expected: compiles (helper is `#[cfg(test)]` via the module).

- [ ] **Step 3: Commit**

```bash
git add src/testutil.rs
git commit -m "test: add worktree helper to testutil"
```

---

### Task 3: `worktrees.rs` — `Worktree` struct, `Verdict`, and `verdict()`

**Files:**
- Create: `src/worktrees.rs`
- Modify: `src/lib.rs:10` (register module)

**Interfaces:**
- Produces:
  - `enum Verdict { Home, Prunable, Active, Deletable, Cold }` with `fn label(&self) -> &'static str`
  - `struct Worktree { repo_name: String, repo_path: PathBuf, worktree_path: PathBuf, branch: Option<String>, last_commit: Option<DateTime<Utc>>, merged: bool, dirty: bool, prunable: bool, is_main: bool }`
  - `Worktree::verdict(&self) -> Verdict`
  - `Worktree::short_name(&self) -> String` → `"{repo_name}/{worktree basename}"`

- [ ] **Step 1: Create the module with struct, enum, and the pure verdict + failing unit tests**

Create `src/worktrees.rs`:

```rust
//! Worktree inventory: joins `git worktree list` with merged/idle/state signals.
use crate::scanner::{default_branch, find_repos, git};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Cleanup classification of a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Main worktree (default checkout). Never deletable.
    Home,
    /// Directory gone / orphaned — cleared by `git worktree prune`.
    Prunable,
    /// Uncommitted changes or no clear branch. Live work.
    Active,
    /// Merged into default and clean — disk clutter.
    Deletable,
    /// Not merged and clean — review candidate.
    Cold,
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Home => "home",
            Verdict::Prunable => "prunable",
            Verdict::Active => "active",
            Verdict::Deletable => "deletable",
            Verdict::Cold => "cold",
        }
    }
}

/// A repository worktree.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub worktree_path: PathBuf,
    pub branch: Option<String>,
    pub last_commit: Option<DateTime<Utc>>,
    pub merged: bool,
    pub dirty: bool,
    pub prunable: bool,
    pub is_main: bool,
}

impl Worktree {
    /// Deterministic verdict; first matching rule wins.
    pub fn verdict(&self) -> Verdict {
        if self.is_main {
            return Verdict::Home;
        }
        if self.prunable {
            return Verdict::Prunable;
        }
        if self.dirty {
            return Verdict::Active;
        }
        match self.branch {
            None => Verdict::Active, // detached but clean — safe default
            Some(_) if self.merged => Verdict::Deletable,
            Some(_) => Verdict::Cold,
        }
    }

    /// Short table name: `repo/<worktree-basename>`.
    pub fn short_name(&self) -> String {
        let base = self
            .worktree_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.worktree_path.display().to_string());
        format!("{}/{}", self.repo_name, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wt(branch: Option<&str>, merged: bool, dirty: bool, prunable: bool, is_main: bool) -> Worktree {
        Worktree {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            worktree_path: PathBuf::from("/tmp/app/.wt/x"),
            branch: branch.map(|b| b.into()),
            last_commit: None,
            merged,
            dirty,
            prunable,
            is_main,
        }
    }

    #[test]
    fn verdict_covers_all_combinations() {
        assert_eq!(wt(Some("main"), true, false, false, true).verdict(), Verdict::Home);
        assert_eq!(wt(Some("x"), false, false, true, false).verdict(), Verdict::Prunable);
        assert_eq!(wt(Some("x"), false, true, false, false).verdict(), Verdict::Active);
        assert_eq!(wt(Some("x"), true, false, false, false).verdict(), Verdict::Deletable);
        assert_eq!(wt(Some("x"), false, false, false, false).verdict(), Verdict::Cold);
        // detached clean -> active
        assert_eq!(wt(None, false, false, false, false).verdict(), Verdict::Active);
        // is_main beats prunable/dirty
        assert_eq!(wt(Some("main"), false, true, true, true).verdict(), Verdict::Home);
    }

    #[test]
    fn short_name_uses_basename() {
        let w = wt(Some("x"), false, false, false, false);
        assert_eq!(w.short_name(), "app/x");
    }
}
```

- [ ] **Step 2: Register the module in `src/lib.rs`**

Add after `pub mod sessions;` (line 11), keeping alphabetical-ish order is not required — place it after `sessions`:

```rust
pub mod worktrees;
```

- [ ] **Step 3: Run the unit tests to confirm they pass**

Run: `cargo test --lib worktrees::tests`
Expected: PASS (2 tests)

- [ ] **Step 4: Commit**

```bash
git add src/worktrees.rs src/lib.rs
git commit -m "feat(worktrees): add Worktree model and deterministic verdict"
```

---

### Task 4: `worktrees.rs` — enumeration (`worktrees` + `scan_worktrees`)

**Files:**
- Modify: `src/worktrees.rs` (add functions + tests)

**Interfaces:**
- Consumes: `scanner::git`, `scanner::default_branch`, `scanner::find_repos` (all already `pub`/`pub(crate)` in-crate); `testutil::add_worktree` (Task 2).
- Produces:
  - `fn worktrees(repo: &Path) -> Result<Vec<Worktree>>`
  - `fn scan_worktrees(roots: &[PathBuf]) -> (Vec<Worktree>, Vec<String>)`

- [ ] **Step 1: Add the failing enumeration tests**

In `src/worktrees.rs`, inside `mod tests`, add (keep the existing `use super::*;`; add the testutil import):

```rust
    use crate::testutil;

    #[test]
    fn worktrees_classifies_deletable_cold_and_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);

        // deletable: new branch off main (merged), clean worktree
        let del = tmp.path().join("wt-del");
        testutil::add_worktree(&repo, &del, "feat/done");

        // cold: branch with its own commit (unmerged), clean worktree
        let cold = tmp.path().join("wt-cold");
        testutil::add_worktree(&repo, &cold, "feat/cold");
        std::fs::write(cold.join("c.txt"), "c").unwrap();
        testutil::git(&cold, &["add", "."]);
        testutil::git(&cold, &["commit", "-m", "wip cold"]);

        // active (dirty): new branch off main with an uncommitted file
        let dirty = tmp.path().join("wt-dirty");
        testutil::add_worktree(&repo, &dirty, "feat/dirty");
        std::fs::write(dirty.join("d.txt"), "d").unwrap();

        let all = worktrees(&repo).unwrap();
        let by_branch = |b: &str| {
            all.iter()
                .find(|w| w.branch.as_deref() == Some(b))
                .unwrap_or_else(|| panic!("branch {b} missing"))
        };
        assert_eq!(by_branch("feat/done").verdict(), Verdict::Deletable);
        assert_eq!(by_branch("feat/cold").verdict(), Verdict::Cold);
        assert_eq!(by_branch("feat/dirty").verdict(), Verdict::Active);

        // main becomes home
        let main = all.iter().find(|w| w.is_main).expect("main worktree");
        assert_eq!(main.verdict(), Verdict::Home);
    }

    #[test]
    fn scan_worktrees_aggregates_and_does_not_abort() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        let extra = tmp.path().join("wt-extra");
        testutil::add_worktree(&repo, &extra, "feat/extra");

        let (all, warnings) = scan_worktrees(&[tmp.path().to_path_buf()]);
        assert!(all.iter().any(|w| w.branch.as_deref() == Some("feat/extra")));
        assert!(warnings.is_empty());
    }
```

Also add `tempfile` to dev-deps if missing — it is already present (`Cargo.toml:30`).

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib worktrees::tests::worktrees_classifies`
Expected: FAIL — `worktrees` function not found.

- [ ] **Step 3: Implement `worktrees()` and `scan_worktrees()`**

Add to `src/worktrees.rs` (after the `impl Worktree` block, before `#[cfg(test)]`):

```rust
/// Enumerates and classifies a repository's worktrees.
///
/// # Errors
///
/// Returns `Err` if `git worktree list` fails.
pub fn worktrees(repo: &Path) -> Result<Vec<Worktree>> {
    let raw = git(repo, &["worktree", "list", "--porcelain"])?;
    let default = default_branch(repo).ok();
    let merged_set: HashSet<String> = match &default {
        Some(d) => git(repo, &["branch", "--merged", d, "--format=%(refname:short)"])
            .unwrap_or_default()
            .lines()
            .map(|s| s.trim().to_string())
            .collect(),
        None => HashSet::new(),
    };
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string());

    let mut out = Vec::new();
    let mut first = true;
    for block in raw.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let mut wt_path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        let mut prunable = false;
        let mut bare = false;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                wt_path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line == "bare" {
                bare = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                prunable = true;
            }
            // "detached" => branch stays None
        }
        let Some(wt_path) = wt_path else { continue };
        if bare {
            continue;
        }
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
}

/// Scans worktrees of all repos found under the roots, in parallel.
///
/// Per-repo failures become warnings, never abort.
pub fn scan_worktrees(roots: &[PathBuf]) -> (Vec<Worktree>, Vec<String>) {
    let repos = find_repos(roots);
    let results: Vec<Result<Vec<Worktree>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos.iter().map(|r| s.spawn(move || worktrees(r))).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("panic while scanning worktrees")))
            })
            .collect()
    });
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut w) => all.append(&mut w),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
        }
    }
    (all, warnings)
}
```

- [ ] **Step 4: Run the enumeration tests**

Run: `cargo test --lib worktrees::tests`
Expected: PASS (all worktrees tests)

- [ ] **Step 5: Commit**

```bash
git add src/worktrees.rs
git commit -m "feat(worktrees): enumerate and classify repo worktrees"
```

---

### Task 5: `output.rs` — `render_worktrees`

**Files:**
- Modify: `src/output.rs` (add render fn + helpers + tests)

**Interfaces:**
- Consumes: `worktrees::{Worktree, Verdict}`, `output::human_age` (existing).
- Produces: `output::render_worktrees(wts: &[Worktree], now: DateTime<Utc>) -> String`

- [ ] **Step 1: Add the failing render tests**

In `src/output.rs`, inside `mod tests`, add:

```rust
    use crate::worktrees::Worktree;

    fn wt(branch: &str, merged: bool, dirty: bool, idade_dias: i64) -> Worktree {
        Worktree {
            repo_name: "app".into(),
            repo_path: std::path::PathBuf::from("/tmp/app"),
            worktree_path: std::path::PathBuf::from(format!("/tmp/app/{branch}")),
            branch: Some(branch.into()),
            last_commit: Some(Utc::now() - Duration::days(idade_dias)),
            merged,
            dirty,
            prunable: false,
            is_main: false,
        }
    }

    #[test]
    fn render_worktrees_sorts_deletable_first_and_shows_command() {
        let out = render_worktrees(
            &[
                wt("feat/cold", false, false, 40),
                wt("fix/done", true, false, 8),
            ],
            Utc::now(),
        );
        // header ASCII
        assert!(out.contains("WORKTREE"));
        assert!(out.contains("VERDICT"));
        // deletable aparece antes de cold
        let pos_done = out.find("fix/done").unwrap();
        let pos_cold = out.find("feat/cold").unwrap();
        assert!(pos_done < pos_cold);
        // bloco de comando para a deletable
        assert!(out.contains("worktree remove"));
        assert!(out.contains("branch -d fix/done"));
        // ASCII-only
        assert!(out.is_ascii());
    }

    #[test]
    fn render_worktrees_no_action_says_nothing() {
        let out = render_worktrees(&[wt("feat/cold", false, false, 3)], Utc::now());
        assert!(out.contains("nothing to clean up"));
    }

    #[test]
    fn render_worktrees_empty() {
        assert!(render_worktrees(&[], Utc::now()).contains("No worktrees found"));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib output::tests::render_worktrees`
Expected: FAIL — `render_worktrees` not found.

- [ ] **Step 3: Implement `render_worktrees` and helpers**

Add to `src/output.rs` (after `render_table`, before `#[cfg(test)]`). Add `use std::collections::HashSet;` and `use crate::worktrees::{Verdict, Worktree};` to the top imports.

```rust
fn verdict_rank(v: &Verdict) -> u8 {
    match v {
        Verdict::Deletable | Verdict::Prunable => 0,
        Verdict::Cold => 1,
        Verdict::Active => 2,
        Verdict::Home => 3,
    }
}

fn branch_label(w: &Worktree) -> String {
    w.branch.clone().unwrap_or_else(|| "(detached)".into())
}

/// Renders the worktree table + ASCII cleanup-command block.
///
/// Order: deletable/prunable first, then oldest idle first.
pub fn render_worktrees(wts: &[Worktree], now: DateTime<Utc>) -> String {
    if wts.is_empty() {
        return "No worktrees found.\n".into();
    }
    let epoch = DateTime::from_timestamp(0, 0).unwrap();
    let mut sorted: Vec<&Worktree> = wts.iter().collect();
    sorted.sort_by_key(|w| (verdict_rank(&w.verdict()), w.last_commit.unwrap_or(epoch)));

    let name_w = sorted.iter().map(|w| w.short_name().len()).max().unwrap_or(8).max(8);
    let branch_w = sorted.iter().map(|w| branch_label(w).len()).max().unwrap_or(6).max(6);

    let mut out = format!(
        "{:<name_w$}  {:<branch_w$}  {:>5}  {:>6}  {:>5}  {}\n",
        "WORKTREE", "BRANCH", "IDLE", "MERGED", "STATE", "VERDICT"
    );
    for w in &sorted {
        out.push_str(&format!(
            "{:<name_w$}  {:<branch_w$}  {:>5}  {:>6}  {:>5}  {}\n",
            w.short_name(),
            branch_label(w),
            w.last_commit.map(|t| human_age(now, t)).unwrap_or_else(|| "?".into()),
            if w.merged { "yes" } else { "no" },
            if w.dirty { "dirty" } else { "clean" },
            w.verdict().label()
        ));
    }

    let mut cmds: Vec<String> = Vec::new();
    let mut pruned: HashSet<PathBuf> = HashSet::new();
    for w in &sorted {
        match w.verdict() {
            Verdict::Deletable => {
                if let Some(b) = &w.branch {
                    cmds.push(format!(
                        "git -C {repo} worktree remove {wt} && git -C {repo} branch -d {b}",
                        repo = w.repo_path.display(),
                        wt = w.worktree_path.display(),
                    ));
                }
            }
            Verdict::Prunable => {
                if pruned.insert(w.repo_path.clone()) {
                    cmds.push(format!("git -C {} worktree prune", w.repo_path.display()));
                }
            }
            _ => {}
        }
    }
    if cmds.is_empty() {
        out.push_str("\n# nothing to clean up.\n");
    } else {
        out.push_str(&format!("\n# {} worktree(s) to clean up. Copy to run:\n", cmds.len()));
        for c in &cmds {
            out.push_str(c);
            out.push('\n');
        }
    }
    out
}
```

Add `use std::path::PathBuf;` to the top of `output.rs` if not already imported.

- [ ] **Step 4: Run the render tests**

Run: `cargo test --lib output::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/output.rs
git commit -m "feat(output): render worktree inventory table with cleanup commands"
```

---

### Task 6: CLI wiring — `worktrees` / `wt` command

**Files:**
- Modify: `src/cli.rs` (variant + `run_worktrees`, import `worktrees`)
- Modify: `src/main.rs` (dispatch arm)
- Test: `tests/cli.rs` (new e2e test)

**Interfaces:**
- Consumes: `worktrees::scan_worktrees`, `output::render_worktrees`.
- Produces: `Command::Worktrees` (alias `wt`), `cli::run_worktrees(base: &Path) -> Result<()>`

- [ ] **Step 1: Add the failing e2e test**

In `tests/cli.rs`, append:

```rust
#[test]
fn worktrees_lists_and_suggests_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("meu-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    // worktree mergeada (branch nova off main) e limpa => deletable
    let wt = tmp.path().join("wt-done");
    git(&repo, &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"]);

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("deletable"))
        .stdout(predicate::str::contains("worktree remove"));

    // alias wt funciona
    loops(&home).arg("wt").assert().success();
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --test cli worktrees_lists_and_suggests_cleanup`
Expected: FAIL — `worktrees` not a recognized subcommand.

- [ ] **Step 3: Add the command variant in `src/cli.rs`**

Add the `worktrees` import to the top `use crate::{...}` line (it currently reads `use crate::{cache, distill, output, sessions};` → add `worktrees`):

```rust
use crate::{cache, distill, output, sessions, worktrees};
```

Add to the `Command` enum (after `Ignore`, before `Completions`):

```rust
    /// List git worktrees with a cleanup verdict (alias: wt)
    #[command(visible_alias = "wt")]
    Worktrees,
```

- [ ] **Step 4: Add `run_worktrees` in `src/cli.rs`**

```rust
pub fn run_worktrees(base: &Path) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let (wts, warnings) = worktrees::scan_worktrees(&cfg.roots);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    print!("{}", output::render_worktrees(&wts, chrono::Utc::now()));
    Ok(())
}
```

- [ ] **Step 5: Dispatch in `src/main.rs`**

Add an arm (before the `Completions` arm):

```rust
        Some(Command::Worktrees) => cli::run_worktrees(&base),
```

- [ ] **Step 6: Run the test to confirm it passes**

Run: `cargo test --test cli worktrees_lists_and_suggests_cleanup`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat(cli): add worktrees command (alias wt)"
```

---

### Task 7: English migration — status & error strings

Translate all user-facing string literals (errors, warnings, status lines, table headers, command `about`/doc-comments) to English across modules, and fix the existing tests that assert Portuguese text. Code comments and `#[test]` names stay Portuguese.

**Files:**
- Modify: `src/cli.rs`, `src/output.rs`, `src/main.rs`, `src/scanner.rs`, `src/config.rs`, `src/distill.rs` (error strings only — the prompt is Task 8), `src/cache.rs`, `src/ignores.rs`
- Modify: `src/scanner.rs` tests and `tests/cli.rs` (assertions that match PT text)

**Interfaces:** No signature changes — string content only.

- [ ] **Step 1: Translate `src/cli.rs` strings**

Apply these exact replacements:

| Portuguese | English |
|---|---|
| `about = "Recupere o contexto de trabalhos pausados"` | `about = "Recover the context of paused work"` |
| `/// Registra raízes de repositórios (ex.: loops init ~/repo)` | `/// Register repository roots (e.g. loops init ~/repo)` |
| `/// Destila o contexto de um loop: por quê, feito, falta, próximo passo` | `/// Distill a loop's context: why, done, remaining, next step` |
| `/// Descarta um loop morto da lista (formato repo/branch)` | `/// Drop a dead loop from the list (repo/branch format)` |
| `"nenhuma raiz configurada. Rode: loops init <dir-com-seus-repos>"` (both occurrences, run_list + run_resume) | `"no roots configured. Run: loops init <dir-with-your-repos>"` |
| `eprintln!("aviso: {w}");` (both occurrences) | `eprintln!("warning: {w}");` |
| `"uso: loops init <dir> [<dir>...]"` | `"usage: loops init <dir> [<dir>...]"` |
| `println!("raízes registradas:");` | `println!("roots registered:");` |
| `println!("\nconfig em {}", store.config_path().display());` | `println!("\nconfig at {}", store.config_path().display());` |
| `"formato esperado: repo/branch (rode `loops` para ver as chaves)"` | `"expected format: repo/branch (run `loops` to see the keys)"` |
| `println!("ignorado: {key}");` | `println!("ignored: {key}");` |
| `"nenhum loop bate com '{query}'. Rode `loops` para ver os abertos."` | `"no loop matches '{query}'. Run `loops` to see open ones."` |
| `"query ambígua, candidatos:\n{}"` | `"ambiguous query, candidates:\n{}"` |
| `"aviso: nenhuma sessão de IA encontrada — confiança baixa, contexto só do git"` | `"warning: no AI session found — low confidence, context from git only"` |

- [ ] **Step 2: Translate `src/output.rs` strings**

| Portuguese | English |
|---|---|
| `"Nenhum loop aberto. Tudo finalizado ou ignorado.\n"` | `"No open loops. All finished or ignored.\n"` |
| header `"LOOP", "PARADO HÁ", "AHEAD", "BEHIND"` | `"LOOP", "IDLE", "AHEAD", "BEHIND"` |

Update the existing test `render_table_vazia_celebra` assertion `contains("Nenhum loop aberto")` → `contains("No open loops")`. Update `render_table_ordena_mais_parado_primeiro` if it asserts the old header (it asserts `contains("LOOP")`, still valid).

- [ ] **Step 3: Translate `src/main.rs` strings**

| Portuguese | English |
|---|---|
| `eprintln!("erro: {e:#}");` | `eprintln!("error: {e:#}");` |
| `.expect("HOME não definido")` | `.expect("HOME not set")` |

- [ ] **Step 4: Translate `src/scanner.rs` strings (and tests)**

| Portuguese | English |
|---|---|
| `"git não encontrado no PATH — instale o git"` | `"git not found in PATH — install git"` |
| `"git {:?} falhou em {}: {}"` | `"git {:?} failed in {}: {}"` |
| `"não achei a branch default em {} (esperava origin/HEAD, main ou master)"` | `"couldn't find the default branch in {} (expected origin/HEAD, main or master)"` |
| `"aviso: linha inesperada do git for-each-ref ignorada: {line:?}"` | `"warning: unexpected line from git for-each-ref ignored: {line:?}"` |
| `"data inválida vinda do git: {date}"` | `"invalid date from git: {date}"` |
| `"sem datas de commit para {branch}"` (both occurrences) | `"no commit dates for {branch}"` |
| `"panic ao escanear o repositório"` | `"panic while scanning repository"` |

Update scanner tests: `default_branch_erro_sem_main_nem_master` asserts `contains("não achei a branch default")` → `contains("couldn't find the default branch")`.

- [ ] **Step 5: Translate `src/config.rs` strings**

| Portuguese | English |
|---|---|
| `format!("lendo {}", path.display())` | `format!("reading {}", path.display())` |
| `format!("config.toml inválido em {}", path.display())` | `format!("invalid config.toml at {}", path.display())` |
| `format!("criando {}", self.base.display())` | `format!("creating {}", self.base.display())` |
| `format!("raiz inexistente: {}", p.display())` | `format!("nonexistent root: {}", p.display())` |

- [ ] **Step 6: Translate `src/distill.rs` error strings (NOT the prompt — Task 8)**

| Portuguese | English |
|---|---|
| `"falha ao executar o comando LLM `{llm_command}` — está instalado? Ajuste llm_command no config.toml"` | `"failed to run the LLM command `{llm_command}` — is it installed? Adjust llm_command in config.toml"` |
| `"stdin não disponível para o processo LLM"` | `"stdin not available for the LLM process"` |
| `"falha ao escrever o prompt no stdin do LLM"` | `"failed to write the prompt to the LLM stdin"` |
| `"falha ao aguardar o processo LLM"` | `"failed to wait for the LLM process"` |
| `"comando LLM falhou (`{llm_command}`): {}"` | `"LLM command failed (`{llm_command}`): {}"` |

- [ ] **Step 7: Translate `src/cache.rs` and `src/ignores.rs` strings**

`src/cache.rs`:

| Portuguese | English |
|---|---|
| `"caminho do cache não tem diretório pai"` | `"cache path has no parent directory"` |

`src/ignores.rs`:

| Portuguese | English |
|---|---|
| `format!("ignores.toml inválido em {}", path.display())` | `format!("invalid ignores.toml at {}", path.display())` |
| `format!("lendo {}", path.display())` | `format!("reading {}", path.display())` |
| `format!("caminho sem diretório pai: {}", self.path.display())` | `format!("path has no parent directory: {}", self.path.display())` |

- [ ] **Step 8: Fix `tests/cli.rs` Portuguese assertions**

In `fluxo_completo_init_list_resume_cache_ignore`: change `predicate::str::contains("raízes registradas")` → `predicate::str::contains("roots registered")`. Leave repo/branch keys (`meu-app/feat/login`) unchanged — those are data, not translated. Scan the rest of `tests/cli.rs` for any other PT assertion strings (e.g. `ignorado`, `nenhum loop`) and translate them to match the new output (`ignored`, `no loop matches`).

- [ ] **Step 9: Rename existing Portuguese test function names to English**

Find every test with a Portuguese name:

```bash
grep -rnE '#\[test\]' -A1 src/ tests/ | grep -E 'fn .*(_[a-z]*ã|ç|õ|aviso|erro|vazi|mergeada|acha|nao|naum|sem_|cobre|ordena|celebra|completo|fluxo)'
```

Rename each `fn <pt_name>()` to an English equivalent (keep the asserted behavior). Known renames in already-read files:

| Portuguese | English |
|---|---|
| `default_branch_detecta_main` | `default_branch_detects_main` |
| `git_falha_com_mensagem_contextual` | `git_fails_with_contextual_message` |
| `find_repos_acha_repos_ate_profundidade_3_e_pula_ocultos` | `find_repos_finds_repos_up_to_depth_3_and_skips_hidden` |
| `open_loops_acha_nao_mergeada_ignora_mergeada_e_default` | `open_loops_finds_unmerged_ignores_merged_and_default` |
| `scan_agrega_repos_e_reporta_warning_sem_abortar` | `scan_aggregates_repos_and_reports_warning_without_aborting` |
| `helpers_de_contexto_retornam_commits_e_janela` | `context_helpers_return_commits_and_window` |
| `default_branch_detecta_master_fallback` | `default_branch_detects_master_fallback` |
| `default_branch_erro_sem_main_nem_master` | `default_branch_errors_without_main_or_master` |
| `human_age_minutos_horas_dias` | `human_age_minutes_hours_days` |
| `render_table_ordena_mais_parado_primeiro` | `render_table_sorts_most_idle_first` |
| `render_table_vazia_celebra` | `render_table_empty_celebrates` |
| `fluxo_completo_init_list_resume_cache_ignore` | `full_flow_init_list_resume_cache_ignore` |

For test names in `config.rs`, `distill.rs`, `cache.rs`, `ignores.rs`, `sessions/*` not listed above (the grep surfaces them), translate each to a clear English name. Comments inside tests stay Portuguese.

- [ ] **Step 10: Translate and trim code comments to English**

Find the Portuguese comments still in `src/`:

```bash
grep -rnE '//' src/ | grep -E 'ã|ç|õ|é|í|ú|â|ê|à'
```

For every hit (module docs `//!`, doc-comments `///`, inline `//`) in `scanner.rs`, `cli.rs`, `output.rs`, `main.rs`, `config.rs`, `distill.rs`, `cache.rs`, `ignores.rs`, `sessions/mod.rs`, `sessions/claude_code.rs`, `testutil.rs`:
- Rewrite it in English.
- Apply the minimal-comment rule: delete comments that merely restate the code (e.g. `// checkout na branch`), keep only those that explain *why* (e.g. the shell-out design rationale in `scanner.rs:1-3`, the "broken pipe" handling in `distill.rs`).
- `# Errors` doc sections: translate the body (`Retorna `Err` se...` → `Returns `Err` if...`).

Do NOT touch `tests/cli.rs` data strings or doc text inside `docs/` here (handled elsewhere).

- [ ] **Step 11: Run the full suite and lint**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "refactor: migrate CLI output, errors, comments, and test names to English"
```

---

### Task 8: English migration — resume distillation output

The `resume` output sections come from the LLM prompt, which currently requests Portuguese. Translate the prompt, the section names, and the `## Fontes` source block, then fix the distill/cli tests that assert Portuguese.

**Files:**
- Modify: `src/distill.rs` (`build_prompt`, `with_sources`, and their tests)
- Modify: `tests/cli.rs` (assertions on prompt/section text, if any)

**Interfaces:** No signature changes.

- [ ] **Step 1: Translate the prompt in `build_prompt`**

Replace the `format!` prompt body (lines ~22-29) with:

```rust
        "You reconstruct the context of a paused work branch.\n\
         Answer in markdown, in English, with exactly these sections:\n\n\
         ## Why\n## Done\n## Remaining\n## Next step\n\n\
         Be concrete and direct. Rely ONLY on the evidence below.\n\
         If the evidence is insufficient for a section, write \"insufficient evidence\".\n\n\
         # Branch\n{key} (base: {default_branch})\n\n\
         # Commits (base..branch)\n{commits}\n\n\
         # Diffstat\n{diffstat}\n",
```

Replace the no-sessions line:

```rust
        p.push_str("\n# AI sessions\nnone found\n");
```

And the per-session header:

```rust
                "\n# Session {} (modified {})\n{}\n",
```

- [ ] **Step 2: Translate `with_sources` (line ~101)**

Replace `## Fontes` and the git source line:

```rust
        "# {}\n\n{}\n\n## Sources\n\n- git: branch {} (HEAD {})\n",
```

(Translate any other Portuguese label in `with_sources` — e.g. a sessions source line — to English: "session" / "AI session".)

- [ ] **Step 3: Update `distill.rs` tests**

| Old assertion | New assertion |
|---|---|
| `p.contains("## Por quê")` | `p.contains("## Why")` |
| `p.contains("## Próximo passo")` | `p.contains("## Next step")` |
| `with_sources("## Por quê\nlogin", ...)` | `with_sources("## Why\nlogin", ...)` |
| `doc.contains("## Fontes")` | `doc.contains("## Sources")` |
| `with_sources("## Por quê\nconteudo", ...)` | `with_sources("## Why\nconteudo", ...)` |

Scan the rest of `distill.rs` tests for other PT section strings and translate to the new names.

- [ ] **Step 4: Update `tests/cli.rs`**

If the resume e2e asserts any section text, translate it. (The current resume test checks the prompt contains commit text, which is data — verify and leave unchanged unless it asserts a PT section header.)

- [ ] **Step 5: Run distill + cli tests**

Run: `cargo test --lib distill && cargo test --test cli`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/distill.rs tests/cli.rs
git commit -m "refactor(distill): produce resume output in English"
```

---

### Task 9: Quality-focused flow tests

End-to-end tests through the real binary that guard *quality* properties, not just happy-path wiring: cross-repo aggregation, the safety guard (never suggest destroying live/unmerged work), the ASCII guarantee, multi-shell completions, and no-false-positive on a clean environment.

**Files:**
- Modify: `tests/cli.rs` (append tests)

**Interfaces:** uses the existing `git` and `loops` test helpers in `tests/cli.rs`.

- [ ] **Step 1: Add a helper to build a repo with a worktree**

In `tests/cli.rs`, add near the other helpers:

```rust
/// Builds a git repo at `repo` (main + 1 commit) and returns its path ready for worktrees.
fn init_repo(repo: &std::path::Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "init"]);
}
```

- [ ] **Step 2: Add the quality flow tests**

```rust
#[test]
fn worktrees_aggregates_across_multiple_repos() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    for (i, name) in ["app-a", "app-b"].iter().enumerate() {
        let repo = root.join(name);
        init_repo(&repo);
        let wt = tmp.path().join(format!("wt-{i}"));
        git(&repo, &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"]);
    }
    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("app-a/wt-0"))
        .stdout(predicate::str::contains("app-b/wt-1"))
        // one cleanup command per deletable worktree
        .stdout(predicate::str::contains("2 worktree(s) to clean up"));
}

#[test]
fn worktrees_never_suggests_removing_unmerged_or_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("app");
    init_repo(&repo);

    // cold: unmerged branch with its own commit, clean
    let cold = tmp.path().join("wt-cold");
    git(&repo, &["worktree", "add", cold.to_str().unwrap(), "-b", "feat/cold"]);
    std::fs::write(cold.join("c.txt"), "c").unwrap();
    git(&cold, &["add", "."]);
    git(&cold, &["commit", "-m", "wip"]);

    // dirty: branch off main with an uncommitted file
    let dirty = tmp.path().join("wt-dirty");
    git(&repo, &["worktree", "add", dirty.to_str().unwrap(), "-b", "feat/dirty"]);
    std::fs::write(dirty.join("d.txt"), "d").unwrap();

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("cold"))
        .stdout(predicate::str::contains("active"))
        // safety: no destructive command suggested for live/unmerged work
        .stdout(predicate::str::contains("nothing to clean up"))
        .stdout(predicate::str::contains("worktree remove").not());
}

#[test]
fn worktrees_output_is_ascii() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("app");
    init_repo(&repo);
    let wt = tmp.path().join("wt");
    git(&repo, &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"]);
    loops(&home).arg("init").arg(&root).assert().success();

    let out = loops(&home).arg("worktrees").assert().success().get_output().stdout.clone();
    assert!(out.is_ascii(), "worktrees output must be ASCII-only");
}

#[test]
fn worktrees_clean_environment_has_no_false_positive() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("app");
    init_repo(&repo); // only the main worktree
    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("home"))
        .stdout(predicate::str::contains("nothing to clean up"))
        .stdout(predicate::str::contains("worktree remove").not());
}

#[test]
fn completions_for_zsh_and_fish_are_nonempty() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    for shell in ["zsh", "fish"] {
        loops(&home)
            .arg("completions")
            .arg(shell)
            .assert()
            .success()
            .stdout(predicate::str::contains("loops"));
    }
}
```

Note: `predicate::str::contains(...).not()` requires `use predicates::prelude::*;` — already imported at the top of `tests/cli.rs`.

- [ ] **Step 3: Run the new tests**

Run: `cargo test --test cli`
Expected: PASS (all flow tests, including the existing one).

- [ ] **Step 4: Commit**

```bash
git add tests/cli.rs
git commit -m "test: add quality-focused worktree and completion flow tests"
```

---

### Task 10: Docs, CLAUDE.md rule, CHANGELOG, final verification

**Files:**
- Modify: `docs/features.md` (translate; add `worktrees` and `completions` sections)
- Modify: `docs/configuration.md`, `docs/setup.md` (translate to English)
- Modify: `CLAUDE.md` (update the error-language rule)
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Translate `docs/features.md` and add new command docs**

Translate the existing sections to English. Then add:

````markdown
## `loops worktrees` (alias `wt`) — worktree inventory

```bash
loops worktrees
# WORKTREE          BRANCH       IDLE  MERGED  STATE  VERDICT
# my-app/fix-bug    fix/bug       8d   yes     clean  deletable
# api/spike-redis   spike/redis   40d  no      clean  cold
```

Lists every git worktree across the configured roots with a cleanup verdict:

- `deletable` — merged into the default branch and clean; safe to remove.
- `cold` — not merged, clean; review candidate.
- `active` — has uncommitted changes; live work, left alone.
- `prunable` — directory gone / orphaned; `git worktree prune` clears it.
- `home` — the main worktree; never removed.

For `deletable`/`prunable` worktrees it prints the exact cleanup command to copy.
It never deletes anything itself.

## `loops completions <shell>` — shell autocomplete

```bash
loops completions zsh > ~/.zfunc/_loops   # zsh
loops completions bash > /etc/bash_completion.d/loops
loops completions fish > ~/.config/fish/completions/loops.fish
```

Prints a completion script for the given shell (`bash`, `zsh`, `fish`, ...).
````

- [ ] **Step 2: Translate `docs/configuration.md` and `docs/setup.md`**

Translate all prose and examples to English. Keep commands and config keys unchanged.

- [ ] **Step 3: Update the error-language rule in `CLAUDE.md`**

In the `## Convenções` section, replace:

```
- Conventional Commits (hook valida); mensagens de erro em PT, acionáveis
```

with:

```
- Conventional Commits (hook valida); mensagens de erro em EN, acionáveis
```

- [ ] **Step 4: Update `CHANGELOG.md`**

Run: `just changelog`

If git-cliff is unavailable or the entry is thin, add a manual `## [Unreleased]` section noting: `loops worktrees`/`wt` command, `loops completions`, and the English migration of all CLI output.

- [ ] **Step 5: Full verification**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all PASS, no warnings, no format diffs.

- [ ] **Step 6: Coverage gate**

Run: `just cov`
Expected: total coverage ≥ 70% (core ≥ 85%). If `worktrees.rs`/`output.rs` drag it below, add unit tests for the uncovered branches (e.g. `prunable` verdict path) before proceeding.

- [ ] **Step 7: Manual smoke test**

Run:
```bash
cargo run -- worktrees
cargo run -- completions zsh | head
```
Expected: a worktree table (or `No worktrees found.`) in English; a non-empty zsh completion script.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "docs: english docs, worktrees + completions, error-language rule"
```

---

## Self-Review Notes

- **Spec coverage:** worktrees command (Tasks 3-6), completions (Task 1), ASCII + verdict rule (Tasks 3,5), EN migration incl. errors + resume output + comments + test names + docs + CLAUDE.md (Tasks 7,8,10), quality flow tests — cross-repo aggregation, safety guard, ASCII, multi-shell completions, no false positives (Task 9), tests with real worktrees (Tasks 2,4,6). All spec sections mapped.
- **Out of scope honored:** no `loops clean` deletion command anywhere.
- **`--json`:** spec deferred it (global flag not yet implemented); not in this plan.
- **Type consistency:** `Worktree`/`Verdict` field and variant names are used identically in Tasks 3→4→5→6; `render_worktrees`/`scan_worktrees`/`run_worktrees` signatures match across producer and consumer tasks.
