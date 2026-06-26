//! Repository and unmerged-branch discovery via git shell-out.
//! Design decision: shell-out (not git2/gix) — simple and debuggable;
//! the product performance bottleneck is the LLM, not git.
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Default branch: origin/HEAD if it exists; otherwise main; otherwise master.
///
/// # Errors
///
/// Returns `Err` if no default branch is found.
pub fn default_branch(repo: &Path) -> Result<String> {
    if let Ok(sym) = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        if let Some(branch) = sym.strip_prefix("origin/") {
            return Ok(branch.to_string());
        }
    }
    for candidate in ["main", "master"] {
        if git(
            repo,
            &["rev-parse", "--verify", &format!("refs/heads/{candidate}")],
        )
        .is_ok()
        {
            return Ok(candidate.to_string());
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
                by_common
                    .entry(common)
                    .or_insert(RepoCandidate {
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

/// Returns all unmerged branches (except default) in a repo.
///
/// Light phase (default branch, merged set, for-each-ref) always runs. The heavy
/// phase (`rev-list` for ahead/behind) runs only when `need_ahead_behind` is true.
///
/// # Errors
///
/// Returns `Err` if git fails or if the default branch is not found.
pub fn open_loops(repo: &Path, root_label: &str, need_ahead_behind: bool) -> Result<Vec<OpenLoop>> {
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
        let (ahead, behind) = if need_ahead_behind {
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
            (Some(ahead), Some(behind))
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
    Ok(result)
}

/// Scans all repos found under the roots in parallel.
///
/// `repo_filter`, when set, retains only repos whose canonical name (from dedup)
/// matches before `open_loops` runs. Individual repo failures become warnings and
/// never abort the scan.
pub fn scan(
    roots: &[PathBuf],
    labels: &[(PathBuf, String)],
    scan_depth: usize,
    need_ahead_behind: bool,
    repo_filter: Option<&str>,
) -> (Vec<OpenLoop>, Vec<String>) {
    let (mut repos, mut warnings) = find_repos(roots, scan_depth);
    if let Some(filter) = repo_filter {
        let needle = filter.to_lowercase();
        repos.retain(|r| r.repo_name.to_lowercase().contains(&needle));
    }
    let results: Vec<Result<Vec<OpenLoop>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| {
                let label = crate::config::label_for_repo(labels, &repo.path);
                let path = repo.path.clone();
                s.spawn(move || open_loops(&path, &label, need_ahead_behind))
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
            Err(e) => warnings.push(format!("{}: {e:#}", repo.path.display())),
        }
    }
    (all, warnings)
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

        let loops = open_loops(&container, "root", true).unwrap();
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

        let loops = open_loops(&bare, "r", true).unwrap();
        assert_eq!(loops[0].repo_name, "foo");
    }

    #[test]
    fn open_loops_finds_unmerged_ignores_merged_and_default() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        testutil::git(&repo, &["branch", "merged"]); // points to main => merged

        let loops = open_loops(&repo, "root", true).unwrap();
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

        let loops = open_loops(&container, "root", true).unwrap();
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

        let loops = open_loops(&container, "root", true).unwrap();
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
        let loops = open_loops(&repo, "root", true).unwrap();
        assert_eq!(loops[0].branch, "feat/x");
        assert_eq!(loops[0].repo_path, repo); // not checked out in a worktree → fallback
    }

    #[test]
    fn open_loops_skips_rev_list_when_need_ahead_behind_false() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let loops = open_loops(&repo, "root", false).unwrap();
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

        let loops = open_loops(&repo, "root", true).unwrap();
        assert_eq!(loops.len(), 1);
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
        let (loops, _) = scan(&[tmp.path().to_path_buf()], &labels, 4, false, Some("api"));
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
        let (loops, _) = scan(&[tmp.path().to_path_buf()], &labels, 4, false, Some("api"));
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
        let (loops, warnings) = scan(
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
        let (loops, warnings) = scan(&[tmp.path().to_path_buf()], &labels, 4, true, None);
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
        assert_eq!(map.get("main"), Some(&container.join("main")));
        assert_eq!(map.get("dev"), Some(&container.join("dev")));
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
