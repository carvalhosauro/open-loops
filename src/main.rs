use clap::Parser;
use open_loops::cli::{self, Cli, Command};
use std::path::PathBuf;

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
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
        eprintln!("error: {}", open_loops::error::error_chain(&e));
        std::process::exit(1);
    }
}

/// Configures the stderr tracing subscriber that renders progress and warnings.
///
/// Precedence: an explicit `RUST_LOG` always wins (standard Rust ecosystem
/// composition). Otherwise `--verbose` raises the crate to `debug` (which today
/// surfaces the `info!`-level phase progress); with neither, the default is
/// `warn`, so unadorned runs still surface warnings and keep stdout clean for
/// piping. A malformed `RUST_LOG` falls back to `warn` rather than silencing
/// warnings.
fn init_tracing(verbose: bool) {
    use std::io::IsTerminal;
    use tracing_subscriber::EnvFilter;
    let filter = match std::env::var("RUST_LOG") {
        // try_new so an unparseable RUST_LOG degrades to `warn` instead of an
        // empty (level-OFF) filter that would hide warnings too.
        Ok(v) if !v.is_empty() => {
            EnvFilter::try_new(&v).unwrap_or_else(|_| EnvFilter::new("warn"))
        }
        _ if verbose => EnvFilter::new("open_loops=debug"),
        _ => EnvFilter::new("warn"),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        // Colour only an interactive stderr; piped/captured output stays plain.
        .with_ansi(std::io::stderr().is_terminal())
        .without_time()
        .with_target(false)
        .init();
}

/// OPEN_LOOPS_HOME overrides the default for tests and non-standard installs.
fn base_dir() -> PathBuf {
    std::env::var_os("OPEN_LOOPS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().expect("HOME not set").join(".open-loops"))
}
