//! Repository and unmerged-branch discovery via git shell-out.
//! Design decision: shell-out (not git2/gix) — simple and debuggable;
//! the product performance bottleneck is the LLM, not git.
use crate::error::{error_chain, GitError};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::index::Index;
use crate::inventory::{self, InventoryFile, InventoryStore, LoopMemo};

/// Scanner shell-out and discovery errors.
type ScanResult<T> = Result<T, GitError>;

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
pub(crate) fn git(repo: &Path, args: &[&str]) -> ScanResult<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                GitError::NotInPath(source)
            } else {
                GitError::SpawnFailed {
                    repo: repo.to_path_buf(),
                    source,
                }
            }
        })?;
    if !out.status.success() {
        return Err(GitError::CommandFailed {
            repo: repo.to_path_buf(),
            command: format!("{args:?}"),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Default branch: origin/HEAD's target if it resolves locally; otherwise main;
/// otherwise master.
///
/// # Errors
///
/// Returns `Err` if no default branch is found.
pub fn default_branch(repo: &Path) -> ScanResult<String> {
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
fn default_branch_and_sha(repo: &Path) -> ScanResult<(String, String)> {
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
    Err(GitError::NoDefaultBranch {
        repo: repo.to_path_buf(),
    })
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
pub fn git_common_dir(path: &Path) -> ScanResult<PathBuf> {
    let raw = git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(raw))
}

/// Cheap fingerprint of a repo's refs: the MAX mtime (unix nanoseconds since the
/// epoch) of `<common_dir>/HEAD`, `<common_dir>/packed-refs` (when present), the
/// newest entry anywhere under the `<common_dir>/refs` tree, and the newest entry
/// anywhere under the `<common_dir>/worktrees/` tree. Missing files or directories
/// contribute 0.
///
/// This is the refs-fingerprint gate (#13): when it is unchanged since the last
/// index write, the cached loops are still valid and the heavy git phase
/// (`for-each-ref`, `branch --merged`, per-branch `rev-list`) can be skipped
/// entirely.
///
/// Precision note: the brief specified whole-second mtimes, but a branch created
/// (or advanced) within the same wall-clock second as the previous index write
/// would then leave the fingerprint unchanged and silently serve stale loops —
/// e.g. a brand-new branch would not appear. Using nanosecond precision closes
/// that window so a new/advanced ref is always detected, regardless of timing.
/// `i64` nanos-since-epoch overflow only in the year 2262, so the range is safe.
/// On filesystems that expose only second-granularity mtimes the sub-second part
/// is simply 0 — the gate then degrades to whole-second behaviour, never worse.
///
/// Other notes:
/// - the gate is additionally paired with `default_sha` in `cached_loops`, so a
///   moved default branch invalidates even if mtimes somehow collide.
/// - `git gc`/repacking rewrites `packed-refs`, which bumps the fingerprint
///   without a semantic change. That is acceptable — it only forces one
///   recompute, never stale data.
/// - `git worktree add`/`remove` mutates `<common_dir>/worktrees/`, which is now
///   covered: any worktree change bumps the fingerprint → gate invalidates →
///   recompute → fresh `worktree_path` values are served.
pub fn refs_fingerprint(common_dir: &Path) -> i64 {
    let mut max = 0_i64;
    max = max.max(file_mtime_nanos(&common_dir.join("HEAD")));
    max = max.max(file_mtime_nanos(&common_dir.join("packed-refs")));
    max = max.max(newest_mtime_in_tree(&common_dir.join("refs")));
    max = max.max(newest_mtime_in_tree(&common_dir.join("worktrees")));
    max
}

/// Unix mtime of a single path in nanoseconds since the epoch, or 0 when it is
/// missing / unreadable. Saturates at `i64::MAX` (year 2262) rather than wrap.
fn file_mtime_nanos(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Newest mtime (unix nanos) found by walking `dir` recursively (files and the
/// directories themselves). Returns 0 for a missing/unreadable tree.
///
/// Loose refs live as files under `refs/`; a brand-new branch creates a new file
/// (and bumps the containing directory's mtime), and advancing a branch rewrites
/// its ref file — both raise this value, which is exactly the signal the gate
/// wants. Packed refs are covered separately via `packed-refs`.
fn newest_mtime_in_tree(dir: &Path) -> i64 {
    let mut max = file_mtime_nanos(dir);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return max;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => max = max.max(newest_mtime_in_tree(&path)),
            _ => max = max.max(file_mtime_nanos(&path)),
        }
    }
    max
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
pub fn worktree_map(repo: &Path) -> ScanResult<std::collections::HashMap<String, PathBuf>> {
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
    find_repos_cached(roots, scan_depth, None)
}

/// Like [`find_repos`] but optionally consults `index` to skip `git rev-parse`
/// for already-known paths (resolves #17).
pub fn find_repos_cached(
    roots: &[PathBuf],
    scan_depth: usize,
    index: Option<&Index>,
) -> (Vec<RepoCandidate>, Vec<String>) {
    let mut candidates = Vec::new();
    for root in roots {
        walk(root, 0, scan_depth, &mut candidates);
    }
    dedup_candidates_cached(candidates, index)
}

/// Like `dedup_candidates` but optionally uses `index` to cache/reuse
/// `--git-common-dir` results, skipping the git shell-out on cache hits.
fn dedup_candidates_cached(
    candidates: Vec<PathBuf>,
    index: Option<&Index>,
) -> (Vec<RepoCandidate>, Vec<String>) {
    use std::collections::HashMap;
    let mut by_common: HashMap<PathBuf, RepoCandidate> = HashMap::new();
    let mut warnings = Vec::new();
    for candidate in candidates {
        // Try index cache first (hit = skip shell-out).
        let cached = index.and_then(|idx| idx.cached_common_dir(&candidate));
        let common_result = if let Some((_hash, common_dir)) = cached {
            Ok(common_dir)
        } else {
            // Cache miss: call git and store the result back.
            match git_common_dir(&candidate) {
                Ok(common) => {
                    if let Some(idx) = index {
                        let hash = crate::inventory::common_dir_hash(&common);
                        idx.put_repo_common_dir(&candidate, &hash, &common);
                    }
                    Ok(common)
                }
                Err(e) => Err(e),
            }
        };

        match common_result {
            Ok(common) => {
                let repo_name = repo_name_from_common_dir(&common);
                by_common.entry(common).or_insert(RepoCandidate {
                    path: candidate,
                    repo_name,
                });
            }
            Err(e) => {
                warnings.push(format!("{}: {}", candidate.display(), error_chain(&e)));
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
) -> ScanResult<(Vec<OpenLoop>, Option<InvUpdate>)> {
    open_loops_indexed(repo, root_label, opts, None)
}

/// Like [`open_loops`] but optionally consults a SQLite `index` for the
/// refs-fingerprint gate (#13).
///
/// When `index` is `Some` and `opts.fresh` is false, the gate is checked first:
/// if the repo's `refs_fingerprint` and `default_sha` are unchanged since the
/// last index write, the cached loops are returned and the heavy git phase
/// (`for-each-ref`, `branch --merged`, per-branch `rev-list`) is skipped
/// entirely. On a miss (or `opts.fresh`), the full logic runs and the result is
/// written through to the index for the next call.
///
/// When `index` is `None`, the behaviour is byte-for-byte identical to the
/// pre-index code path.
///
/// Index errors never abort a scan: git is the source of truth and the index is
/// disposable, so a degraded index simply forces a recompute.
///
/// # Errors
///
/// Returns `Err` if git fails or if the default branch is not found.
pub fn open_loops_indexed(
    repo: &Path,
    root_label: &str,
    opts: &ScanOptions,
    index: Option<&Index>,
) -> ScanResult<(Vec<OpenLoop>, Option<InvUpdate>)> {
    // Resolve default branch and its SHA once (PERF-2: avoid duplicate rev-parse).
    let (default, default_sha) = default_branch_and_sha(repo)?;

    let common_dir = git_common_dir(repo)?;
    let repo_name = repo_name_from_common_dir(&common_dir);

    // -- Refs-fingerprint gate (#13) ---------------------------------------
    // Compute the fingerprint once; reused for both the read gate and the
    // write-through below.
    let refs_fp = refs_fingerprint(&common_dir);
    let gate_hash = inventory::common_dir_hash(&common_dir);

    if let Some(idx) = index {
        if !opts.fresh {
            if let Some(rows) = idx.cached_loops(&gate_hash, refs_fp, &default_sha) {
                // A hit must serve what the caller needs: if ahead/behind were
                // requested but the cached rows lack them, treat as a miss and
                // recompute (never hand back None to a caller that asked).
                let serves = !opts.need_ahead_behind || rows.iter().all(|r| r.ahead.is_some());
                if serves {
                    let loops = rows
                        .into_iter()
                        .map(|r| OpenLoop {
                            root_label: root_label.to_string(),
                            repo_name: repo_name.clone(),
                            repo_path: r.worktree_path,
                            branch: r.branch,
                            head_sha: r.head_sha,
                            last_commit: r.last_commit,
                            ahead: r.ahead,
                            behind: r.behind,
                        })
                        .collect();
                    // Cache hit returns no inventory update: the heavy phase did
                    // not run, so there is nothing new to memoise.
                    return Ok((loops, None));
                }
            }
        }
    }
    let worktrees = worktree_map(repo).unwrap_or_else(|e| {
        tracing::warn!(
            "git worktree list failed in {}: {}; session matching falls back to the repo path",
            repo.display(),
            error_chain(&e)
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
            tracing::warn!("unexpected line from git for-each-ref ignored: {line:?}");
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
            .map_err(|source| GitError::InvalidCommitDate {
                date: date.to_string(),
                source,
            })?
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

    // -- Write-through to the index (#13) ----------------------------------
    // A miss (or `--fresh`) just recomputed everything: persist it so the next
    // unchanged-refs scan hits the gate and skips the heavy git phase. Index
    // errors are swallowed inside `put_loops` (git is the source of truth).
    if let Some(idx) = index {
        let rows: Vec<crate::index::LoopRow> = result
            .iter()
            .map(|l| crate::index::LoopRow {
                branch: l.branch.clone(),
                head_sha: l.head_sha.clone(),
                base_sha: default_sha.clone(),
                ahead: l.ahead,
                behind: l.behind,
                last_commit: l.last_commit,
                worktree_path: l.repo_path.clone(),
            })
            .collect();
        idx.put_loops(
            &gate_hash,
            repo,
            &common_dir,
            &default,
            &default_sha,
            refs_fp,
            &rows,
        );
    }

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
    scan_indexed(roots, labels, scan_depth, opts, repo_filter, None)
}

/// Cheap per-repo git values computed once, in parallel, for the gate.
///
/// All three are derived from git subprocess calls (`default_branch_and_sha`,
/// `git_common_dir`) plus a recursive ref-tree stat (`refs_fingerprint`). They
/// are computed exactly ONCE per repo and threaded through both the gate read
/// and the write-through so the git work never runs more than once per cold repo.
struct GateInputs {
    default: String,
    default_sha: String,
    common_dir: PathBuf,
    refs_fp: i64,
    gate_hash: String,
}

/// Like [`scan`] but optionally consults a SQLite `index` for the
/// refs-fingerprint gate (#13) and to cache `--git-common-dir` during repo
/// discovery.
///
/// `rusqlite::Connection` is `Send` but `!Sync`, so a single `&Index` cannot be
/// shared across the parallel scan threads. We therefore keep the cheap SQLite
/// reads/writes on the calling thread while running ALL git work on a bounded
/// worker pool (`crate::parallel`, capped at ≈`nproc` — #16). The shape is three
/// phases:
///
/// 1. **Parallel git probes** (bounded pool): for every repo, compute the
///    cheap gate inputs (`default_branch_and_sha`, `git_common_dir`,
///    `refs_fingerprint`) ONCE. This replaces the old serial `git` fan-out that
///    serialized ~2 subprocess spawns per repo on the calling thread.
/// 2. **Sequential SQLite gate** on the calling thread: one indexed
///    `cached_loops` read per repo, using the precomputed inputs. Hits are
///    served directly; misses are deferred.
/// 3. **Parallel recompute** of the misses (bounded pool), followed by a
///    sequential write-through that REUSES the already-computed gate inputs
///    (no third re-shell of the git values).
///
/// `--fresh` runs phase 1 (so the write-through still has its inputs) but skips
/// the phase-2 gate read, treating every repo as a miss — the gate is bypassed
/// yet the index is still refreshed, matching the pre-index behaviour.
///
/// The `None` path is byte-for-byte identical to the pre-index `scan`: it skips
/// phases 1–2 entirely and recomputes every repo in parallel with no gate and
/// no write-through.
pub fn scan_indexed(
    roots: &[PathBuf],
    labels: &[(PathBuf, String)],
    scan_depth: usize,
    opts: &ScanOptions,
    repo_filter: Option<&str>,
    index: Option<&Index>,
) -> (Vec<OpenLoop>, Vec<String>, Vec<InvUpdate>) {
    let (mut repos, mut warnings) = find_repos_cached(roots, scan_depth, index);
    if let Some(filter) = repo_filter {
        let needle = filter.to_lowercase();
        repos.retain(|r| r.repo_name.to_lowercase().contains(&needle));
    }

    let mut all = Vec::new();
    let mut inventory_updates = Vec::new();

    // No index: the gate is inert. Recompute everything in parallel exactly like
    // the pre-index `scan` (the `None`-path contract — no gate inputs, no
    // write-through).
    let Some(idx) = index else {
        let misses: Vec<&RepoCandidate> = repos.iter().collect();
        recompute_misses(&misses, &[], labels, opts, None, &mut all, &mut warnings)
            .into_iter()
            .for_each(|u| inventory_updates.push(u));
        return (all, warnings, inventory_updates);
    };

    // Phase 1: compute the cheap gate inputs for every repo IN PARALLEL, on a
    // bounded worker pool (#16) rather than one thread per repo. Results are
    // positionally aligned with `repos`; `Err` is a fatal git error reported
    // once. The inputs are reused for both the gate read and the write-through.
    let gate_inputs: Vec<ScanResult<GateInputs>> = crate::parallel::try_map(
        &repos,
        crate::parallel::default_concurrency(),
        "panic while probing repository",
        |repo| compute_gate_inputs(&repo.path),
    );

    // Phase 2: sequential SQLite gate read on the calling thread (skipped under
    // `--fresh`, which still recomputes and writes through). Hits are served
    // from cache; misses (and their precomputed inputs) are deferred.
    let mut misses: Vec<&RepoCandidate> = Vec::new();
    let mut miss_inputs: Vec<GateInputs> = Vec::new();
    for (repo, inputs) in repos.iter().zip(gate_inputs) {
        let inputs = match inputs {
            Ok(i) => i,
            // Propagate the git error so the repo is reported once, not retried.
            Err(e) => {
                warnings.push(format!("{}: {}", repo.path.display(), error_chain(&e)));
                continue;
            }
        };
        let label = crate::config::label_for_repo(labels, &repo.path);
        // `--fresh` bypasses the gate read but keeps the write-through below.
        let hit = if opts.fresh {
            None
        } else {
            gate_lookup(&label, opts, idx, &inputs)
        };
        match hit {
            Some(mut loops) => all.append(&mut loops),
            None => {
                misses.push(repo);
                miss_inputs.push(inputs);
            }
        }
    }

    // Phase 3: recompute the misses in parallel and write them through using the
    // gate inputs already computed in phase 1 (no re-shell of the git values).
    recompute_misses(
        &misses,
        &miss_inputs,
        labels,
        opts,
        index,
        &mut all,
        &mut warnings,
    )
    .into_iter()
    .for_each(|u| inventory_updates.push(u));

    (all, warnings, inventory_updates)
}

/// Recomputes `misses` in parallel (heavy git phase) and, when an index is
/// present, writes each result through using the matching precomputed
/// `gate_inputs` (positionally aligned with `misses`; empty when there is no
/// index / `--fresh`, in which case write-through is skipped). Appends loops to
/// `all`, warnings to `warnings`, and returns the inventory updates.
fn recompute_misses(
    misses: &[&RepoCandidate],
    gate_inputs: &[GateInputs],
    labels: &[(PathBuf, String)],
    opts: &ScanOptions,
    index: Option<&Index>,
    all: &mut Vec<OpenLoop>,
    warnings: &mut Vec<String>,
) -> Vec<InvUpdate> {
    let mut inventory_updates = Vec::new();
    // Heavy git phase on the bounded worker pool (#16): at most `nproc` repos are
    // scanned at once instead of one thread per miss. Results stay positionally
    // aligned with `misses` so the write-through below can pair each with its
    // precomputed gate inputs by index.
    let results: Vec<ScanResult<(Vec<OpenLoop>, Option<InvUpdate>)>> = crate::parallel::try_map(
        misses,
        crate::parallel::default_concurrency(),
        "panic while scanning repository",
        |repo| {
            let label = crate::config::label_for_repo(labels, &repo.path);
            open_loops(&repo.path, &label, opts)
        },
    );

    for (i, (repo, res)) in misses.iter().zip(results).enumerate() {
        match res {
            Ok((loops, inv)) => {
                // Write the freshly computed loops through to the index using the
                // gate inputs already computed in phase 1 (no re-shell).
                if let Some(idx) = index {
                    if let Some(inputs) = gate_inputs.get(i) {
                        write_through(&repo.path, &loops, idx, inputs);
                    }
                }
                all.extend(loops);
                if let Some(update) = inv {
                    inventory_updates.push(update);
                }
            }
            Err(e) => warnings.push(format!("{}: {}", repo.path.display(), error_chain(&e))),
        }
    }
    inventory_updates
}

/// Computes the cheap per-repo gate inputs once (called in parallel, phase 1).
///
/// # Errors
///
/// Returns `Err` if the default branch or common-dir cannot be resolved.
fn compute_gate_inputs(repo: &Path) -> ScanResult<GateInputs> {
    let (default, default_sha) = default_branch_and_sha(repo)?;
    let common_dir = git_common_dir(repo)?;
    let refs_fp = refs_fingerprint(&common_dir);
    let gate_hash = inventory::common_dir_hash(&common_dir);
    Ok(GateInputs {
        default,
        default_sha,
        common_dir,
        refs_fp,
        gate_hash,
    })
}

/// Sequential SQLite gate read for one repo using its precomputed inputs.
/// `Some(loops)` is a hit the caller can use directly; `None` is a cache miss to
/// be recomputed in parallel. No git work happens here — only one indexed read.
fn gate_lookup(
    label: &str,
    opts: &ScanOptions,
    idx: &Index,
    inputs: &GateInputs,
) -> Option<Vec<OpenLoop>> {
    let rows = idx.cached_loops(&inputs.gate_hash, inputs.refs_fp, &inputs.default_sha)?;
    // Serve only if the cached rows satisfy the caller's ahead/behind need.
    if opts.need_ahead_behind && !rows.iter().all(|r| r.ahead.is_some()) {
        return None;
    }
    let repo_name = repo_name_from_common_dir(&inputs.common_dir);
    let loops = rows
        .into_iter()
        .map(|r| OpenLoop {
            root_label: label.to_string(),
            repo_name: repo_name.clone(),
            repo_path: r.worktree_path,
            branch: r.branch,
            head_sha: r.head_sha,
            last_commit: r.last_commit,
            ahead: r.ahead,
            behind: r.behind,
        })
        .collect();
    Some(loops)
}

/// Persists freshly recomputed `loops` for `repo` to the index after a miss,
/// reusing the gate inputs already computed in phase 1 (no re-shell of git).
fn write_through(repo: &Path, loops: &[OpenLoop], idx: &Index, inputs: &GateInputs) {
    let rows: Vec<crate::index::LoopRow> = loops
        .iter()
        .map(|l| crate::index::LoopRow {
            branch: l.branch.clone(),
            head_sha: l.head_sha.clone(),
            base_sha: inputs.default_sha.clone(),
            ahead: l.ahead,
            behind: l.behind,
            last_commit: l.last_commit,
            worktree_path: l.repo_path.clone(),
        })
        .collect();
    idx.put_loops(
        &inputs.gate_hash,
        repo,
        &inputs.common_dir,
        &inputs.default,
        &inputs.default_sha,
        inputs.refs_fp,
        &rows,
    );
}

/// Branch-exclusive commits relative to the default (for the distillation prompt).
///
/// # Errors
///
/// Returns `Err` if git fails.
pub fn git_log(repo: &Path, default: &str, branch: &str) -> ScanResult<String> {
    git(repo, &["log", "--oneline", &format!("{default}..{branch}")])
}

/// Diffstat of the branch against the base (for the distillation prompt).
///
/// # Errors
///
/// Returns `Err` if git fails.
pub fn diffstat(repo: &Path, default: &str, branch: &str) -> ScanResult<String> {
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
) -> ScanResult<(DateTime<Utc>, DateTime<Utc>)> {
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
        .ok_or_else(|| GitError::NoCommitDates {
            branch: branch.to_string(),
        })?;
    let max = dates
        .iter()
        .max()
        .copied()
        .ok_or_else(|| GitError::NoCommitDates {
            branch: branch.to_string(),
        })?;
    Ok((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::GitError;
    use crate::index::Index;
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
    fn default_branch_honours_origin_head_when_target_is_local() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo); // main + commit
        testutil::git(&repo, &["branch", "develop"]); // local develop exists
        testutil::git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ],
        );
        // origin/HEAD wins over main because its target resolves locally.
        assert_eq!(default_branch(&repo).unwrap(), "develop");
    }

    #[test]
    fn default_branch_falls_back_when_origin_head_target_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo); // main + commit, no local "ghost"
        testutil::git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/ghost",
            ],
        );
        // Stale origin/HEAD target → fall through to main, not an error.
        assert_eq!(default_branch(&repo).unwrap(), "main");
    }

    #[test]
    fn git_fails_with_contextual_message() {
        let tmp = tempfile::tempdir().unwrap();
        // directory is not a git repo
        let err = git(tmp.path(), &["status"]).unwrap_err();
        match &err {
            GitError::CommandFailed { repo, command, .. } => {
                assert_eq!(repo, tmp.path());
                assert_eq!(command, r#"["status"]"#);
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
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
        assert!(matches!(err, GitError::NoDefaultBranch { .. }));
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
    fn parse_worktree_porcelain_ignores_garbage_between_fields() {
        // an unrecognized line inside an entry block is skipped; the entry and
        // its trailing `branch` field still parse.
        let out = "\
worktree /home/u/app/main
HEAD aaaaaaaa
garbage line the parser does not know
branch refs/heads/main
";
        let entries = parse_worktree_porcelain(out);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].bare);
        assert!(!entries[0].prunable);
    }

    #[test]
    fn parse_worktree_porcelain_preserves_windows_style_path() {
        // Windows porcelain emits backslash paths; the parser stores the path
        // verbatim (string input, so this test runs identically on every OS).
        let out = "worktree C:\\Users\\u\\app\\main\nbranch refs/heads/main\n";
        let entries = parse_worktree_porcelain(out);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].path,
            std::path::PathBuf::from("C:\\Users\\u\\app\\main")
        );
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
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

    // -----------------------------------------------------------------------
    // Task 2: cached find_repos / dedup tests
    // -----------------------------------------------------------------------

    /// (a) find_repos_cached with a fresh in-memory index populates repos.common_dir.
    #[test]
    fn find_repos_cached_populates_index() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);

        let index = Index::open_in_memory();
        let (repos, warnings) = find_repos_cached(&[tmp.path().to_path_buf()], 4, Some(&index));

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(repos.len(), 1);

        // The index must now have an entry for this path.
        let (hash, cd) = index
            .cached_common_dir(&repo)
            .expect("index should have cached the common_dir after find_repos_cached");
        assert!(!hash.is_empty());
        assert!(
            cd.ends_with(".git"),
            "common_dir should end with .git, got: {cd:?}"
        );
    }

    /// (b) Second dedup_candidates_cached call reads from cache (SENTINEL proves
    ///     git_common_dir was NOT called again).
    ///
    /// We pre-seed the index with a fake/sentinel common_dir_hash for the repo
    /// path. If the cache is consulted, we get the sentinel back; if git is
    /// called instead, we get the real hash — different values prove the path
    /// taken.
    #[test]
    fn dedup_candidates_cached_uses_index_on_second_call() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);

        let index = Index::open_in_memory();

        // Pre-seed the index with a SENTINEL hash and a real-looking common_dir
        // so dedup can still build a RepoCandidate (repo_name_from_common_dir
        // only needs the path shape, not a real dir).
        let sentinel_hash = "sentinel_hash_no_git";
        let sentinel_cd = repo.join(".git"); // same shape as reality
        index.put_repo_common_dir(&repo, sentinel_hash, &sentinel_cd);

        // Now call dedup; it should hit the cache and return the sentinel.
        let (repos, warnings) = dedup_candidates_cached(vec![repo.clone()], Some(&index));

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(repos.len(), 1);

        // The cached entry must still be the sentinel (git was not called to overwrite it).
        let (got_hash, _) = index
            .cached_common_dir(&repo)
            .expect("index entry must still exist");
        assert_eq!(
            got_hash, sentinel_hash,
            "sentinel hash changed — git was called instead of using cache"
        );
    }

    /// (c) N worktrees of the same repo → 1 RepoCandidate on the cached path.
    #[test]
    fn dedup_cached_n_worktrees_yields_one_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("my-app");
        testutil::init_bare_worktree_container(&container);
        let dev = container.join("dev");
        testutil::add_named_worktree(&container, "dev", "dev");

        let index = Index::open_in_memory();

        // Both the container and the dev worktree point to the same common_dir.
        let candidates = vec![container.clone(), dev.clone()];
        let (repos, warnings) = dedup_candidates_cached(candidates, Some(&index));

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            repos.len(),
            1,
            "N worktrees must dedup to 1 repo, got: {repos:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3: refs-fingerprint gate (#13)
    // -----------------------------------------------------------------------

    /// Helper: indexed open_loops with ahead/behind, returning only the loops.
    fn open_loops_indexed_simple(
        repo: &std::path::Path,
        idx: Option<&Index>,
        fresh: bool,
    ) -> Vec<OpenLoop> {
        let opts = ScanOptions {
            need_ahead_behind: true,
            fresh,
            ..ScanOptions::default()
        };
        open_loops_indexed(repo, "root", &opts, idx).unwrap().0
    }

    /// (a) ZERO rev-list proof. After warming the cache with the real value
    /// (ahead=1), we overwrite the *cached* ahead/behind with an impossible
    /// sentinel (999/888) while keeping the SAME refs_fingerprint + default_sha.
    /// A second indexed scan returns the sentinel — which the live git repo can
    /// never produce — proving the gate served cached data and `rev-list` (and
    /// for-each-ref / branch --merged) were not re-run.
    #[test]
    fn warm_scan_unchanged_refs_skips_rev_list() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let index = Index::open_in_memory();

        // First scan: miss → real compute (ahead=1) + write-through.
        let first = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].ahead, Some(1));
        assert_eq!(first[0].behind, Some(0));

        // Capture the live fingerprint/hash/default_sha, then poison the cached
        // row's ahead/behind with a value the repo cannot produce.
        let common = git_common_dir(&repo).unwrap();
        let hash = crate::inventory::common_dir_hash(&common);
        let refs_fp = refs_fingerprint(&common);
        let (default, default_sha) = default_branch_and_sha(&repo).unwrap();
        let poisoned = vec![crate::index::LoopRow {
            branch: "feat/x".into(),
            head_sha: first[0].head_sha.clone(),
            base_sha: default_sha.clone(),
            ahead: Some(999),
            behind: Some(888),
            last_commit: first[0].last_commit,
            worktree_path: first[0].repo_path.clone(),
        }];
        index.put_loops(
            &hash,
            &repo,
            &common,
            &default,
            &default_sha,
            refs_fp,
            &poisoned,
        );

        // Second scan: refs unchanged → gate HIT → returns the sentinel.
        let second = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(second.len(), 1);
        assert_eq!(
            second[0].ahead,
            Some(999),
            "gate must serve cached ahead — git was re-run if this is 1"
        );
        assert_eq!(second[0].behind, Some(888));
    }

    /// (b) Advancing HEAD changes the fingerprint → recompute → fresh values.
    #[test]
    fn advancing_head_invalidates_and_recomputes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt"); // ahead=1

        let index = Index::open_in_memory();

        // Warm + poison with a sentinel under the OLD fingerprint.
        let first = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(first[0].ahead, Some(1));
        let common = git_common_dir(&repo).unwrap();
        let hash = crate::inventory::common_dir_hash(&common);
        let old_fp = refs_fingerprint(&common);
        let (default, default_sha) = default_branch_and_sha(&repo).unwrap();
        index.put_loops(
            &hash,
            &repo,
            &common,
            &default,
            &default_sha,
            old_fp,
            &[crate::index::LoopRow {
                branch: "feat/x".into(),
                head_sha: first[0].head_sha.clone(),
                base_sha: default_sha.clone(),
                ahead: Some(999),
                behind: Some(888),
                last_commit: first[0].last_commit,
                worktree_path: first[0].repo_path.clone(),
            }],
        );

        // Add a second commit on feat/x → new loose ref mtime → fingerprint bumps.
        testutil::git(&repo, &["checkout", "feat/x"]);
        std::fs::write(repo.join("x2.txt"), "x2").unwrap();
        testutil::git(&repo, &["add", "."]);
        testutil::git(&repo, &["commit", "-m", "wip more"]);
        testutil::git(&repo, &["checkout", "main"]);

        let new_fp = refs_fingerprint(&common);
        assert!(
            new_fp >= old_fp,
            "fingerprint must not go backwards: {old_fp} -> {new_fp}"
        );

        // Second scan: fingerprint differs → MISS → real recompute (ahead=2).
        let second = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(
            second[0].ahead,
            Some(2),
            "must recompute after HEAD advance"
        );
        assert_eq!(second[0].behind, Some(0));
    }

    /// (c) Changing the default-branch SHA invalidates the cache even when the
    /// branch's own refs and the gross fingerprint match the stored default_sha.
    #[test]
    fn default_sha_change_invalidates() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let index = Index::open_in_memory();
        let first = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(first[0].ahead, Some(1));

        let common = git_common_dir(&repo).unwrap();
        let hash = crate::inventory::common_dir_hash(&common);
        let refs_fp = refs_fingerprint(&common);
        let (default, _real_sha) = default_branch_and_sha(&repo).unwrap();

        // Store a poisoned row under a STALE default_sha but the live fingerprint.
        index.put_loops(
            &hash,
            &repo,
            &common,
            &default,
            "stale_default_sha_0000000000000000000000",
            refs_fp,
            &[crate::index::LoopRow {
                branch: "feat/x".into(),
                head_sha: first[0].head_sha.clone(),
                base_sha: "stale_default_sha_0000000000000000000000".into(),
                ahead: Some(999),
                behind: Some(888),
                last_commit: first[0].last_commit,
                worktree_path: first[0].repo_path.clone(),
            }],
        );

        // The live default_sha != stored stale default_sha → MISS → recompute.
        let second = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(
            second[0].ahead,
            Some(1),
            "stale default_sha must force recompute, not serve 999"
        );
    }

    /// (d) `fresh: true` bypasses the gate even when a (poisoned) cache exists.
    #[test]
    fn fresh_bypasses_the_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let index = Index::open_in_memory();
        let first = open_loops_indexed_simple(&repo, Some(&index), false);

        // Poison the cache under the live fingerprint/default_sha.
        let common = git_common_dir(&repo).unwrap();
        let hash = crate::inventory::common_dir_hash(&common);
        let refs_fp = refs_fingerprint(&common);
        let (default, default_sha) = default_branch_and_sha(&repo).unwrap();
        index.put_loops(
            &hash,
            &repo,
            &common,
            &default,
            &default_sha,
            refs_fp,
            &[crate::index::LoopRow {
                branch: "feat/x".into(),
                head_sha: first[0].head_sha.clone(),
                base_sha: default_sha.clone(),
                ahead: Some(999),
                behind: Some(888),
                last_commit: first[0].last_commit,
                worktree_path: first[0].repo_path.clone(),
            }],
        );

        // fresh=true must IGNORE the poisoned cache and recompute real values.
        let fresh = open_loops_indexed_simple(&repo, Some(&index), true);
        assert_eq!(
            fresh[0].ahead,
            Some(1),
            "fresh must recompute, not serve 999"
        );
        assert_eq!(fresh[0].behind, Some(0));
    }

    /// (e) A brand-new branch after caching → fingerprint changes → it appears.
    #[test]
    fn new_branch_after_caching_appears() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let index = Index::open_in_memory();
        let first = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(first.len(), 1);

        // Add a brand-new unmerged branch.
        testutil::add_branch_with_commit(&repo, "feat/y", "y.txt");

        // Fingerprint must have changed (new loose ref under refs/heads).
        let second = open_loops_indexed_simple(&repo, Some(&index), false);
        let mut names: Vec<_> = second.iter().map(|l| l.branch.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["feat/x".to_string(), "feat/y".to_string()]);
    }

    /// A cache HIT whose rows lack ahead/behind must NOT be served to a caller
    /// that needs them — it degrades to a recompute (correctness guard).
    #[test]
    fn hit_with_null_ahead_behind_recomputes_when_needed() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let index = Index::open_in_memory();
        // Warm the index with a LIGHT-phase scan (no ahead/behind) so the cached
        // rows have NULL ahead/behind under the live fingerprint.
        let light_opts = ScanOptions {
            need_ahead_behind: false,
            ..ScanOptions::default()
        };
        let light = open_loops_indexed(&repo, "root", &light_opts, Some(&index))
            .unwrap()
            .0;
        assert_eq!(light[0].ahead, None);

        // Now ask WITH ahead/behind: the NULL-ahead cache must not be served.
        let full = open_loops_indexed_simple(&repo, Some(&index), false);
        assert_eq!(
            full[0].ahead,
            Some(1),
            "must recompute when cached rows lack the requested ahead/behind"
        );
        assert_eq!(full[0].behind, Some(0));
    }

    /// `scan_indexed(None)` is identical to `scan`: the gate is inert.
    #[test]
    fn scan_indexed_none_matches_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let api = tmp.path().join("api-service");
        testutil::init_repo(&api);
        testutil::add_branch_with_commit(&api, "feat/api", "a.txt");
        let labels = vec![(tmp.path().to_path_buf(), "r".to_string())];
        let opts = ScanOptions {
            need_ahead_behind: true,
            ..ScanOptions::default()
        };
        let (loops, warnings, _) =
            scan_indexed(&[tmp.path().to_path_buf()], &labels, 4, &opts, None, None);
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].branch, "feat/api");
        assert_eq!(loops[0].ahead, Some(1));
    }

    /// Adding a worktree for an ALREADY-EXISTING branch (so no new loose ref is
    /// written under `refs/`) must still bump the fingerprint, because
    /// `<common_dir>/worktrees/<name>/` is created. This guards against
    /// `git worktree remove` leaving a stale `worktree_path` in the index that
    /// points at a deleted directory.
    ///
    /// We assert on ADD (not remove) because creating a new directory entry
    /// reliably bumps the parent dir mtime even on coarse filesystems, whereas
    /// remove leaves a gap the OS may or may not fill before the next read.
    #[test]
    fn adding_worktree_for_existing_branch_bumps_fingerprint() {
        let tmp = tempfile::tempdir().unwrap();
        let container = tmp.path().join("proj");
        // init_bare_worktree_container gives us: .bare/ (common_dir), a `main`
        // worktree, and one commit — a branch ref already exists.
        testutil::init_bare_worktree_container(&container);

        let common = git_common_dir(&container).unwrap();
        let fp_before = refs_fingerprint(&common);

        // Add a worktree for the existing `main` branch ref reusing `-b` on a
        // fresh name so no new ref is created (we just add a worktrees/<name>/ entry).
        // `add_named_worktree` uses `git worktree add -b <branch> <path>`, which
        // creates a brand-new branch. To avoid creating a new ref we use
        // `git worktree add --detach` on an existing commit instead.
        let wt_path = container.join("extra");
        testutil::git(
            &container,
            &[
                "worktree",
                "add",
                "--detach",
                wt_path.to_str().unwrap(),
                "HEAD",
            ],
        );

        let fp_after = refs_fingerprint(&common);
        assert!(
            fp_after > fp_before,
            "fingerprint must increase after git worktree add (before={fp_before}, after={fp_after})"
        );
    }

    /// End-to-end through `scan_indexed`: a warm scan with an index serves the
    /// poisoned cache (gate hit on the sequential path), proving the heavy phase
    /// was skipped at the scan level too.
    #[test]
    fn scan_indexed_warm_serves_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        let labels = vec![(tmp.path().to_path_buf(), "root".to_string())];
        let opts = ScanOptions {
            need_ahead_behind: true,
            ..ScanOptions::default()
        };
        let index = Index::open_in_memory();

        // Cold scan warms the index (write-through).
        let (cold, _, _) = scan_indexed(
            &[tmp.path().to_path_buf()],
            &labels,
            4,
            &opts,
            None,
            Some(&index),
        );
        assert_eq!(cold.len(), 1);
        assert_eq!(cold[0].ahead, Some(1));

        // Poison the cache under the live gate.
        let common = git_common_dir(&repo).unwrap();
        let hash = crate::inventory::common_dir_hash(&common);
        let refs_fp = refs_fingerprint(&common);
        let (default, default_sha) = default_branch_and_sha(&repo).unwrap();
        index.put_loops(
            &hash,
            &repo,
            &common,
            &default,
            &default_sha,
            refs_fp,
            &[crate::index::LoopRow {
                branch: "feat/x".into(),
                head_sha: cold[0].head_sha.clone(),
                base_sha: default_sha.clone(),
                ahead: Some(999),
                behind: Some(888),
                last_commit: cold[0].last_commit,
                worktree_path: cold[0].repo_path.clone(),
            }],
        );

        // Warm scan: gate hit on the sequential path → sentinel served.
        let (warm, warnings, _) = scan_indexed(
            &[tmp.path().to_path_buf()],
            &labels,
            4,
            &opts,
            None,
            Some(&index),
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(warm.len(), 1);
        assert_eq!(
            warm[0].ahead,
            Some(999),
            "scan_indexed warm path must serve cached loops, not re-run git"
        );
    }
}
