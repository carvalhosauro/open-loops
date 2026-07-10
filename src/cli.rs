//! Command definitions and module orchestration.
#[path = "cli_command.rs"]
mod cli_command;
pub use cli_command::{Cli, Command};

use crate::config::Store;
use crate::distill::Confidence;
use crate::error::{CliError, OpenLoopsError};
use crate::ignores::Ignores;
use crate::index::Index;
use crate::inventory::InventoryStore;
use crate::scanner::{self, OpenLoop, ScanOptions};
use crate::state::State;
use crate::{cache, distill, output, sessions, worktrees};
use sessions::SessionExcerpt;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, OpenLoopsError>;

struct ResumeEvidence {
    default_branch: String,
    commits: String,
    diffstat: String,
    excerpts: Vec<SessionExcerpt>,
    confidence: Confidence,
}

type ScanResult = (
    Vec<OpenLoop>,
    Vec<(String, crate::inventory::InventoryFile)>,
);

fn progress(msg: &str) {
    eprintln!("{msg}");
}

/// Loads config and enforces the invariant that at least one root is registered.
/// Every command that scans repos shares this preamble; centralizing it keeps the
/// "no roots" guidance identical across all entry points.
fn load_cfg_with_roots(base: &Path) -> Result<(Store, crate::config::Config)> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    if cfg.roots.is_empty() {
        return Err(CliError::NoRootsConfigured.into());
    }
    Ok((store, cfg))
}

/// Builds the query `Candidate` view of a loop. `key` is borrowed by the returned
/// Candidate, so the caller must own it (via `l.key()`) for at least as long as
/// the Candidate — hence it is passed in rather than computed here.
fn candidate_of<'a>(l: &'a OpenLoop, key: &'a str, ignored: bool) -> crate::query::Candidate<'a> {
    crate::query::Candidate {
        repo_name: &l.repo_name,
        branch: &l.branch,
        key,
        last_commit: l.last_commit,
        ahead: l.ahead,
        behind: l.behind,
        ignored,
    }
}

fn resolve_plan_persisting(
    base: &Path,
    cfg: &crate::config::Config,
    query: &str,
) -> Result<crate::query::ScanPlan> {
    let mut state = State::load(base)?;
    let plan = crate::query::resolve_plan(
        query,
        cfg,
        &crate::query::ResolveOptions {
            current_context: state.current_context(),
        },
    )?;

    match crate::query::context_persistence_from_query(query)? {
        crate::query::ContextPersistence::Set(name) => {
            state.set_current_context(Some(name))?;
        }
        crate::query::ContextPersistence::Clear => {
            state.set_current_context(None)?;
        }
        crate::query::ContextPersistence::Unchanged => {}
    }

    Ok(plan)
}

/// Writes inventory updates produced by a scan to disk.
fn write_inventory(
    inv_store: &InventoryStore,
    updates: Vec<(String, crate::inventory::InventoryFile)>,
) {
    for (hash, file) in updates {
        if let Err(e) = inv_store.save(&hash, &file) {
            eprintln!(
                "warning: failed to write inventory for {}: {e:#}",
                file.repo_path.display()
            );
        }
    }
}

/// Scans with inventory write-through and returns the found loops and raw inventory
/// updates. Prints scan warnings to stderr. The caller may filter `inv_updates`
/// before writing (as in `run_refresh`) or write them directly (as in `resolve_loop`
/// and `run_list`).
#[allow(clippy::too_many_arguments)]
fn scan_with_inventory(
    base: &Path,
    cfg: &crate::config::Config,
    plan: &crate::query::ScanPlan,
    roots: &[PathBuf],
    labels: &[(PathBuf, String)],
    need_ahead_behind: bool,
    fresh: bool,
    index: Option<&Index>,
) -> Result<ScanResult> {
    let inv_store = InventoryStore::new(base);
    let opts = ScanOptions {
        need_ahead_behind,
        fresh,
        inventory_dir: Some(inv_store.dir.clone()),
        inventory_ttl_secs: cfg.inventory_ttl_secs,
    };
    // Only the first repo filter is pushed into the scan as a directory-walk
    // narrowing hint; any remaining repo filters (and all other filters) are
    // applied afterward in memory by ScanPlan::matches, so correctness does not
    // depend on the scan honoring more than one.
    let (found, warnings, inv_updates) = scanner::scan_indexed(
        roots,
        labels,
        cfg.scan_depth,
        &opts,
        plan.repo_filters.first().map(|s| s.as_str()),
        index,
    );
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok((found, inv_updates))
}

fn resolve_loop(base: &Path, query: &str, fresh: bool) -> Result<OpenLoop> {
    let (_store, cfg) = load_cfg_with_roots(base)?;
    let mut plan = resolve_plan_persisting(base, &cfg, query)?;
    plan.include_ignored = true; // resume can target an ignored loop by key
    let labels = cfg.resolve_labels()?;
    let roots = cfg.resolve_scan_roots(&plan)?;
    let inv_store = InventoryStore::new(base);
    // Open the disposable SQLite index once per command. `Index::open` is
    // tolerant: a corrupt/unopenable db self-heals (rebuild or in-memory
    // fallback), so this never aborts the command.
    let index = Index::open(base);
    let (found, inv_updates) = scan_with_inventory(
        base,
        &cfg,
        &plan,
        &roots,
        &labels,
        plan.need_ahead_behind,
        fresh,
        Some(&index),
    )?;
    write_inventory(&inv_store, inv_updates);
    let now = chrono::Utc::now();
    let matches: Vec<&OpenLoop> = found
        .iter()
        .filter(|l| {
            let key = l.key();
            plan.matches(&candidate_of(l, &key, false), now)
        })
        .collect();
    match matches.len() {
        0 => Err(CliError::NoLoopMatches {
            query: query.to_string(),
        }
        .into()),
        1 => Ok(matches[0].clone()),
        _ => Err(CliError::AmbiguousQuery {
            candidates: matches
                .iter()
                .map(|l| format!("  {}", l.key()))
                .collect::<Vec<_>>()
                .join("\n"),
        }
        .into()),
    }
}

fn gather_resume_evidence(base: &Path, lp: &OpenLoop) -> Result<ResumeEvidence> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    let default_branch = scanner::default_branch(&lp.repo_path)?;
    let commits = scanner::git_log(&lp.repo_path, &default_branch, &lp.branch)?;
    let diffstat = scanner::diffstat(&lp.repo_path, &default_branch, &lp.branch)?;
    let window = scanner::commit_window(&lp.repo_path, &default_branch, &lp.branch)?;
    progress("matching AI sessions…");
    let source = sessions::claude_code::ClaudeCode {
        projects_dir: cfg.sessions_dir.clone(),
    };
    // Open the disposable index and run the FTS-accelerated mention probe (#14).
    // `Index::open` is tolerant, and `excerpts_indexed` degrades to the in-memory
    // file probe on any index error — so this never aborts resume.
    let index = Index::open(base);
    let excerpts = source.excerpts_indexed(
        &lp.repo_path,
        &lp.branch,
        window,
        cfg.max_sessions,
        cfg.max_session_kb,
        Some(&index),
    )?;
    let confidence = distill::compute_confidence(&excerpts);
    Ok(ResumeEvidence {
        default_branch,
        commits,
        diffstat,
        excerpts,
        confidence,
    })
}

/// Backs `loops [query]`: persists any `@context` switch, scans the matching
/// repos, and renders the inventory table to stdout.
pub fn run_list(base: &Path, query: &str, fresh: bool, show_path: bool) -> Result<()> {
    let (_store, cfg) = load_cfg_with_roots(base)?;
    let mut plan = resolve_plan_persisting(base, &cfg, query)?;
    plan.need_ahead_behind = true; // table always renders AHEAD/BEHIND columns
    let labels = cfg.resolve_labels()?;
    let roots = cfg.resolve_scan_roots(&plan)?;
    progress("scanning git repositories…");
    let inv_store = InventoryStore::new(base);
    let index = Index::open(base);
    let need_ahead_behind = true;
    let (found, inv_updates) = scan_with_inventory(
        base,
        &cfg,
        &plan,
        &roots,
        &labels,
        need_ahead_behind,
        fresh,
        Some(&index),
    )?;
    write_inventory(&inv_store, inv_updates);
    let ignores = Ignores::load(base)?;
    let now = chrono::Utc::now();
    let visible: Vec<OpenLoop> = found
        .into_iter()
        .filter(|l| {
            let key = l.key();
            plan.matches(&candidate_of(l, &key, ignores.contains(&key)), now)
        })
        .collect();
    if visible.is_empty() && !query.trim().is_empty() {
        eprintln!("No loops match: {query}");
        eprintln!("(hint: run `loops` to list all)");
    }
    print!("{}", output::render_table(&visible, now, show_path));
    Ok(())
}

/// Backs `loops init`: registers repository roots in the config so later scans
/// know where to look.
pub fn run_init(base: &Path, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Err(CliError::InitMissingPaths.into());
    }
    let store = Store::new(base.to_path_buf());
    let cfg = store.add_roots(paths)?;
    println!("roots registered:");
    for r in &cfg.roots {
        println!("  {}", r.display());
    }
    println!("\nconfig at {}", store.config_path().display());
    Ok(())
}

/// Backs `loops ignore`: persists a `repo/branch` key to the ignore list so it
/// no longer surfaces as an open loop.
pub fn run_ignore(base: &Path, key: &str) -> Result<()> {
    if !key.contains('/') {
        return Err(CliError::IgnoreKeyMissingSlash.into());
    }
    let mut ignores = Ignores::load(base)?;
    ignores.add(key)?;
    println!("ignored: {key}");
    Ok(())
}

/// Backs `loops resume`: resolves the single matching loop and distills its
/// context via the LLM, serving from cache when possible. `dry_run` prints the
/// matched evidence without calling the LLM.
pub fn run_resume(base: &Path, query: &str, dry_run: bool, fresh: bool) -> Result<()> {
    progress("scanning git…");
    let lp = resolve_loop(base, query, fresh)?;

    if dry_run {
        let evidence = gather_resume_evidence(base, &lp)?;
        let doc = distill::format_dry_run(
            &lp,
            &evidence.default_branch,
            &evidence.commits,
            &evidence.diffstat,
            &evidence.excerpts,
            evidence.confidence,
        );
        print!("{doc}");
        return Ok(());
    }

    let cache = cache::Cache::new(base);
    if let Some(hit) = cache.get(&lp) {
        println!("{hit}");
        return Ok(());
    }

    let evidence = gather_resume_evidence(base, &lp)?;
    let prompt = distill::build_prompt(
        &lp,
        &evidence.default_branch,
        &evidence.commits,
        &evidence.diffstat,
        &evidence.excerpts,
    );
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    progress("distilling…");
    let answer = distill::run_llm(&cfg.llm_command, &prompt)?;
    let doc = distill::with_sources(&answer, &lp, &evidence.excerpts, evidence.confidence);
    cache.put(&lp, &doc)?;
    println!("{doc}");
    Ok(())
}

/// Reindexes ahead/behind for all repos matching `query` (or all repos when
/// `query` is empty), writes the updated inventory, and prunes orphan files.
pub fn run_refresh(base: &Path, query: &str) -> Result<()> {
    let (_store, cfg) = load_cfg_with_roots(base)?;
    let plan = resolve_plan_persisting(base, &cfg, query)?;
    let labels = cfg.resolve_labels()?;
    let roots = cfg.resolve_scan_roots(&plan)?;
    progress("scanning git repositories…");
    let inv_store = InventoryStore::new(base);
    // `fresh: true` bypasses the gate but still writes through, so the index's
    // `loops`/`repos` rows are rebuilt for the scoped repos on this scan.
    let index = Index::open(base);
    let need_ahead_behind = true;
    let fresh = true; // refresh always recomputes — ignores any cached memo
    let (found, inv_updates) = scan_with_inventory(
        base,
        &cfg,
        &plan,
        &roots,
        &labels,
        need_ahead_behind,
        fresh,
        Some(&index),
    )?;

    // Scope the reindex to the loops the query would actually list. `repo:`/`root:`
    // are already pushed down into the scan, but bare terms and branch:/key:/idle:/
    // ahead:/behind: filters only narrow in memory — so apply them here too, or
    // `loops refresh beta` would rewrite every repo instead of the matching ones.
    // A repo is reindexed when at least one of its loops matches; we correlate a
    // loop to its inventory file by HEAD sha (globally unique, and worktree-safe:
    // two worktrees share one common-dir file but keep distinct branch HEADs).
    let scoped = if has_in_memory_filter(&plan) {
        let ignores = Ignores::load(base)?;
        let now = chrono::Utc::now();
        let matching: std::collections::HashSet<&str> = found
            .iter()
            .filter(|l| {
                let key = l.key();
                plan.matches(&candidate_of(l, &key, ignores.contains(&key)), now)
            })
            .map(|l| l.head_sha.as_str())
            .collect();
        inv_updates
            .into_iter()
            .filter(|(_, file)| {
                file.loops
                    .iter()
                    .any(|m| matching.contains(m.head_sha.as_str()))
            })
            .collect()
    } else {
        inv_updates
    };

    let n = scoped.len();
    write_inventory(&inv_store, scoped);
    inv_store.prune_orphans()?;
    // Reclaim index rows for repos gone from disk (same orphan semantics as the
    // inventory prune above). Tolerant: a failure here never aborts refresh.
    index.prune_missing_repos();
    let noun = if n == 1 { "repo" } else { "repos" };
    eprintln!("refreshed {n} {noun}");
    Ok(())
}

/// True when the plan carries filters that `scanner::scan` cannot push down to
/// repo scope and that are only applied in memory (bare terms, branch/key
/// substrings, attribute comparisons). When false, the `repo:`/`root:` push-down
/// has already scoped the scan and every scanned repo is in scope.
fn has_in_memory_filter(plan: &crate::query::ScanPlan) -> bool {
    !plan.terms.is_empty()
        || !plan.branch_filters.is_empty()
        || !plan.key_filters.is_empty()
        || !plan.attr_filters.is_empty()
}

pub fn run_completions(shell: clap_complete::Shell) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "loops", &mut std::io::stdout());
    Ok(())
}

pub fn run_worktrees(base: &Path) -> Result<()> {
    let (_store, cfg) = load_cfg_with_roots(base)?;
    progress("scanning git worktrees…");
    let (wts, warnings) = worktrees::scan_worktrees(&cfg.roots, cfg.scan_depth);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    print!("{}", output::render_worktrees(&wts, chrono::Utc::now()));
    Ok(())
}
