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

/// Parses a query string into a `ScanPlan`. Tokens split on whitespace only —
/// a `/` is literal inside a term.
pub fn parse(input: &str) -> Result<ScanPlan> {
    let mut plan = ScanPlan::default();
    for tok in input.split_whitespace() {
        if let Some((name, val)) = split_attr(tok) {
            match name {
                "repo" => plan.repo_filter = Some(val.to_string()),
                "branch" => plan.branch_filter = Some(val.to_string()),
                "key" => plan.key_filter = Some(val.to_string()),
                "root" => plan.root_filter = Some(val.to_string()),
                "idle" => {
                    let (cmp, rest) = split_cmp(val, true)
                        .ok_or_else(|| anyhow::anyhow!("idle needs a comparator, e.g. idle:>7d"))?;
                    plan.attr_filters.push(AttrFilter::Idle(cmp, parse_duration(rest)?));
                }
                "ahead" => {
                    let (cmp, rest) = split_cmp(val, false).expect("optional op never None");
                    plan.attr_filters
                        .push(AttrFilter::Ahead(cmp, parse_count(rest)?));
                    plan.need_ahead_behind = true;
                }
                "behind" => {
                    let (cmp, rest) = split_cmp(val, false).expect("optional op never None");
                    plan.attr_filters
                        .push(AttrFilter::Behind(cmp, parse_count(rest)?));
                    plan.need_ahead_behind = true;
                }
                _ => unreachable!("split_attr only returns known names"),
            }
        } else {
            plan.terms.push(tok.to_string());
        }
    }
    Ok(plan)
}

/// Returns `(name, value)` when `tok` is `name:value` and `name` is a known
/// attribute; otherwise `None` (the caller treats the token as a bare term).
fn split_attr(tok: &str) -> Option<(&str, &str)> {
    let (name, val) = tok.split_once(':')?;
    matches!(
        name,
        "repo" | "branch" | "key" | "root" | "idle" | "ahead" | "behind"
    )
    .then_some((name, val))
}

/// Splits a leading comparator off a value. When `require_op` and none is
/// present, returns `None`; otherwise defaults to `Cmp::Eq`.
fn split_cmp(val: &str, require_op: bool) -> Option<(Cmp, &str)> {
    for (prefix, cmp) in [(">=", Cmp::Ge), ("<=", Cmp::Le), (">", Cmp::Gt), ("<", Cmp::Lt)] {
        if let Some(rest) = val.strip_prefix(prefix) {
            return Some((cmp, rest));
        }
    }
    if require_op {
        None
    } else {
        Some((Cmp::Eq, val))
    }
}

fn parse_count(s: &str) -> Result<u32> {
    s.parse::<u32>()
        .map_err(|_| anyhow::anyhow!("expected a number, got '{s}'"))
}

/// Parses `<N><unit>` where unit is one of m/h/d/w.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let (num, unit) = s.split_at(
        s.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(s.len()),
    );
    let n: i64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration '{s}' (expected e.g. 7d)"))?;
    match unit {
        "m" => Ok(Duration::minutes(n)),
        "h" => Ok(Duration::hours(n)),
        "d" => Ok(Duration::days(n)),
        "w" => Ok(Duration::weeks(n)),
        other => bail!("invalid duration unit '{other}' (use m, h, d, or w)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bare_terms_and_substring_attrs() {
        let p = parse("api feat/login repo:billing branch:fix/ key:work/api root:~/work").unwrap();
        assert_eq!(p.terms, vec!["api".to_string(), "feat/login".to_string()]);
        assert_eq!(p.repo_filter.as_deref(), Some("billing"));
        assert_eq!(p.branch_filter.as_deref(), Some("fix/"));
        assert_eq!(p.key_filter.as_deref(), Some("work/api"));
        assert_eq!(p.root_filter.as_deref(), Some("~/work"));
        assert!(!p.need_ahead_behind);
    }

    #[test]
    fn unknown_attr_prefix_is_a_bare_term() {
        // a stray colon on an unknown name is not an error; it is a term
        let p = parse("foo:bar").unwrap();
        assert_eq!(p.terms, vec!["foo:bar".to_string()]);
    }

    #[test]
    fn parse_numeric_and_duration_attrs() {
        let p = parse("idle:>7d behind:>0 ahead:0").unwrap();
        assert_eq!(
            p.attr_filters,
            vec![
                AttrFilter::Idle(Cmp::Gt, Duration::days(7)),
                AttrFilter::Behind(Cmp::Gt, 0),
                AttrFilter::Ahead(Cmp::Eq, 0),
            ]
        );
        // ahead/behind attrs force the heavy phase
        assert!(p.need_ahead_behind);
    }

    #[test]
    fn idle_without_operator_is_an_error() {
        let err = parse("idle:7d").unwrap_err().to_string();
        assert!(err.contains("idle"), "got: {err}");
    }

    #[test]
    fn bad_duration_unit_is_an_error() {
        let err = parse("idle:>7y").unwrap_err().to_string();
        assert!(err.contains("duration"), "got: {err}");
    }

    #[test]
    fn duration_units_minutes_hours_days_weeks() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("6h").unwrap(), Duration::hours(6));
        assert_eq!(parse_duration("2d").unwrap(), Duration::days(2));
        assert_eq!(parse_duration("3w").unwrap(), Duration::weeks(3));
    }
}
