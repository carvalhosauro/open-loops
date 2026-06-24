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

/// An open loop: an unmerged branch with its own commits.
#[derive(Debug, Clone)]
pub struct OpenLoop {
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub head_sha: String,
    pub last_commit: DateTime<Utc>,
    pub ahead: u32,
    pub behind: u32,
}

impl OpenLoop {
    /// Canonical key used in resume/ignore: "repo/branch".
    pub fn key(&self) -> String {
        format!("{}/{}", self.repo_name, self.branch)
    }
}

const MAX_DEPTH: usize = 3;
const SKIP_DIRS: [&str; 2] = ["node_modules", "target"];

/// Walks roots up to MAX_DEPTH looking for directories with .git.
///
/// Hidden directories (name starts with `.`) and those listed in `SKIP_DIRS`
/// are skipped.
pub fn find_repos(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for root in roots {
        walk(root, 0, &mut repos);
    }
    repos.sort();
    repos
}

fn walk(dir: &Path, depth: usize, repos: &mut Vec<PathBuf>) {
    if dir.join(".git").is_dir() {
        repos.push(dir.to_path_buf());
        return;
    }
    if depth >= MAX_DEPTH {
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
        walk(&path, depth + 1, repos);
    }
}

/// Returns all unmerged branches (except default) in a repo.
///
/// # Errors
///
/// Returns `Err` if git fails or if the default branch is not found.
pub fn open_loops(repo: &Path) -> Result<Vec<OpenLoop>> {
    let default = default_branch(repo)?;
    let merged: std::collections::HashSet<String> = git(
        repo,
        &["branch", "--merged", &default, "--format=%(refname:short)"],
    )?
    .lines()
    .map(|s| s.trim().to_string())
    .collect();
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string());
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
        result.push(OpenLoop {
            repo_name: repo_name.clone(),
            repo_path: repo.to_path_buf(),
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
/// Individual repo failures become warnings and never abort the scan.
pub fn scan(roots: &[PathBuf]) -> (Vec<OpenLoop>, Vec<String>) {
    let repos = find_repos(roots);
    let results: Vec<Result<Vec<OpenLoop>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| s.spawn(move || open_loops(repo)))
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
    let mut warnings = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut loops) => all.append(&mut loops),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
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
    fn find_repos_finds_repos_up_to_depth_3_and_skips_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        testutil::init_repo(&tmp.path().join("a/b/repo1"));
        testutil::init_repo(&tmp.path().join("repo2"));
        testutil::init_repo(&tmp.path().join(".hidden/repo3"));
        let repos = find_repos(&[tmp.path().to_path_buf()]);
        let names: Vec<_> = repos
            .iter()
            .map(|r| r.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"repo1".to_string()));
        assert!(names.contains(&"repo2".to_string()));
        assert!(!names.contains(&"repo3".to_string()));
    }

    #[test]
    fn open_loops_finds_unmerged_ignores_merged_and_default() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        testutil::git(&repo, &["branch", "merged"]); // points to main => merged

        let loops = open_loops(&repo).unwrap();
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.branch, "feat/x");
        assert_eq!(l.repo_name, "app");
        assert_eq!(l.key(), "app/feat/x");
        assert_eq!(l.ahead, 1);
        assert_eq!(l.behind, 0);
        assert_eq!(l.head_sha.len(), 40);
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

        let (loops, warnings) = scan(&[tmp.path().to_path_buf()]);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].key(), "good/feat/ok");
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
}
