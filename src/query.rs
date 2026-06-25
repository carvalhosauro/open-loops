//! Query parsing and in-memory evaluation. Pure: no git, no I/O.
//! Grammar lives in ADR 0003. This module turns a query string into a
//! `ScanPlan` and decides whether a candidate loop matches it.
use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};

/// Numeric/temporal comparator for `idle`/`ahead`/`behind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
}

impl Cmp {
    fn test_i64(self, lhs: i64, rhs: i64) -> bool {
        match self {
            Cmp::Gt => lhs > rhs,
            Cmp::Lt => lhs < rhs,
            Cmp::Ge => lhs >= rhs,
            Cmp::Le => lhs <= rhs,
            Cmp::Eq => lhs == rhs,
        }
    }
}

/// An attribute filter evaluated in memory after the scan.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrFilter {
    Idle(Cmp, Duration),
    Ahead(Cmp, u32),
    Behind(Cmp, u32),
}

/// The parsed query, derived before any heavy I/O.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanPlan {
    /// Bare terms; each must substring-match repo, branch, or key (AND across terms).
    pub terms: Vec<String>,
    pub repo_filter: Option<String>,
    pub branch_filter: Option<String>,
    pub key_filter: Option<String>,
    /// Raw `root:` value; resolved against configured roots in Phase 2 push-down.
    pub root_filter: Option<String>,
    pub attr_filters: Vec<AttrFilter>,
    /// `+ignored` includes dismissed loops; default hides them.
    pub include_ignored: bool,
    /// True when AHEAD/BEHIND must be available (query references them, or the
    /// caller renders the columns — the caller ORs in the render need).
    pub need_ahead_behind: bool,
}

/// A loop as seen by the evaluator. Borrowed to keep `matches` allocation-free.
pub struct Candidate<'a> {
    pub repo_name: &'a str,
    pub branch: &'a str,
    pub key: &'a str,
    pub last_commit: DateTime<Utc>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub ignored: bool,
}
