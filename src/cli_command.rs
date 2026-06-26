// Clap command surface shared by runtime and `build.rs` (via `include!`).
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "loops", version, about = "Recover the context of paused work")]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Filter the inventory (e.g. `loops api idle:>7d`). See ADR 0003 grammar.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub query: Vec<String>,
    /// Ignore cached inventory memo and recompute ahead/behind from git
    #[arg(long)]
    pub fresh: bool,
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
        /// Ignore cached inventory memo and recompute ahead/behind from git
        #[arg(long)]
        fresh: bool,
    },
    /// Drop a dead loop from the list (repo/branch format)
    Ignore { key: String },
    /// List git worktrees with a cleanup verdict (alias: wt)
    #[command(visible_alias = "wt")]
    Worktrees,
    /// Generate a shell completion script (bash, zsh, fish, ...)
    Completions { shell: clap_complete::Shell },
    /// Reindex the ahead/behind inventory for all repos (or those matching a query)
    Refresh {
        /// Optional query to scope the refresh (same syntax as `loops [query]`)
        query: Vec<String>,
    },
}
