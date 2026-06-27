//! Repository and unmerged-branch discovery via git shell-out.
//! Design decision: shell-out (not git2/gix) — simple and debuggable;
//! the product performance bottleneck is the LLM, not git.
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::inventory::{self, InventoryFile, InventoryStore, LoopMemo};

/// Inventory update produced by one `open_loops` call: `(common-dir hash, file)`.
type InvUpdate = (String, InventoryFile);

/// Options controlling a scan (light phase always; heavy phase optional + memoised).
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Whether to compute ahead/behind counts (heavy phase via `rev-list`).
    pub need_ahead_behind: bool,
    /// When true, skip any cached inventory memo and recompute `rev-list`.
    pub fresh: bool,
    /// Directory for the inventory JSON files. `None` disables memoisation.
    pub inventory_dir: Option<PathBuf>,
    /// Seconds before a cached entry expires; 0 = SHA-only validation.
    pub inventory_ttl_secs: u64,
}

/// Runs a git subcommand in `repo` and returns trimmed stdout.
///
/// # Errors
///
/// Returns `Err` if git is not in PATH or if the command fails.
pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("git not found in PATH — install git")?;
    if !out.status.success() {
        bail!(
            "git {:?} failed in {}: {}",
            args,
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Default branch: origin/HEAD's target if it resolves locally; otherwise main;
/// otherwise master.
///
/// # Errors
///
/// Returns `Err` if no default branch is found.
pub fn default_branch(repo: &Path) -> Result<String> {
    let (name, _) = default_branch_and_sha(repo)?;
    Ok(name)
}

/// Default branch name and its SHA, resolved in a single rev-parse call.
/// Used internally to avoid redundant git calls in the heavy phase.
///
/// origin/HEAD only wins when its target branch exists locally: a stale or
/// `--single-branch` origin/HEAD can name a branch with no local ref, and we
/// must fall through to main/master rather than hide the whole repo.
///
/// # Errors
///
/// Returns `Err` if no default branch is found.
fn default_branch_and_sha(repo: &Path) -> Result<(String, String)> {
    if let Ok(sym) = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        if let Some(branch) = sym.strip_prefix("origin/") {
            // Only honour origin/HEAD when its branch resolves locally; otherwise
            // fall through so a stale pointer doesn't make the repo disappear.
            if let Ok(sha) = git(repo, &["rev-parse", &format!("refs/heads/{branch}")]) {
                return Ok((branch.to_string(), sha));
            }
        }
    }
    for candidate in ["main", "master"] {
        if let Ok(sha) = git(
            repo,
            &["rev-parse", "--verify", &format!("refs/heads/{candidate}")],
        ) {
            return Ok((candidate.to_string(), sha));
        }
    }
    bail!(
        "couldn't find the default branch in {} (expected origin/HEAD, main or master)",
        repo.display()
    )
}

/// A git repository discovered under a configured root (deduped by common-dir).
#[derive(Debug, Clone)]
pub struct RepoCandidate {
    pub path: PathBuf,
    /// Canonical repo name from `--git-common-dir` (computed once during dedup).
    pub repo_name: String,
}

/// An open loop: an unmerged branch with its own commits.
#[derive(Debug, Clone)]
pub struct OpenLoop {
    pub root_label: String,
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub head_sha: String,
    pub last_commit: DateTime<Utc>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

impl OpenLoop {
    /// Canonical key used in resume/ignore: "root-label/repo/branch".
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.root_label, self.repo_name, self.branch)
    }
}

const SKIP_DIRS: [&str; 2] = ["node_modules", "target"];

fn looks_like_bare(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}

fn is_repo_candidate(dir: &Path) -> bool {
    dir.join(".git").exists() || looks_like_bare(dir)
}

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
    base.strip_suffix(".git").map(str::to_owned).unwrap_or(base)
}

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

// PERF-1: git_common_dir is called twice per repo — once in dedup_candidates
// (computed but not stored), and again in open_loops. Reusing the value from
// dedup would require changing RepoCandidate or open_loops's public signature.
// Both are internal, but threading the common_dir through without altering the
// public API would require wrapping it in a private helper that cli.rs doesn't call.
// Current cost: negligible (one extra git call per repo per scan), acceptable
// trade-off for keeping the public signature stable. Revisit if scan latency
// becomes dominated by this call (measure: `time loops scan --fresh`).

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

fn normalize_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

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
        .filter_map(|e| e.branch.map(|b| (b, normalize_path(e.path))))
        .collect())
}

/// Walks roots up to `scan_depth` looking for git repo candidates, then
/// deduplicates by absolute `--git-common-dir`.
pub fn find_repos(roots: &[PathBuf], scan_depth: usize) -> (Vec<RepoCandidate>, Vec<String>) {
    let mut candidates = Vec::new();
    for root in roots {
        walk(root, 0, scan_depth, &mut candidates);
    }
    dedup_candidates(candidates)
}

fn dedup_candidates(candidates: Vec<PathBuf>) -> (Vec<RepoCandidate>, Vec<String>) {
    use std::collections::HashMap;
    let mut by_common: HashMap<PathBuf, RepoCandidate> = HashMap::new();
    let mut warnings = Vec::new();
    for candidate in candidates {
        match git_common_dir(&candidate) {
            Ok(common) => {
                let repo_name = repo_name_from_common_dir(&common);
                by_common.entry(common).or_insert(RepoCandidate {
                    path: candidate,
                    repo_name,
                });
            }
            Err(e) => {
                warnings.push(format!("{}: {e:#}", candidate.display()));
            }
        }
    }
    let mut repos: Vec<RepoCandidate> = by_common.into_values().collect();
    repos.sort_by(|a, b| a.path.cmp(&b.path));
    (repos, warnings)
}

fn walk(dir: &Path, depth: usize, scan_depth: usize, candidates: &mut Vec<PathBuf>) {
    if is_repo_candidate(dir) {
        candidates.push(dir.to_path_buf());
        return;
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

/// Path-based repo name guess when `git rev-parse --git-common-dir` fails.
/// Primary naming comes from common-dir during dedup; this is the error fallback only.
pub fn repo_name_hint(path: &Path) -> String {
    let base = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    base.strip_suffix(".git").map(str::to_owned).unwrap_or(base)
}

/// Returns all unmerged branches (except default) in a repo, optionally reading
/// and updating the inventory memo for ahead/behind.
///
/// Light phase (default branch, merged set, `for-each-ref`) always runs. The
/// heavy phase (`rev-list` for ahead/behind) runs only when
/// `opts.need_ahead_behind` is true, and consults the inventory memo unless
/// `opts.fresh` is set.
///
/// Returns the open loops and, when memoisation is active, the updated
/// `(hash, InventoryFile)` pair for write-through by the caller.
///
/// # Errors
///
/// Returns `Err` if git fails or if the default branch is not found.
pub fn open_loops(
    repo: &Path,
    root_label: &str,
    opts: &ScanOptions,
) -> Result<(Vec<OpenLoop>, Option<InvUpdate>)> {
    // Resolve default branch and its SHA once (PERF-2: avoid duplicate rev-parse).
    let (default, default_sha) = default_branch_and_sha(repo)?;

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

    // Determine whether to use the inventory memo for this scan.
    let use_inventory = opts.need_ahead_behind && opts.inventory_dir.is_some();

    // Robustness: if default_sha is empty, skip memoisation to avoid poisoning the cache.
    let use_inventory = use_inventory && !default_sha.is_empty();

    let hash = if use_inventory {
        inventory::common_dir_hash(&common_dir)
    } else {
        String::new()
    };

    // Load the existing inventory file unless `--fresh` was requested.
    // Destructure inventory_dir once to avoid .unwrap() landmine.
    let existing: Option<InventoryFile> = if use_inventory && !opts.fresh {
        if let Some(inv_dir) = &opts.inventory_dir {
            let store = InventoryStore {
                dir: inv_dir.clone(),
            };
            store.load(&hash)
        } else {
            None
        }
    } else {
        None
    };

    let now = Utc::now();
    let repo_canonical = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let mut new_memos: Vec<LoopMemo> = Vec::new();
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

        let (ahead, behind) = if opts.need_ahead_behind {
            let cached = if use_inventory {
                existing.as_ref().and_then(|f| {
                    inventory::lookup_ahead_behind(
                        f,
                        branch,
                        sha,
                        &default_sha,
                        opts.inventory_ttl_secs,
                        now,
                    )
                })
            } else {
                None
            };

            let (a, b) = if let Some(hit) = cached {
                hit
            } else {
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
                let behind_val: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
                let ahead_val: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
                (ahead_val, behind_val)
            };

            if use_inventory {
                new_memos.push(LoopMemo {
                    branch: branch.to_string(),
                    head_sha: sha.to_string(),
                    ab_base_sha: default_sha.clone(),
                    ahead: a,
                    behind: b,
                });
            }
            (Some(a), Some(b))
        } else {
            (None, None)
        };

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

    let inventory_update = if use_inventory {
        Some((
            hash,
            InventoryFile {
                repo_path: repo_canonical,
                indexed_at: now,
                loops: new_memos,
            },
        ))
    } else {
        None
    };

    Ok((result, inventory_update))
}

/// Scans all repos found under the roots in parallel.
///
/// `repo_filter`, when set, retains only repos whose canonical name (from dedup)
/// matches before `open_loops` runs. Individual repo failures become warnings and
/// never abort the scan.
///
/// Returns `(loops, warnings, inventory_updates)` where `inventory_updates` is a
/// vec of `(hash, file)` pairs ready for write-through by the caller.
pub fn scan(
    roots: &[PathBuf],
    labels: &[(PathBuf, String)],
    scan_depth: usize,
    opts: &ScanOptions,
    repo_filter: Option<&str>,
) -> (Vec<OpenLoop>, Vec<String>, Vec<InvUpdate>) {
    let (mut repos, mut warnings) = find_repos(roots, scan_depth);
    if let Some(filter) = repo_filter {
        let needle = filter.to_lowercase();
        repos.retain(|r| r.repo_name.to_lowercase().contains(&needle));
    }
    let results: Vec<Result<(Vec<OpenLoop>, Option<InvUpdate>)>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| {
                let label = crate::config::label_for_repo(labels, &repo.path);
                let path = repo.path.clone();
                s.spawn(move || open_loops(&path, &label, opts))
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
    let mut inventory_updates = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok((mut loops, inv)) => {
                all.append(&mut loops);
                if let Some(update) = inv {
                    inventory_updates.push(update);
                }
            }
            Err(e) => warnings.push(format!("{}: {e:#}", repo.path.display())),
        }
    }
    (all, warnings, inventory_updates)
}

/// Branch-exclusive commits relative to the default (for the distillation prompt).
///
/// # Errors
///
/// Returns `Err` if git fails.
pub fn git_log(repo: &Path, default: &str, branch: &str) -> Result<String> {
    git(repo, &["log", "--oneline", &format!("{default}..{branch}")])
}

/// Diffstat of the branch against the base (for the distillation prompt).
///
/// # Errors
///
/// Returns `Err` if git fails.
pub fn diffstat(repo: &Path, default: &str, branch: &str) -> Result<String> {
    git(repo, &["diff", "--stat", &format!("{default}...{branch}")])
}

/// Time window of the branch-exclusive commits.
///
/// Used to filter out AI sessions that predate the branch work.
///
/// # Errors
///
/// Returns `Err` if git fails or if there are no commits on the branch.
pub fn commit_window(
    repo: &Path,
    default: &str,
    branch: &str,
) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let raw = git(
        repo,
        &["log", "--format=%cI", &format!("{default}..{branch}")],
    )?;
    let mut dates: Vec<DateTime<Utc>> = raw
        .lines()
        .filter_map(|l| DateTime::parse_from_rfc3339(l.trim()).ok())
        .map(|d| d.with_timezone(&Utc))
        .collect();
    if dates.is_empty() {
        // branch has no exclusive commit: fall back to its latest commit
        let head = git(repo, &["log", "-1", "--format=%cI", branch])?;
        dates.push(DateTime::parse_from_rfc3339(head.trim())?.with_timezone(&Utc));
    }
    let min = dates
        .iter()
        .min()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("no commit dates for {branch}"))?;
    let max = dates
        .iter()
        .max()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("no commit dates for {branch}"))?;
    Ok((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    /// Helper: call `open_loops` without inventory, returning only the loops vec.
    fn open_loops_simple(
        repo: &std::path::Path,
        root_label: &str,
        need_ahead_behind: bool,
    ) -> Vec<OpenLoop> {
        let opts = ScanOptions {
            need_ahead_behind,
            ..ScanOptions::default()
        };
        open_loops(repo, root_label, &opts).unwrap().0
    }

    /// Helper: call `scan` without inventory, returning only `(loops, warnings)`.
    fn scan_simple(
        roots: &[PathBuf],
        labels: &[(PathBuf, String)],
        depth: usize,
        need_ahead_behind: bool,
        filter: Option<&str>,
    ) -> (Vec<OpenLoop>, Vec<String>) {
        let opts = ScanOptions {
            need_ahead_behind,
            ..ScanOptions::default()
        };
        let (loops, warnings, _inv) = scan(roots, labels, depth, &opts, filter);
        (loops, warnings)
    }

    fn assert_same_path(actual: &std::path::Path, expected: &std::path::Path) {
        let a = std::fs::canonicalize(actual).unwrap_or_else(|_| actual.to_path_buf());
        let b = std::fs::canonicalize(expected).unwrap_or_else(|_| expected.to_path_buf());
        assert_eq!(a, b);
    }

    #[test]
    fn default_branch_detects_main() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        assert_eq!(default_branch(&repo).unwrap(), "main");
    }

    #[test]
    fn git_fails_with_contextual_message() {
        let tmp = tempfile::tempdir().unwrap();
        // directory is not a git repo
        let err = git(tmp.path(), &["status"]).unwrap_err();
        assert!(err.to_string().contains(&tmp.path().display().to_string()));
    }

    #[test]
    fn find_repos_dedups_container_and_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        let dev = container.join("dev");
        testutil::add_named_worktree(&container, "dev", "dev");
        let (repos, warnings) = find_repos(&[container.clone(), dev], 4);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].path, container);
    }

    #[test]
    fn find_repos_respects_scan_depth_and_skips_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        testutil::init_repo(&tmp.path().join("a/b/c/repo-deep"));
        testutil::init_repo(&tmp.path().join("a/b/repo-mid"));
        testutil::init_repo(&tmp.path().join("repo-shallow"));
        testutil::init_repo(&tmp.path().join(".hidden/repo3"));

        let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
        let names: Vec<_> = repos
            .iter()
            .filter_map(|r| r.path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"repo-deep".to_string()));
        assert!(names.contains(&"repo-mid".to_string()));
        assert!(names.contains(&"repo-shallow".to_string()));
        assert!(!names.contains(&"repo3".to_string()));

        let (shallow, _) = find_repos(&[tmp.path().to_path_buf()], 2);
        let shallow_names: Vec<_> = shallow
            .iter()
            .filter_map(|r| r.path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert!(!shallow_names.contains(&"repo-deep".to_string()));
        assert!(shallow_names.contains(&"repo-shallow".to_string()));
    }

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
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].path, container);
    }

    #[test]
    fn find_repos_finds_pure_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("foo.git");
        testutil::init_bare_repo(&bare);
        testutil::seed_bare_main(&bare);
        let (repos, _) = find_repos(&[tmp.path().to_path_buf()], 4);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].path, bare);
    }

    #[test]
    fn open_loops_uses_common_dir_repo_name_in_bare_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        testutil::add_named_worktree(&container, "dev", "dev");
        testutil::add_branch_on_bare(&container.join(".bare"), "feat/x", "x.txt");

        let loops = open_loops_simple(&container, "root", true);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].repo_name, "my-app");
        assert_eq!(loops[0].branch, "feat/x");
        assert_eq!(loops[0].key(), "root/my-app/feat/x");
    }

    #[test]
    fn open_loops_bare_root_repo_name_strips_dot_git_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("foo.git");
        testutil::init_bare_repo(&bare);
        testutil::seed_bare_main(&bare);
        testutil::add_branch_on_bare(&bare, "feat/y", "y.txt");

        let loops = open_loops_simple(&bare, "r", true);
        assert_eq!(loops[0].repo_name, "foo");
    }

    #[test]
    fn open_loops_finds_unmerged_ignores_merged_and_default() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        testutil::git(&repo, &["branch", "merged"]); // points to main => merged

        let loops = open_loops_simple(&repo, "root", true);
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.branch, "feat/x");
        assert_eq!(l.repo_name, "app");
        assert_eq!(l.root_label, "root");
        assert_eq!(l.key(), "root/app/feat/x");
        assert_eq!(l.ahead, Some(1));
        assert_eq!(l.behind, Some(0));
        assert_eq!(l.head_sha.len(), 40);
    }

    #[test]
    fn open_loops_sets_repo_path_to_worktree_when_branch_checked_out() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        testutil::add_worktree_with_commit(&container, "feat-x", "feat/x", "x.txt");

        let loops = open_loops_simple(&container, "root", true);
        let lp = loops
            .iter()
            .find(|l| l.branch == "feat/x")
            .expect("feat/x loop");
        assert_same_path(&lp.repo_path, &container.join("feat-x"));
    }

    #[test]
    fn open_loops_falls_back_to_container_when_branch_has_no_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        // feat/y exists in the store but is NOT checked out in any worktree
        testutil::add_branch_on_bare(&container.join(".bare"), "feat/y", "y.txt");

        let loops = open_loops_simple(&container, "root", true);
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
        let loops = open_loops_simple(&repo, "root", true);
        assert_eq!(loops[0].branch, "feat/x");
        assert_eq!(loops[0].repo_path, repo); // not checked out in a worktree → fallback
    }

    #[test]
    fn open_loops_skips_rev_list_when_need_ahead_behind_false() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let loops = open_loops_simple(&repo, "root", false);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].ahead, None);
        assert_eq!(loops[0].behind, None);
    }

    #[test]
    fn open_loops_computes_ahead_behind_when_need_ahead_behind_true() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let loops = open_loops_simple(&repo, "root", true);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].ahead, Some(1));
        assert_eq!(loops[0].behind, Some(0));
    }

    #[test]
    fn open_loops_reuses_inventory_memo_on_repeated_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        let inv_dir = tmp.path().join("inv");

        let opts = ScanOptions {
            need_ahead_behind: true,
            fresh: false,
            inventory_dir: Some(inv_dir.clone()),
            inventory_ttl_secs: 0,
        };

        // First call: no cache → runs rev-list and writes inventory.
        let (loops1, inv1) = open_loops(&repo, "root", &opts).unwrap();
        assert_eq!(loops1.len(), 1);
        assert_eq!(loops1[0].ahead, Some(1));
        let (hash, file) = inv1.unwrap();
        let store = InventoryStore {
            dir: inv_dir.clone(),
        };
        store.save(&hash, &file).unwrap();

        // Second call: memo present → cache hit; ahead/behind same.
        let (loops2, inv2) = open_loops(&repo, "root", &opts).unwrap();
        assert_eq!(loops2.len(), 1);
        assert_eq!(loops2[0].ahead, Some(1));
        assert_eq!(loops2[0].behind, Some(0));
        // inventory update is still returned (for write-through)
        assert!(inv2.is_some());
    }

    #[test]
    fn open_loops_fresh_ignores_inventory_memo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        let inv_dir = tmp.path().join("inv");

        // Pre-seed inventory with wrong ahead/behind values to detect if it's
        // being used.
        let common = git_common_dir(&repo).unwrap();
        let hash = crate::inventory::common_dir_hash(&common);
        let store = InventoryStore {
            dir: inv_dir.clone(),
        };
        let fake_sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let stub_file = InventoryFile {
            repo_path: repo.clone(),
            indexed_at: chrono::Utc::now(),
            loops: vec![LoopMemo {
                branch: "feat/x".to_string(),
                head_sha: fake_sha.to_string(),
                ab_base_sha: fake_sha.to_string(),
                ahead: 99,
                behind: 99,
            }],
        };
        store.save(&hash, &stub_file).unwrap();

        let opts = ScanOptions {
            need_ahead_behind: true,
            fresh: true, // <-- bypass cache
            inventory_dir: Some(inv_dir.clone()),
            inventory_ttl_secs: 0,
        };
        let (loops, _) = open_loops(&repo, "root", &opts).unwrap();
        // real values, not the stubbed 99/99
        assert_eq!(loops[0].ahead, Some(1));
        assert_eq!(loops[0].behind, Some(0));
    }

    #[test]
    fn scan_repo_filter_pushdown_skips_non_matching_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let api = tmp.path().join("api-service");
        let web = tmp.path().join("web-app");
        testutil::init_repo(&api);
        testutil::init_repo(&web);
        testutil::add_branch_with_commit(&api, "feat/api", "a.txt");
        testutil::add_branch_with_commit(&web, "feat/web", "w.txt");

        let labels = vec![(tmp.path().to_path_buf(), "r".to_string())];
        let (loops, _) = scan_simple(&[tmp.path().to_path_buf()], &labels, 4, false, Some("api"));
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].repo_name, "api-service");
        assert_eq!(loops[0].branch, "feat/api");
    }

    #[test]
    fn repo_name_hint_strips_dot_git_suffix() {
        assert_eq!(repo_name_hint(std::path::Path::new("/srv/foo.git")), "foo");
    }

    #[test]
    fn scan_repo_filter_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let api = tmp.path().join("API-Service");
        testutil::init_repo(&api);
        testutil::add_branch_with_commit(&api, "feat/api", "a.txt");

        let labels = vec![(tmp.path().to_path_buf(), "r".to_string())];
        // lowercase filter must match a mixed-case repo dir (both sides lowered)
        let (loops, _) = scan_simple(&[tmp.path().to_path_buf()], &labels, 4, false, Some("api"));
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].repo_name, "API-Service");
    }

    #[test]
    fn scan_repo_filter_matching_nothing_yields_no_loops() {
        let tmp = tempfile::tempdir().unwrap();
        let api = tmp.path().join("api-service");
        testutil::init_repo(&api);
        testutil::add_branch_with_commit(&api, "feat/api", "a.txt");

        let labels = vec![(tmp.path().to_path_buf(), "r".to_string())];
        let (loops, warnings) = scan_simple(
            &[tmp.path().to_path_buf()],
            &labels,
            4,
            false,
            Some("zzz-nope"),
        );
        assert!(loops.is_empty());
        assert!(
            warnings.is_empty(),
            "filtered-out repos must not warn: {warnings:?}"
        );
    }

    #[test]
    fn scan_aggregates_repos_and_reports_warning_without_aborting() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("good");
        testutil::init_repo(&good);
        testutil::add_branch_with_commit(&good, "feat/ok", "ok.txt");
        // truly broken repo: no commits, so default_branch fails
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        testutil::git(&empty, &["init", "-b", "main"]);

        let labels = vec![(tmp.path().to_path_buf(), "r".to_string())];
        let (loops, warnings) = scan_simple(&[tmp.path().to_path_buf()], &labels, 4, true, None);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].key(), "r/good/feat/ok");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("empty"));
    }

    #[test]
    fn context_helpers_return_commits_and_window() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let log = git_log(&repo, "main", "feat/x").unwrap();
        assert!(log.contains("wip feat/x"));
        let stat = diffstat(&repo, "main", "feat/x").unwrap();
        assert!(stat.contains("x.txt"));
        let (start, end) = commit_window(&repo, "main", "feat/x").unwrap();
        assert!(start <= end);
    }

    #[test]
    fn default_branch_detects_master_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        testutil::git(repo, &["init", "-b", "master"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        testutil::git(repo, &["add", "."]);
        testutil::git(repo, &["commit", "-m", "init"]);
        assert_eq!(default_branch(repo).unwrap(), "master");
    }

    #[test]
    fn default_branch_errors_without_main_or_master() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        testutil::git(repo, &["init", "-b", "trunk"]);
        // no commits: refs/heads/main and refs/heads/master do not exist
        let err = default_branch(repo).unwrap_err();
        assert!(err.to_string().contains("couldn't find the default branch"));
    }

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
        assert_eq!(
            entries[0].path,
            std::path::PathBuf::from("/home/u/app/main")
        );
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

    #[test]
    fn worktree_map_maps_checked_out_branches_to_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container); // main worktree at container/main
        testutil::add_named_worktree(&container, "dev", "dev"); // dev worktree at container/dev

        let map = worktree_map(&container).unwrap();
        assert_same_path(map.get("main").unwrap(), &container.join("main"));
        assert_same_path(map.get("dev").unwrap(), &container.join("dev"));
        // the `.bare` entry is filtered out (no branch / bare)
        assert!(!map.values().any(|p| p.ends_with(".bare")));
    }

    #[test]
    fn worktree_map_errors_on_non_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // a plain directory is not a git repo → git worktree list fails
        assert!(worktree_map(tmp.path()).is_err());
    }

    #[test]
    fn parse_worktree_porcelain_ignores_lines_before_first_worktree() {
        let out = "branch refs/heads/orphan\nHEAD deadbeef\nworktree /home/u/app/main\nbranch refs/heads/main\n";
        let entries = parse_worktree_porcelain(out);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].path,
            std::path::PathBuf::from("/home/u/app/main")
        );
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn repo_name_from_common_dir_table() {
        use std::path::Path;

        let cases: &[(&str, &str)] = &[
            ("/home/u/my-app/.bare", "my-app"),
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
}
