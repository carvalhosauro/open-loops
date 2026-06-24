//! Definição dos comandos e orquestração dos módulos.
use crate::config::Store;
use crate::ignores::Ignores;
use crate::scanner::{self, OpenLoop};
use crate::{cache, distill, output, sessions, worktrees};
use anyhow::{bail, ensure, Result};
use clap::{Parser, Subcommand};
use sessions::SessionSource;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "loops",
    version,
    about = "Recupere o contexto de trabalhos pausados"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Registra raízes de repositórios (ex.: loops init ~/repo)
    Init { paths: Vec<PathBuf> },
    /// Destila o contexto de um loop: por quê, feito, falta, próximo passo
    Resume { query: String },
    /// Descarta um loop morto da lista (formato repo/branch)
    Ignore { key: String },
    /// List git worktrees with a cleanup verdict (alias: wt)
    #[command(visible_alias = "wt")]
    Worktrees,
    /// Generate a shell completion script (bash, zsh, fish, ...)
    Completions { shell: clap_complete::Shell },
}

pub fn run_list(base: &Path) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "nenhuma raiz configurada. Rode: loops init <dir-com-seus-repos>"
    );
    let (found, warnings) = scanner::scan(&cfg.roots);
    for w in &warnings {
        eprintln!("aviso: {w}");
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
    ensure!(!paths.is_empty(), "uso: loops init <dir> [<dir>...]");
    let store = Store::new(base.to_path_buf());
    let cfg = store.add_roots(paths)?;
    println!("raízes registradas:");
    for r in &cfg.roots {
        println!("  {}", r.display());
    }
    println!("\nconfig em {}", store.config_path().display());
    Ok(())
}

pub fn run_ignore(base: &Path, key: &str) -> Result<()> {
    ensure!(
        key.contains('/'),
        "formato esperado: repo/branch (rode `loops` para ver as chaves)"
    );
    let mut ignores = Ignores::load(base)?;
    ignores.add(key)?;
    println!("ignorado: {key}");
    Ok(())
}

pub fn run_resume(base: &Path, query: &str) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "nenhuma raiz configurada. Rode: loops init <dir-com-seus-repos>"
    );
    let (found, warnings) = scanner::scan(&cfg.roots);
    for w in &warnings {
        eprintln!("aviso: {w}");
    }
    // resolução fuzzy: substring case-insensitive sobre a chave repo/branch
    let q = query.to_lowercase();
    let matches: Vec<&OpenLoop> = found
        .iter()
        .filter(|l| l.key().to_lowercase().contains(&q))
        .collect();
    let lp = match matches.len() {
        0 => bail!("nenhum loop bate com '{query}'. Rode `loops` para ver os abertos."),
        1 => matches[0],
        _ => bail!(
            "query ambígua, candidatos:\n{}",
            matches
                .iter()
                .map(|l| format!("  {}", l.key()))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };

    let cache = cache::Cache::new(base);
    if let Some(hit) = cache.get(lp) {
        println!("{hit}");
        return Ok(());
    }

    let default = scanner::default_branch(&lp.repo_path)?;
    let commits = scanner::git_log(&lp.repo_path, &default, &lp.branch)?;
    let diffstat = scanner::diffstat(&lp.repo_path, &default, &lp.branch)?;
    let window = scanner::commit_window(&lp.repo_path, &default, &lp.branch)?;
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
    if excerpts.is_empty() {
        eprintln!("aviso: nenhuma sessão de IA encontrada — confiança baixa, contexto só do git");
    }
    let prompt = distill::build_prompt(lp, &default, &commits, &diffstat, &excerpts);
    let answer = distill::run_llm(&cfg.llm_command, &prompt)?;
    let doc = distill::with_sources(&answer, lp, &excerpts);
    cache.put(lp, &doc)?;
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
