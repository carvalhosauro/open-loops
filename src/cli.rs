//! Command definitions and module orchestration.
#[path = "cli_command.rs"]
mod cli_command;
pub use cli_command::{Cli, Command};

use crate::config::Store;
use crate::distill::Confidence;
use crate::ignores::Ignores;
use crate::inventory::InventoryStore;
use crate::scanner::{self, OpenLoop, ScanOptions};
use crate::state::State;
use crate::{cache, distill, output, sessions, worktrees};
use anyhow::{bail, ensure, Result};
use sessions::{SessionExcerpt, SessionSource};
use std::path::{Path, PathBuf};

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
fn scan_with_inventory(
    base: &Path,
    cfg: &crate::config::Config,
    plan: &crate::query::ScanPlan,
    roots: &[PathBuf],
    labels: &[(PathBuf, String)],
    need_ahead_behind: bool,
    fresh: bool,
) -> Result<ScanResult> {
    let inv_store = InventoryStore::new(base);
    let opts = ScanOptions {
        need_ahead_behind,
        fresh,
        inventory_dir: Some(inv_store.dir.clone()),
        inventory_ttl_secs: cfg.inventory_ttl_secs,
    };
    let (found, warnings, inv_updates) = scanner::scan(
        roots,
        labels,
        cfg.scan_depth,
        &opts,
        plan.repo_filters.first().map(|s| s.as_str()),
    );
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok((found, inv_updates))
}

fn resolve_loop(base: &Path, query: &str, fresh: bool) -> Result<OpenLoop> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let mut plan = resolve_plan_persisting(base, &cfg, query)?;
    plan.include_ignored = true; // resume can target an ignored loop by key
    let labels = cfg.resolve_labels()?;
    let roots = cfg.resolve_scan_roots(&plan)?;
    let inv_store = InventoryStore::new(base);
    let (found, inv_updates) = scan_with_inventory(
        base,
        &cfg,
        &plan,
        &roots,
        &labels,
        plan.need_ahead_behind,
        fresh,
    )?;
    write_inventory(&inv_store, inv_updates);
    let now = chrono::Utc::now();
    let matches: Vec<&OpenLoop> = found
        .iter()
        .filter(|l| {
            let key = l.key();
            plan.matches(
                &crate::query::Candidate {
                    repo_name: &l.repo_name,
                    branch: &l.branch,
                    key: &key,
                    last_commit: l.last_commit,
                    ahead: l.ahead,
                    behind: l.behind,
                    ignored: false,
                },
                now,
            )
        })
        .collect();
    match matches.len() {
        0 => bail!("no loop matches '{query}'. Run `loops` to see open ones."),
        1 => Ok(matches[0].clone()),
        _ => bail!(
            "ambiguous query, candidates:\n{}",
            matches
                .iter()
                .map(|l| format!("  {}", l.key()))
                .collect::<Vec<_>>()
                .join("\n")
        ),
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
    let excerpts = source.excerpts(
        &lp.repo_path,
        &lp.branch,
        window,
        cfg.max_sessions,
        cfg.max_session_kb,
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

pub fn run_list(base: &Path, query: &str, fresh: bool) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let mut plan = resolve_plan_persisting(base, &cfg, query)?;
    plan.need_ahead_behind = true; // table always renders AHEAD/BEHIND columns
    let labels = cfg.resolve_labels()?;
    let roots = cfg.resolve_scan_roots(&plan)?;
    progress("scanning git repositories…");
    let inv_store = InventoryStore::new(base);
    let (found, inv_updates) = scan_with_inventory(
        base, &cfg, &plan, &roots, &labels, true, // need_ahead_behind
        fresh,
    )?;
    write_inventory(&inv_store, inv_updates);
    let ignores = Ignores::load(base)?;
    let now = chrono::Utc::now();
    let visible: Vec<OpenLoop> = found
        .into_iter()
        .filter(|l| {
            let key = l.key();
            plan.matches(
                &crate::query::Candidate {
                    repo_name: &l.repo_name,
                    branch: &l.branch,
                    key: &key,
                    last_commit: l.last_commit,
                    ahead: l.ahead,
                    behind: l.behind,
                    ignored: ignores.contains(&key),
                },
                now,
            )
        })
        .collect();
    if visible.is_empty() && !query.trim().is_empty() {
        eprintln!("No loops match: {query}");
        eprintln!("(hint: run `loops` to list all)");
    }
    print!("{}", output::render_table(&visible, now));
    Ok(())
}

pub fn run_init(base: &Path, paths: &[PathBuf]) -> Result<()> {
    ensure!(!paths.is_empty(), "usage: loops init <dir> [<dir>...]");
    let store = Store::new(base.to_path_buf());
    let cfg = store.add_roots(paths)?;
    println!("roots registered:");
    for r in &cfg.roots {
        println!("  {}", r.display());
    }
    println!("\nconfig at {}", store.config_path().display());
    Ok(())
}

pub fn run_ignore(base: &Path, key: &str) -> Result<()> {
    ensure!(
        key.contains('/'),
        "expected format: repo/branch (run `loops` to see the keys)"
    );
    let mut ignores = Ignores::load(base)?;
    ignores.add(key)?;
    println!("ignored: {key}");
    Ok(())
}

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
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let plan = resolve_plan_persisting(base, &cfg, query)?;
    let labels = cfg.resolve_labels()?;
    let roots = cfg.resolve_scan_roots(&plan)?;
    progress("scanning git repositories…");
    let inv_store = InventoryStore::new(base);
    let (found, inv_updates) = scan_with_inventory(
        base, &cfg, &plan, &roots, &labels, true, // need_ahead_behind
        true, // fresh: refresh always recomputes — ignores any cached memo
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
                plan.matches(
                    &crate::query::Candidate {
                        repo_name: &l.repo_name,
                        branch: &l.branch,
                        key: &key,
                        last_commit: l.last_commit,
                        ahead: l.ahead,
                        behind: l.behind,
                        ignored: ignores.contains(&key),
                    },
                    now,
                )
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
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    progress("scanning git worktrees…");
    let (wts, warnings) = worktrees::scan_worktrees(&cfg.roots, cfg.scan_depth);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    print!("{}", output::render_worktrees(&wts, chrono::Utc::now()));
    Ok(())
}
