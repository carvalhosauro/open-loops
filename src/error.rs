//! Typed error types for the library's public API (anyhow -> thiserror
//! migration, see docs/architecture ADRs). Domains are added incrementally;
//! `query.rs` is the first (spec 4.1a), `scanner.rs` the second (4.1b),
//! config/cache/distill/sessions/ignores/index the third (4.1c), and
//! cli/inventory/state the fourth and final (4.1d).
use std::path::PathBuf;

use thiserror::Error;

/// Formats an error followed by its whole `source()` chain, joined with
/// `": "` — the shape `anyhow` prints for `{:#}`. Print boundaries (the CLI
/// edge in `main.rs`, scan/index warnings) must use this instead of bare
/// `Display`, and messages must not interpolate their own `#[source]`,
/// otherwise the cause is either lost or printed twice.
pub fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    use std::fmt::Write;
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let _ = write!(out, ": {s}");
        src = s.source();
    }
    out
}

/// Errors from parsing and resolving queries (`src/query.rs`).
#[derive(Debug, Error)]
pub enum QueryError {
    #[error("idle needs a comparator, e.g. idle:>7d")]
    IdleMissingComparator,

    #[error("invalid duration '{0}' (expected e.g. 7d)")]
    InvalidDuration(String),

    #[error("invalid duration unit '{0}' (use m, h, d, or w)")]
    InvalidDurationUnit(String),

    #[error("expected a number, got '{0}'")]
    InvalidNumber(String),

    #[error("only one @context per query")]
    MultipleContexts,

    #[error(
        "context '{name}' filter token '{token}' cannot contain '@' (reports are ADR 0003 phase 5)"
    )]
    ContextFilterHasAt { name: String, token: String },

    #[error(
        "context '{name}' filter token '{token}' cannot contain ':' (reports are ADR 0003 phase 5)"
    )]
    ContextFilterHasColon { name: String, token: String },

    /// `:report` and `+stale` tokens: reserved for ADR 0003 phase 5, not
    /// implemented yet. The message is pre-formatted at the call site because
    /// the two cases don't share an interpolation shape.
    #[error("{0}")]
    ReservedToken(String),

    /// Wraps `Config::context_filter`'s typed error (`src/config.rs`, spec 4.1c).
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Errors from loading/saving config and resolving roots/labels (`src/config.rs`).
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unknown context '@{name}'; define [contexts.{name}] in config.toml")]
    UnknownContext { name: String },

    #[error(
        "roots {} and {} share label '{label}'; set an alias in config.toml",
        root_a.display(),
        root_b.display()
    )]
    LabelCollision {
        root_a: PathBuf,
        root_b: PathBuf,
        label: String,
    },

    #[error("nonexistent root: {}", path.display())]
    NonexistentRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("reading {}", path.display())]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("creating {}", path.display())]
    CreateDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid config.toml at {}", path.display())]
    InvalidToml {
        path: PathBuf,
        // Boxed (here and in the other *Toml variants): `toml::de::Error`
        // is large enough to push `OpenLoopsError` past clippy's
        // `result_large_err` 128-byte cap on Windows targets.
        #[source]
        source: Box<toml::de::Error>,
    },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
}

/// Errors from the distillation cache (`src/cache.rs`).
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache path has no parent directory")]
    NoParentDir,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Errors from building prompts and calling the LLM command (`src/distill.rs`).
#[derive(Debug, Error)]
pub enum DistillError {
    #[error(
        "failed to run the LLM command `{command}` — is it installed? Adjust llm_command in config.toml"
    )]
    SpawnFailed {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("stdin not available for the LLM process")]
    NoStdin,

    #[error("failed to write the prompt to the LLM stdin")]
    WriteFailed {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to wait for the LLM process")]
    WaitFailed {
        #[source]
        source: std::io::Error,
    },

    #[error("LLM command failed (`{command}`): {stderr}")]
    CommandFailed { command: String, stderr: String },
}

/// Errors from AI session sources (`src/sessions/`).
#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Errors from the ignore list (`src/ignores.rs`).
#[derive(Debug, Error)]
pub enum IgnoreError {
    #[error("invalid ignores.toml at {}", path.display())]
    InvalidToml {
        path: PathBuf,
        // Boxed: see ConfigError::InvalidToml.
        #[source]
        source: Box<toml::de::Error>,
    },

    #[error("reading {}", path.display())]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("path has no parent directory: {}", path.display())]
    NoParentDir { path: PathBuf },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
}

/// Errors from the SQLite-backed disposable index (`src/index/mod.rs`).
///
/// Every public method on `Index` treats these as non-fatal internally (it
/// warns and falls back); the type exists so the fallible internals stay
/// typed instead of `anyhow`.
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("creating index dir {}", path.display())]
    CreateDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("opening {}", path.display())]
    OpenFailed {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("applying pragmas")]
    Pragma(#[source] rusqlite::Error),

    #[error("reading user_version")]
    ReadUserVersion(#[source] rusqlite::Error),

    #[error("migrating v1→v2 (FTS heal)")]
    MigrateV1ToV2(#[source] rusqlite::Error),

    #[error("creating schema v1")]
    CreateSchemaV1(#[source] rusqlite::Error),

    #[error("integrity_check query failed")]
    IntegrityCheckQuery(#[source] rusqlite::Error),

    #[error("integrity_check: {0}")]
    IntegrityCheckFailed(String),
}

/// Errors from the ahead/behind memo store (`src/inventory.rs`).
#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("creating inventory dir {}", path.display())]
    CreateDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("serialising inventory file")]
    Serialize(#[source] serde_json::Error),

    #[error("writing tmp inventory {}", path.display())]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("renaming inventory tmp to {}", path.display())]
    RenameFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("reading inventory dir {}", path.display())]
    ReadDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Errors from `<base>/state.toml` (`src/state.rs`).
#[derive(Debug, Error)]
pub enum StateError {
    #[error("reading {}", path.display())]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid state.toml at {}", path.display())]
    InvalidStateToml {
        path: PathBuf,
        // Boxed: see ConfigError::InvalidToml.
        #[source]
        source: Box<toml::de::Error>,
    },

    #[error("invalid config.toml at {}", path.display())]
    InvalidConfigToml {
        path: PathBuf,
        // Boxed: see ConfigError::InvalidToml.
        #[source]
        source: Box<toml::de::Error>,
    },

    #[error("path has no parent directory: {}", path.display())]
    NoParentDir { path: PathBuf },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
}

/// Errors from git shell-out in `src/scanner.rs`.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("git {command} failed in {}: {stderr}", repo.display())]
    CommandFailed {
        repo: PathBuf,
        command: String,
        stderr: String,
    },

    #[error("git not found in PATH — install git")]
    NotInPath(#[source] std::io::Error),

    #[error(
        "couldn't find the default branch in {} (expected origin/HEAD, main or master)",
        repo.display()
    )]
    NoDefaultBranch { repo: PathBuf },

    #[error("invalid date from git: {date}")]
    InvalidCommitDate { date: String },

    #[error("no commit dates for {branch}")]
    NoCommitDates { branch: String },

    #[error(transparent)]
    InvalidTimestamp(#[from] chrono::ParseError),

    /// Worker panic mapped by [`crate::parallel::try_map`]; becomes a scan
    /// warning, never aborts the driver.
    #[error("{0}")]
    WorkerPanic(&'static str),
}

/// Errors raised directly at the CLI boundary (`src/cli.rs`) that don't
/// belong to any lower domain — user-input validation and "no match" outcomes.
#[derive(Debug, Error)]
pub enum CliError {
    #[error("no roots configured. Run: loops init <dir-with-your-repos>")]
    NoRootsConfigured,

    #[error("usage: loops init <dir> [<dir>...]")]
    InitMissingPaths,

    #[error("expected format: repo/branch (run `loops` to see the keys)")]
    IgnoreKeyMissingSlash,

    #[error("no loop matches '{query}'. Run `loops` to see open ones.")]
    NoLoopMatches { query: String },

    #[error("ambiguous query, candidates:\n{candidates}")]
    AmbiguousQuery { candidates: String },
}

/// Top-level error for the library's public API. Domains are added one per
/// migration task.
#[derive(Debug, Error)]
pub enum OpenLoopsError {
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Distill(#[from] DistillError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Ignore(#[from] IgnoreError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Cli(#[from] CliError),
}
