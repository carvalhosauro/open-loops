//! Worktree inventory: joins `git worktree list` with merged/idle/state signals.
use crate::error::{error_chain, GitError};
use crate::scanner::{default_branch, find_repos, git, parse_worktree_porcelain, WorktreeEntry};
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, GitError>;

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

/// The cheap, once-per-repo context needed to build every `Worktree` of a repo:
/// its name, the merged-into-default branch set, and the non-bare worktree
/// entries (in porcelain order — the first is the main checkout). Gathering this
/// is 1–2 git calls per repo; the expensive per-worktree probes come after.
struct RepoWtContext {
    repo_name: String,
    repo_path: PathBuf,
    merged_set: HashSet<String>,
    entries: Vec<WorktreeEntry>,
}

/// Gathers the per-repo context (`worktree list` + default branch + merged set).
/// Bare entries are dropped here so downstream indices line up with real
/// worktrees.
///
/// # Errors
///
/// Returns `Err` if `git worktree list` fails.
fn worktree_context(repo: &Path) -> Result<RepoWtContext> {
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
    let entries = parse_worktree_porcelain(&raw)
        .into_iter()
        .filter(|e| !e.bare)
        .collect();
    Ok(RepoWtContext {
        repo_name,
        repo_path: repo.to_path_buf(),
        merged_set,
        entries,
    })
}

/// The per-worktree probe: latest commit time + dirty flag, via two git calls
/// (`log -1`, `status --porcelain`). Prunable entries skip the probe entirely —
/// their directory is gone. This is the hot path #18 parallelises: it used to
/// run serially for every worktree of every repo.
fn probe_worktree(entry: &WorktreeEntry) -> (Option<DateTime<Utc>>, bool) {
    if entry.prunable {
        return (None, false);
    }
    let last_commit = git(&entry.path, &["log", "-1", "--format=%cI"])
        .ok()
        .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
        .map(|d| d.with_timezone(&Utc));
    let status = git(&entry.path, &["status", "--porcelain"]).unwrap_or_default();
    (last_commit, !status.trim().is_empty())
}

/// Assembles a `Worktree` from its repo context, its position in that repo's
/// entry list (index 0 is the main checkout), and its probe result.
fn assemble_worktree(
    ctx: &RepoWtContext,
    idx: usize,
    entry: &WorktreeEntry,
    (last_commit, dirty): (Option<DateTime<Utc>>, bool),
) -> Worktree {
    let merged = entry
        .branch
        .as_ref()
        .map(|b| ctx.merged_set.contains(b))
        .unwrap_or(false);
    Worktree {
        repo_name: ctx.repo_name.clone(),
        repo_path: ctx.repo_path.clone(),
        worktree_path: entry.path.clone(),
        branch: entry.branch.clone(),
        last_commit,
        merged,
        dirty,
        prunable: entry.prunable,
        is_main: idx == 0,
    }
}

/// Enumerates and classifies a repository's worktrees.
///
/// The per-worktree probes (`log -1` + `status`) run on a bounded worker pool
/// instead of serially (#18), so a repo with many worktrees no longer pays a
/// strictly serial `2 × N` git round-trips.
///
/// # Errors
///
/// Returns `Err` if `git worktree list` fails.
pub fn worktrees(repo: &Path) -> Result<Vec<Worktree>> {
    let ctx = worktree_context(repo)?;
    let probes = crate::parallel::try_map(
        &ctx.entries,
        crate::parallel::default_concurrency(),
        "panic while probing worktree",
        |e| Ok::<_, GitError>(probe_worktree(e)),
    );
    Ok(ctx
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            // `probe_worktree` never errors; a caught panic degrades to (None, false).
            let probe = probes[i].as_ref().copied().unwrap_or((None, false));
            assemble_worktree(&ctx, i, e, probe)
        })
        .collect())
}

/// Scans worktrees of all repos found under the roots, in parallel.
///
/// Per-repo failures become warnings, never abort. Concurrency is bounded and
/// applied at a SINGLE level (#16/#18): the per-repo contexts are gathered on
/// the pool, then EVERY worktree across ALL repos is flattened into one probe
/// task list and run through the same pool — so a few repos with many worktrees
/// (or many repos with a few) both saturate the workers without nesting pools.
pub fn scan_worktrees(roots: &[PathBuf], scan_depth: usize) -> (Vec<Worktree>, Vec<String>) {
    let (repos, mut warnings) = find_repos(roots, scan_depth);
    let cap = crate::parallel::default_concurrency();

    // Phase 1: gather each repo's cheap context on the bounded pool.
    let contexts = crate::parallel::try_map(&repos, cap, "panic while scanning worktrees", |r| {
        worktree_context(&r.path)
    });
    let mut good: Vec<RepoWtContext> = Vec::new();
    for (repo, res) in repos.iter().zip(contexts) {
        match res {
            Ok(ctx) => good.push(ctx),
            Err(e) => warnings.push(format!("{}: {}", repo.path.display(), error_chain(&e))),
        }
    }

    // Phase 2: flatten to (context idx, entry idx) probe tasks and run the hot
    // per-worktree git calls through one bounded pool — no per-repo barrier.
    let tasks: Vec<(usize, usize)> = good
        .iter()
        .enumerate()
        .flat_map(|(ci, ctx)| (0..ctx.entries.len()).map(move |ei| (ci, ei)))
        .collect();
    let probes =
        crate::parallel::try_map(&tasks, cap, "panic while probing worktree", |&(ci, ei)| {
            Ok::<_, GitError>(probe_worktree(&good[ci].entries[ei]))
        });

    // Reassemble in stable order: repo order, then entry order.
    let mut all = Vec::with_capacity(tasks.len());
    for (&(ci, ei), res) in tasks.iter().zip(probes) {
        let ctx = &good[ci];
        let probe = res.unwrap_or((None, false));
        all.push(assemble_worktree(ctx, ei, &ctx.entries[ei], probe));
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
