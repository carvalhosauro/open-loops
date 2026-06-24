//! Command definitions and module orchestration.
use crate::config::Store;
use crate::distill::Confidence;
use crate::ignores::Ignores;
use crate::scanner::{self, OpenLoop};
use crate::{cache, distill, output, sessions, worktrees};
use anyhow::{bail, ensure, Result};
use clap::{Parser, Subcommand};
use sessions::{SessionExcerpt, SessionSource};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "loops", version, about = "Recover the context of paused work")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Register repository roots (e.g. loops init ~/repo)
    Init { paths: Vec<PathBuf> },
    /// Distill a loop's context: why, done, remaining, next step
    Resume {
        query: String,
        /// Show matched git commits and AI sessions without calling the LLM
        #[arg(long)]
        dry_run: bool,
    },
    /// Drop a dead loop from the list (repo/branch format)
    Ignore { key: String },
    /// List git worktrees with a cleanup verdict (alias: wt)
    #[command(visible_alias = "wt")]
    Worktrees,
    /// Generate a shell completion script (bash, zsh, fish, ...)
    Completions { shell: clap_complete::Shell },
}

struct ResumeEvidence {
    default_branch: String,
    commits: String,
    diffstat: String,
    excerpts: Vec<SessionExcerpt>,
    confidence: Confidence,
}

fn resolve_loop(base: &Path, query: &str) -> Result<OpenLoop> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let (found, warnings) = scanner::scan(&cfg.roots);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let q = query.to_lowercase();
    let matches: Vec<&OpenLoop> = found
        .iter()
        .filter(|l| l.key().to_lowercase().contains(&q))
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

pub fn run_list(base: &Path) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let (found, warnings) = scanner::scan(&cfg.roots);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let ignores = Ignores::load(base)?;
    let visible: Vec<OpenLoop> = found
        .into_iter()
        .filter(|l| !ignores.contains(&l.key()))
        .collect();
    print!("{}", output::render_table(&visible, chrono::Utc::now()));
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

pub fn run_resume(base: &Path, query: &str, dry_run: bool) -> Result<()> {
    let lp = resolve_loop(base, query)?;

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
    let answer = distill::run_llm(&cfg.llm_command, &prompt)?;
    let doc = distill::with_sources(&answer, &lp, &evidence.excerpts, evidence.confidence);
    cache.put(&lp, &doc)?;
    println!("{doc}");
    Ok(())
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
    let (wts, warnings) = worktrees::scan_worktrees(&cfg.roots);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    print!("{}", output::render_worktrees(&wts, chrono::Utc::now()));
    Ok(())
}
