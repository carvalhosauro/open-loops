//! Typed error types for the library's public API (anyhow -> thiserror
//! migration, see docs/architecture ADRs). Domains are added incrementally;
//! `query.rs` is the first (spec 4.1a).
use thiserror::Error;

/// Errors from parsing and resolving queries (`src/query.rs`).
#[derive(Debug, Error, PartialEq, Eq)]
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

    /// Wraps `Config::context_filter`'s "unknown context" message verbatim.
    /// `src/config.rs` still returns `anyhow::Result` (migrated in a later
    /// WAVE 4 task); this keeps the error text unchanged until then.
    #[error("{0}")]
    UnknownContext(String),
}

/// Top-level error for the library's public API. Domains are added one per
/// migration task; only `Query` exists so far.
#[derive(Debug, Error)]
pub enum OpenLoopsError {
    #[error(transparent)]
    Query(#[from] QueryError),
    // Later WAVE 4 tasks add: Git, Config, Cache, Distill, Session, Ignore,
    // Index, Io.
}
