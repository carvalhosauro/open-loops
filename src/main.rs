use clap::Parser;
use open_loops::cli::{self, Cli, Command};
use std::path::PathBuf;

fn main() {
    let cli = Cli::parse();
    let base = base_dir();
    let result = match cli.command {
        None => cli::run_list(&base, &cli.query.join(" "), cli.fresh, cli.path),
        Some(Command::Init { paths }) => cli::run_init(&base, &paths),
        Some(Command::Resume {
            query,
            dry_run,
            fresh,
        }) => cli::run_resume(&base, &query, dry_run, fresh),
        Some(Command::Ignore { key }) => cli::run_ignore(&base, &key),
        Some(Command::Worktrees) => cli::run_worktrees(&base),
        Some(Command::Completions { shell }) => cli::run_completions(shell),
        Some(Command::Refresh { query }) => cli::run_refresh(&base, &query.join(" ")),
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// OPEN_LOOPS_HOME overrides the default for tests and non-standard installs.
fn base_dir() -> PathBuf {
    std::env::var_os("OPEN_LOOPS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().expect("HOME not set").join(".open-loops"))
}
