use clap::Parser;
use open_loops::cli::{self, Cli, Command};
use std::path::PathBuf;

fn main() {
    let cli = Cli::parse();
    let base = base_dir();
    let result = match cli.command {
        None => cli::run_list(&base),
        Some(Command::Init { paths }) => cli::run_init(&base, &paths),
        Some(Command::Resume { query }) => cli::run_resume(&base, &query),
        Some(Command::Ignore { key }) => cli::run_ignore(&base, &key),
    };
    if let Err(e) = result {
        eprintln!("erro: {e:#}");
        std::process::exit(1);
    }
}

/// OPEN_LOOPS_HOME serve para testes e instalações não-padrão.
fn base_dir() -> PathBuf {
    std::env::var_os("OPEN_LOOPS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("HOME não definido")
                .join(".open-loops")
        })
}
