//! Query parsing and in-memory evaluation. Pure: no git, no I/O.
//! Grammar lives in ADR 0003. This module turns a query string into a
//! `ScanPlan` and decides whether a candidate loop matches it.
use crate::config::Config;
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
    pub repo_filters: Vec<String>,
    pub branch_filters: Vec<String>,
    pub key_filters: Vec<String>,
    /// Raw `root:` values; resolved against configured roots in Phase 2 push-down.
    pub root_filters: Vec<String>,
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
        match tok {
            "+ignored" => {
                plan.include_ignored = true;
                continue;
            }
            "-ignored" => {
                plan.include_ignored = false;
                continue;
            }
            "+stale" => bail!("'+stale' is not supported yet (ADR 0003 phase 5)"),
            _ => {}
        }
        if tok.starts_with(':') {
            bail!("reports ({tok}) are not supported yet (ADR 0003 phase 5)");
        }
        if let Some((attr, val)) = split_attr(tok) {
            match attr {
                Attr::Repo => plan.repo_filters.push(val.to_string()),
                Attr::Branch => plan.branch_filters.push(val.to_string()),
                Attr::Key => plan.key_filters.push(val.to_string()),
                Attr::Root => plan.root_filters.push(val.to_string()),
                Attr::Idle => {
                    let (cmp, rest) = split_cmp(val, true)
                        .ok_or_else(|| anyhow::anyhow!("idle needs a comparator, e.g. idle:>7d"))?;
                    plan.attr_filters
                        .push(AttrFilter::Idle(cmp, parse_duration(rest)?));
                }
                // ahead/behind take an optional operator, so split_cmp(_, false)
                // never returns None: a bare `ahead:N` means `ahead == N`.
                Attr::Ahead => {
                    let (cmp, rest) = split_cmp(val, false).expect("optional op never None");
                    plan.attr_filters
                        .push(AttrFilter::Ahead(cmp, parse_count(rest)?));
                    plan.need_ahead_behind = true;
                }
                Attr::Behind => {
                    let (cmp, rest) = split_cmp(val, false).expect("optional op never None");
                    plan.attr_filters
                        .push(AttrFilter::Behind(cmp, parse_count(rest)?));
                    plan.need_ahead_behind = true;
                }
            }
        } else {
            plan.terms.push(tok.to_string());
        }
    }
    Ok(plan)
}

/// Options for [`resolve_plan`].
pub struct ResolveOptions<'a> {
    /// Active context from `state.toml` — used when the query has no `@`.
    pub current_context: Option<&'a str>,
}

/// Whether an explicit `@` token in the query should update persisted config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextPersistence {
    /// `@name` — save to `state.toml`.
    Set(String),
    /// `@none` / `@all` — clear `state.toml`.
    Clear,
    /// No `@` token — leave config unchanged.
    Unchanged,
}

/// Reserved `@`-names that CLEAR the active context rather than naming a real
/// one; both map to [`ContextPersistence::Clear`].
const CONTEXT_RESET_NAMES: [&str; 2] = ["none", "all"];

/// True for the reserved names that clear (not select) the active context.
fn is_context_reset(name: &str) -> bool {
    CONTEXT_RESET_NAMES.contains(&name)
}

/// Enforces the at-most-one-`@context` rule and returns the lone `@`-token
/// (including its `@` prefix), or `None` when the query has no context token.
fn single_context_token<'a>(tokens: &[&'a str]) -> Result<Option<&'a str>> {
    let mut at_tokens = tokens.iter().filter(|t| t.starts_with('@'));
    let first = at_tokens.next();
    if at_tokens.next().is_some() {
        bail!("only one @context per query");
    }
    Ok(first.copied())
}

/// Parses `@` usage for config persistence (call after [`resolve_plan`] succeeds).
pub fn context_persistence_from_query(input: &str) -> Result<ContextPersistence> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    match single_context_token(&tokens)? {
        None => Ok(ContextPersistence::Unchanged),
        Some(tok) => {
            let name = tok.strip_prefix('@').unwrap();
            if is_context_reset(name) {
                Ok(ContextPersistence::Clear)
            } else {
                Ok(ContextPersistence::Set(name.to_string()))
            }
        }
    }
}

/// Merges two plans with AND semantics across filters and OR across flags.
pub fn merge_scan_plans(base: ScanPlan, overlay: ScanPlan) -> ScanPlan {
    ScanPlan {
        terms: {
            let mut terms = base.terms;
            terms.extend(overlay.terms);
            terms
        },
        repo_filters: {
            let mut filters = base.repo_filters;
            filters.extend(overlay.repo_filters);
            filters
        },
        branch_filters: {
            let mut filters = base.branch_filters;
            filters.extend(overlay.branch_filters);
            filters
        },
        key_filters: {
            let mut filters = base.key_filters;
            filters.extend(overlay.key_filters);
            filters
        },
        root_filters: {
            let mut filters = base.root_filters;
            filters.extend(overlay.root_filters);
            filters
        },
        attr_filters: {
            let mut filters = base.attr_filters;
            filters.extend(overlay.attr_filters);
            filters
        },
        include_ignored: base.include_ignored || overlay.include_ignored,
        need_ahead_behind: base.need_ahead_behind || overlay.need_ahead_behind,
    }
}

fn validate_context_filter(name: &str, filter: &str) -> Result<()> {
    for tok in filter.split_whitespace() {
        if tok.contains('@') {
            bail!("context '{name}' filter token '{tok}' cannot contain '@' (reports are ADR 0003 phase 5)");
        }
        if tok.starts_with(':') {
            bail!("context '{name}' filter token '{tok}' cannot contain ':' (reports are ADR 0003 phase 5)");
        }
    }
    Ok(())
}

/// Resolves `@context` tokens and default context into a single [`ScanPlan`].
pub fn resolve_plan(input: &str, cfg: &Config, opts: &ResolveOptions) -> Result<ScanPlan> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let has_at = single_context_token(&tokens)?.is_some();
    let mut plans = Vec::new();

    if !has_at {
        if let Some(ctx) = opts.current_context {
            let filter = cfg.context_filter(ctx)?;
            validate_context_filter(ctx, filter)?;
            plans.push(parse(filter)?);
        }
    }

    let mut user_tokens = Vec::new();
    for tok in tokens {
        if let Some(name) = tok.strip_prefix('@') {
            if is_context_reset(name) {
                continue;
            }
            let filter = cfg.context_filter(name)?;
            validate_context_filter(name, filter)?;
            plans.push(parse(filter)?);
        } else {
            user_tokens.push(tok);
        }
    }

    if !user_tokens.is_empty() {
        plans.push(parse(&user_tokens.join(" "))?);
    }

    match plans.len() {
        0 => Ok(ScanPlan::default()),
        1 => Ok(plans.remove(0)),
        _ => Ok(plans
            .into_iter()
            .reduce(merge_scan_plans)
            .expect("len checked >= 2")),
    }
}

/// The closed set of recognized attribute names. Single source of truth so the
/// name->kind mapping lives in exactly one place (see [`Attr::parse`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attr {
    Repo,
    Branch,
    Key,
    Root,
    Idle,
    Ahead,
    Behind,
}

impl Attr {
    /// Maps an attribute name to its kind. `None` for anything outside the set,
    /// which lets `split_attr` fall through to treating the token as a bare term.
    fn parse(name: &str) -> Option<Attr> {
        match name {
            "repo" => Some(Attr::Repo),
            "branch" => Some(Attr::Branch),
            "key" => Some(Attr::Key),
            "root" => Some(Attr::Root),
            "idle" => Some(Attr::Idle),
            "ahead" => Some(Attr::Ahead),
            "behind" => Some(Attr::Behind),
            _ => None,
        }
    }
}

/// Returns `(attr, value)` when `tok` is `name:value` and `name` is a known
/// attribute; otherwise `None` (the caller treats the token as a bare term).
fn split_attr(tok: &str) -> Option<(Attr, &str)> {
    let (name, val) = tok.split_once(':')?;
    Some((Attr::parse(name)?, val))
}

/// Splits a leading comparator off a value. When `require_op` and none is
/// present, returns `None`; otherwise defaults to `Cmp::Eq`.
fn split_cmp(val: &str, require_op: bool) -> Option<(Cmp, &str)> {
    for (prefix, cmp) in [
        (">=", Cmp::Ge),
        ("<=", Cmp::Le),
        (">", Cmp::Gt),
        ("<", Cmp::Lt),
    ] {
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
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
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

impl ScanPlan {
    /// True when the candidate satisfies every term, substring filter, and
    /// attribute. `root_filters` are intentionally ignored here (push-down).
    pub fn matches(&self, c: &Candidate, now: DateTime<Utc>) -> bool {
        if c.ignored && !self.include_ignored {
            return false;
        }
        let contains_ci =
            |hay: &str, needle: &str| hay.to_lowercase().contains(&needle.to_lowercase());
        for t in &self.terms {
            if !(contains_ci(c.repo_name, t) || contains_ci(c.branch, t) || contains_ci(c.key, t)) {
                return false;
            }
        }
        for f in &self.repo_filters {
            if !contains_ci(c.repo_name, f) {
                return false;
            }
        }
        for f in &self.branch_filters {
            if !contains_ci(c.branch, f) {
                return false;
            }
        }
        for f in &self.key_filters {
            if !contains_ci(c.key, f) {
                return false;
            }
        }
        for attr in &self.attr_filters {
            let ok = match attr {
                AttrFilter::Idle(cmp, dur) => {
                    cmp.test_i64((now - c.last_commit).num_seconds(), dur.num_seconds())
                }
                AttrFilter::Ahead(cmp, n) => {
                    c.ahead.is_some_and(|a| cmp.test_i64(a.into(), (*n).into()))
                }
                AttrFilter::Behind(cmp, n) => c
                    .behind
                    .is_some_and(|b| cmp.test_i64(b.into(), (*n).into())),
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ContextDef};
    use std::collections::BTreeMap;

    fn test_cfg() -> Config {
        Config {
            contexts: BTreeMap::from([
                (
                    "work".into(),
                    ContextDef {
                        filter: "root:~/work".into(),
                    },
                ),
                (
                    "personal".into(),
                    ContextDef {
                        filter: "root:~/personal".into(),
                    },
                ),
                (
                    "recent-work".into(),
                    ContextDef {
                        filter: "root:~/work idle:<=30d".into(),
                    },
                ),
            ]),
            ..Config::default()
        }
    }

    #[test]
    fn parse_bare_terms_and_substring_attrs() {
        let p = parse("api feat/login repo:billing branch:fix/ key:work/api root:~/work").unwrap();
        assert_eq!(p.terms, vec!["api".to_string(), "feat/login".to_string()]);
        assert_eq!(p.repo_filters, vec!["billing".to_string()]);
        assert_eq!(p.branch_filters, vec!["fix/".to_string()]);
        assert_eq!(p.key_filters, vec!["work/api".to_string()]);
        assert_eq!(p.root_filters, vec!["~/work".to_string()]);
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

    #[test]
    fn parse_ignored_tags() {
        assert!(parse("+ignored").unwrap().include_ignored);
        assert!(!parse("-ignored").unwrap().include_ignored);
        assert!(!parse("api").unwrap().include_ignored); // default hides
    }

    #[test]
    fn reserved_report_and_stale_error_clearly() {
        assert!(parse(":hot").unwrap_err().to_string().contains("report"));
        assert!(parse("+stale").unwrap_err().to_string().contains("stale"));
    }

    #[test]
    fn merge_scan_plans_combines_root_filters() {
        let a = parse("root:~/work").unwrap();
        let b = parse("root:~/personal").unwrap();
        let merged = merge_scan_plans(a, b);
        assert_eq!(
            merged.root_filters,
            vec!["~/work".to_string(), "~/personal".to_string()]
        );
    }

    #[test]
    fn resolve_plan_applies_current_context() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: Some("work"),
        };
        let expected = parse("root:~/work api").unwrap();
        let got = resolve_plan("api", &cfg, &opts).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn resolve_plan_explicit_context_replaces_current() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: Some("work"),
        };
        let expected = parse("root:~/personal api").unwrap();
        let got = resolve_plan("@personal api", &cfg, &opts).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn resolve_plan_none_clears_current() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: Some("work"),
        };
        let expected = parse("api").unwrap();
        let got = resolve_plan("@none api", &cfg, &opts).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn resolve_plan_unknown_context_errors() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: Some("work"),
        };
        let err = resolve_plan("@missing", &cfg, &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown context '@missing'"), "got: {err}");
    }

    #[test]
    fn resolve_plan_context_with_idle_filter() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: None,
        };
        let plan = resolve_plan("@recent-work", &cfg, &opts).unwrap();
        assert_eq!(plan.root_filters, vec!["~/work".to_string()]);
        assert_eq!(
            plan.attr_filters,
            vec![AttrFilter::Idle(Cmp::Le, Duration::days(30))]
        );
    }

    #[test]
    fn resolve_plan_rejects_nested_context_in_filter() {
        let cfg = Config {
            contexts: BTreeMap::from([(
                "bad".into(),
                ContextDef {
                    filter: "@work".into(),
                },
            )]),
            ..Config::default()
        };
        let opts = ResolveOptions {
            current_context: None,
        };
        let err = resolve_plan("@bad", &cfg, &opts).unwrap_err().to_string();
        assert!(err.contains("cannot contain '@'"), "got: {err}");
        assert!(err.contains("@work"), "got: {err}");
    }

    #[test]
    fn resolve_plan_rejects_two_context_tokens() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: None,
        };
        let err = resolve_plan("@work @personal", &cfg, &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("only one @context per query"), "got: {err}");
    }

    #[test]
    fn context_persistence_from_explicit_context() {
        assert_eq!(
            context_persistence_from_query("@work api").unwrap(),
            ContextPersistence::Set("work".into())
        );
    }

    #[test]
    fn context_persistence_none_clears() {
        assert_eq!(
            context_persistence_from_query("@none").unwrap(),
            ContextPersistence::Clear
        );
        assert_eq!(
            context_persistence_from_query("@all").unwrap(),
            ContextPersistence::Clear
        );
    }

    #[test]
    fn context_persistence_unchanged_without_at() {
        assert_eq!(
            context_persistence_from_query("api idle:>7d").unwrap(),
            ContextPersistence::Unchanged
        );
    }

    #[test]
    fn resolve_plan_report_still_errors() {
        let cfg = test_cfg();
        let opts = ResolveOptions {
            current_context: None,
        };
        let err = resolve_plan(":hot", &cfg, &opts).unwrap_err().to_string();
        assert!(err.contains("report"), "got: {err}");
    }

    fn cand<'a>(repo: &'a str, branch: &'a str, key: &'a str, days_idle: i64) -> Candidate<'a> {
        Candidate {
            repo_name: repo,
            branch,
            key,
            last_commit: Utc::now() - Duration::days(days_idle),
            ahead: Some(1),
            behind: Some(0),
            ignored: false,
        }
    }

    #[test]
    fn matches_terms_case_insensitive_over_repo_branch_key() {
        let p = parse("API").unwrap();
        let c = cand("my-api", "feat/x", "work/my-api/feat/x", 1);
        assert!(p.matches(&c, Utc::now()));
        let p2 = parse("nope").unwrap();
        assert!(!p2.matches(&c, Utc::now()));
    }

    #[test]
    fn matches_idle_and_numeric_attrs() {
        let now = Utc::now();
        let c = cand("api", "feat/x", "w/api/feat/x", 10);
        assert!(parse("idle:>7d").unwrap().matches(&c, now));
        assert!(!parse("idle:<7d").unwrap().matches(&c, now));
        assert!(parse("behind:0").unwrap().matches(&c, now));
        assert!(!parse("behind:>0").unwrap().matches(&c, now));
    }

    #[test]
    fn matches_excludes_ignored_unless_plus_ignored() {
        let now = Utc::now();
        let mut c = cand("api", "feat/x", "w/api/feat/x", 1);
        c.ignored = true;
        assert!(!parse("api").unwrap().matches(&c, now));
        assert!(parse("api +ignored").unwrap().matches(&c, now));
    }

    #[test]
    fn matches_none_ahead_behind_fails_the_attr() {
        let now = Utc::now();
        let mut c = cand("api", "feat/x", "w/api/feat/x", 1);
        c.behind = None;
        assert!(!parse("behind:0").unwrap().matches(&c, now));
    }
}
