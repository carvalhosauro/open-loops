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
