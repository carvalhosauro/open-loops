//! Worktree inventory: joins `git worktree list` with merged/idle/state signals.
use crate::scanner::{default_branch, find_repos, git, parse_worktree_porcelain};
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

/// Enumerates and classifies a repository's worktrees.
///
/// # Errors
///
/// Returns `Err` if `git worktree list` fails.
pub fn worktrees(repo: &Path) -> Result<Vec<Worktree>> {
    let raw = git(repo, &["worktree", "list", "--porcelain"])?;
    let default = default_branch(repo).ok();
    let merged_set: HashSet<String> = match &default {
        Some(d) => git(
            repo,
            &["branch", "--merged", d, "--format=%(refname:short)"],
        )
        .unwrap_or_default()
        .lines()
        .map(|s| s.trim().to_string())
        // drop the default branch itself: "merged" means merged INTO default
        .filter(|b| !b.is_empty() && b != d)
        .collect(),
        None => HashSet::new(),
    };
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string());

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
}

/// Scans worktrees of all repos found under the roots, in parallel.
///
/// Per-repo failures become warnings, never abort.
pub fn scan_worktrees(roots: &[PathBuf], scan_depth: usize) -> (Vec<Worktree>, Vec<String>) {
    let (repos, mut warnings) = find_repos(roots, scan_depth);
    let results: Vec<Result<Vec<Worktree>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|r| s.spawn(move || worktrees(r)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("panic while scanning worktrees")))
            })
            .collect()
    });
    let mut all = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut w) => all.append(&mut w),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
        }
    }
    (all, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    fn wt(
        branch: Option<&str>,
        merged: bool,
        dirty: bool,
        prunable: bool,
        is_main: bool,
    ) -> Worktree {
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
        assert_eq!(
            wt(Some("main"), true, false, false, true).verdict(),
            Verdict::Home
        );
        assert_eq!(
            wt(Some("x"), false, false, true, false).verdict(),
            Verdict::Prunable
        );
        assert_eq!(
            wt(Some("x"), false, true, false, false).verdict(),
            Verdict::Active
        );
        assert_eq!(
            wt(Some("x"), true, false, false, false).verdict(),
            Verdict::Deletable
        );
        assert_eq!(
            wt(Some("x"), false, false, false, false).verdict(),
            Verdict::Cold
        );
        // detached clean -> active
        assert_eq!(
            wt(None, false, false, false, false).verdict(),
            Verdict::Active
        );
        // is_main beats prunable/dirty
        assert_eq!(
            wt(Some("main"), false, true, true, true).verdict(),
            Verdict::Home
        );
    }

    #[test]
    fn short_name_uses_basename() {
        let w = wt(Some("x"), false, false, false, false);
        assert_eq!(w.short_name(), "app/x");
    }

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

        let (all, warnings) = scan_worktrees(&[tmp.path().to_path_buf()], 4);
        assert!(all
            .iter()
            .any(|w| w.branch.as_deref() == Some("feat/extra")));
        assert!(warnings.is_empty());
    }
}
