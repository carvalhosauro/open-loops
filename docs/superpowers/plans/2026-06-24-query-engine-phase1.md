# Query Engine — Phase 1 (parser + canonical key) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> **Model routing:** Plan = Opus xhigh | Exec `[atômica]` = Sonnet high | Exec `[complexa]` = Opus xhigh | Review = Opus xhigh | Final = Opus xhigh

**Goal:** Ship `loops [query]` — filter the loop inventory by terms/attributes — and migrate every key to the stable 3-segment form `root-label/repo/branch`.

**Architecture:** A new pure `src/query.rs` parses a query string into a `ScanPlan` and evaluates it against candidate loops in memory. `OpenLoop` gains a `root_label` field (resolved from the owning configured root, alias-aware) so `key()`, the distill-cache path, `resolve_loop`, and `ignores` all become 3-segment. This phase keeps the existing eager scan and filters its results in memory; Phase 2 adds git push-down and the two-phase split.

**Tech Stack:** Rust 2021, `clap` v4 (derive), `anyhow`, `chrono`, `serde`/`toml`. Tests use real git repos in tempdirs (`src/testutil.rs`) and `assert_cmd` for the binary.

**Source spec:** `docs/decisions/0003-query-engine.md` (ADR 0003, accepted).

## Scope

This plan implements ADR phases **1a** (parser → `ScanPlan`) and **1b** (canonical key + clap surface + working in-memory filtering). Out of scope here, each a follow-on plan:

- **Phase 2** — root/repo push-down + light/heavy git split + `ahead`/`behind` → `Option<u32>` lazy.
- **Phase 3** — `inventory.rs` SHA-validated memo + `refresh`/`--fresh`.
- **Phase 4** — contexts `@name`/`@none`/`@all`.
- **Phase 5** — reports `:name` + `+stale` + `loops help query`.
- **Phase 6** — `worktrees [query]`.

Tokens reserved-but-not-implemented here (`@…`, `:…`, `+stale`) parse to a clear "not yet supported" error so the grammar is stable.

After this plan: `loops api`, `loops feat/login`, `loops idle:>7d`, `loops behind:>0`, `loops repo:api branch:feat/`, `loops +ignored` all work. `ahead`/`behind` stay non-`Option` `u32` (eager scan still always computes them); the `Option` change lands in Phase 2.

## Global Constraints

- All user-facing output and error messages in **English**, actionable.
- All `#[test]` names and code comments in **English**; comments only explain *why*.
- Tolerant parsing of git output: a bad line → skip + warning, never abort (mirror `scanner::scan`).
- Conventional Commits; subjects in English.
- `just lint` (clippy `-D warnings`) and `just fmt` clean before each commit.
- Coverage gate 70% total (core target 85%) — `just cov` must pass.
- Docs are part of Definition of Done (`docs/features.md`, `docs/configuration.md` are source of truth for commands/config).
- **Breaking change**: keys become 3-segment; old `ignores.toml` entries and old distill-cache paths stop matching. Documented in CHANGELOG; no compat shim (pre-1.0).

---

## File Structure

- **Create** `src/query.rs` — pure query parser + evaluator. Responsibility: `&str` → `ScanPlan`; `ScanPlan::matches(&Candidate, now) -> bool`. No git, no I/O.
- **Modify** `src/lib.rs` — register `pub mod query;`.
- **Modify** `src/config.rs` — add `aliases: BTreeMap<String, String>`; add `Config::resolve_labels()` (collision-checked) + `label_for_repo()`.
- **Modify** `src/scanner.rs` — `OpenLoop` gains `root_label`; `key()` → 3-segment; `open_loops` takes a `root_label`; `scan` resolves labels per repo.
- **Modify** `src/cache.rs` — distill path gains the `root_label` segment.
- **Modify** `src/cli.rs` — `Cli` gains a top-level `query: Vec<String>`; `run_list` applies the query; `resolve_loop` uses the parser and includes ignored loops.
- **Modify** `src/main.rs` — dispatch the default (no-subcommand) action with the query.
- **Modify** `src/output.rs` — no behavior change this phase (keys are longer; width logic already adapts).
- **Modify** `tests/cli.rs` — e2e tests for `loops [query]` and the 3-segment key.
- **Modify** `docs/features.md`, `docs/configuration.md`, `CHANGELOG.md`, `CLAUDE.md` — document query + aliases + the breaking key.

---

### Task 1: `query.rs` skeleton — types + module registration `[atômica]`

**Files:**
- Create: `src/query.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `ScanPlan`, `AttrFilter`, `Cmp`, `Candidate<'a>` (consumed by Tasks 2–5, 10).

- [ ] **Step 1: Create `src/query.rs` with the types**

```rust
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
```

- [ ] **Step 2: Register the module in `src/lib.rs`**

Add after the existing module declarations (e.g. after `pub mod output;`):

```rust
pub mod query;
```

- [ ] **Step 3: Confirm it compiles**

Run: `cargo build`
Expected: compiles (unused-warnings are fine for now; `-D warnings` is enforced only at commit time once code is used).

- [ ] **Step 4: Commit**

```bash
git add src/query.rs src/lib.rs
git commit -m "feat(query): add ScanPlan, AttrFilter, Candidate types"
```

---

### Task 2: parse bare terms + `repo:`/`branch:`/`root:`/`key:` `[atômica]`

**Files:**
- Modify: `src/query.rs`

**Interfaces:**
- Produces: `pub fn parse(input: &str) -> Result<ScanPlan>` (extended in Tasks 3–4).

- [ ] **Step 1: Write the failing test**

In `src/query.rs`, add at the bottom:

```rust
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
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib query::tests::parse_bare_terms_and_substring_attrs`
Expected: FAIL — `parse` not found.

- [ ] **Step 3: Implement `parse` (terms + substring attrs)**

Add above the `tests` module:

```rust
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
    matches!(name, "repo" | "branch" | "key" | "root").then_some((name, val))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib query::tests`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add src/query.rs
git commit -m "feat(query): parse bare terms and substring attributes"
```

---

### Task 3: parse `idle:`/`ahead:`/`behind:` with comparators + durations `[complexa]`

**Files:**
- Modify: `src/query.rs`

**Interfaces:**
- Consumes: `parse` (Task 2), `AttrFilter`, `Cmp`.
- Produces: comparator/duration parsing folded into `parse`; sets `need_ahead_behind` for ahead/behind attrs.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
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
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib query::tests::parse_numeric_and_duration_attrs`
Expected: FAIL — `idle`/`ahead`/`behind` not handled; `parse_duration` not found.

- [ ] **Step 3: Extend `split_attr` to recognize the new names**

Change the `matches!` line in `split_attr` to include them:

```rust
    matches!(
        name,
        "repo" | "branch" | "key" | "root" | "idle" | "ahead" | "behind"
    )
    .then_some((name, val))
```

- [ ] **Step 4: Handle the new attrs in `parse`**

In `parse`, replace the `match name { … }` arms by adding the three attr cases before the `_ => unreachable!()` arm:

```rust
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
```

- [ ] **Step 5: Add the helper functions**

Add below `split_attr`:

```rust
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
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib query::tests`
Expected: PASS (all query tests).

- [ ] **Step 7: Commit**

```bash
git add src/query.rs
git commit -m "feat(query): parse idle/ahead/behind comparators and durations"
```

---

### Task 4: parse tags + reserve `@`/`:`/`+stale` `[atômica]`

**Files:**
- Modify: `src/query.rs`

**Interfaces:**
- Produces: tag handling in `parse`; reserved-token errors.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
    #[test]
    fn parse_ignored_tags() {
        assert!(parse("+ignored").unwrap().include_ignored);
        assert!(!parse("-ignored").unwrap().include_ignored);
        assert!(!parse("api").unwrap().include_ignored); // default hides
    }

    #[test]
    fn reserved_context_report_stale_error_clearly() {
        assert!(parse("@work").unwrap_err().to_string().contains("context"));
        assert!(parse(":hot").unwrap_err().to_string().contains("report"));
        assert!(parse("+stale").unwrap_err().to_string().contains("stale"));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib query::tests::parse_ignored_tags`
Expected: FAIL — tags fall through to `terms`.

- [ ] **Step 3: Handle tags + reserved tokens at the top of the `parse` loop**

In `parse`, at the very start of the `for tok in …` loop body (before the `split_attr` check), add:

```rust
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
        if tok.starts_with('@') {
            bail!("contexts (@{}) are not supported yet (ADR 0003 phase 4)", &tok[1..]);
        }
        if tok.starts_with(':') {
            bail!("reports ({tok}) are not supported yet (ADR 0003 phase 5)");
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib query::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/query.rs
git commit -m "feat(query): handle ignored tags, reserve contexts/reports/stale"
```

---

### Task 5: `ScanPlan::matches` evaluator `[complexa]`

**Files:**
- Modify: `src/query.rs`

**Interfaces:**
- Consumes: `ScanPlan`, `Candidate`, `AttrFilter`, `Cmp`.
- Produces: `pub fn matches(&self, c: &Candidate, now: DateTime<Utc>) -> bool` on `ScanPlan` (consumed by Task 10). `root_filter` is NOT applied here — it is push-down handled by the scan caller.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
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
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib query::tests::matches_terms_case_insensitive_over_repo_branch_key`
Expected: FAIL — `matches` not found.

- [ ] **Step 3: Implement `matches` on `ScanPlan`**

Add an `impl ScanPlan` block above the `tests` module:

```rust
impl ScanPlan {
    /// True when the candidate satisfies every term, substring filter, and
    /// attribute. `root_filter` is intentionally ignored here (push-down).
    pub fn matches(&self, c: &Candidate, now: DateTime<Utc>) -> bool {
        if c.ignored && !self.include_ignored {
            return false;
        }
        let contains_ci = |hay: &str, needle: &str| {
            hay.to_lowercase().contains(&needle.to_lowercase())
        };
        for t in &self.terms {
            if !(contains_ci(c.repo_name, t)
                || contains_ci(c.branch, t)
                || contains_ci(c.key, t))
            {
                return false;
            }
        }
        if let Some(f) = &self.repo_filter {
            if !contains_ci(c.repo_name, f) {
                return false;
            }
        }
        if let Some(f) = &self.branch_filter {
            if !contains_ci(c.branch, f) {
                return false;
            }
        }
        if let Some(f) = &self.key_filter {
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
                AttrFilter::Behind(cmp, n) => {
                    c.behind.is_some_and(|b| cmp.test_i64(b.into(), (*n).into()))
                }
            };
            if !ok {
                return false;
            }
        }
        true
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib query::tests`
Expected: PASS (all query tests).

- [ ] **Step 5: Lint + commit**

```bash
cargo clippy --all-targets -- -D warnings
git add src/query.rs
git commit -m "feat(query): evaluate ScanPlan against candidate loops"
```

---

### Task 6: config `aliases` + root-label resolution with collision error `[complexa]`

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Produces: `Config.aliases: BTreeMap<String, String>`; `Config::resolve_labels(&self) -> Result<Vec<(PathBuf, String)>>`; `fn label_for_repo(labels: &[(PathBuf, String)], repo: &Path) -> String` (consumed by Task 7).

- [ ] **Step 1: Write the failing tests**

In `src/config.rs` `mod tests`, add:

```rust
    #[test]
    fn resolve_labels_uses_basename_then_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let work = tmp.path().join("work");
        let personal = tmp.path().join("personal");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&personal).unwrap();
        let mut cfg = Config {
            roots: vec![work.clone(), personal.clone()],
            ..Config::default()
        };
        let labels = cfg.resolve_labels().unwrap();
        assert!(labels.contains(&(work.clone(), "work".to_string())));
        // alias overrides basename
        cfg.aliases.insert(personal.to_string_lossy().into_owned(), "p".into());
        let labels = cfg.resolve_labels().unwrap();
        assert!(labels.contains(&(personal.clone(), "p".to_string())));
        let _ = store;
    }

    #[test]
    fn resolve_labels_errors_on_collision_without_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a/repos");
        let b = tmp.path().join("b/repos");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let cfg = Config {
            roots: vec![a, b],
            ..Config::default()
        };
        let err = cfg.resolve_labels().unwrap_err().to_string();
        assert!(err.contains("share label"), "got: {err}");
        assert!(err.contains("alias"), "got: {err}");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib config::tests::resolve_labels_uses_basename_then_alias`
Expected: FAIL — `aliases`/`resolve_labels` not found.

- [ ] **Step 3: Add the `aliases` field**

In `src/config.rs`, add the import at the top:

```rust
use std::collections::BTreeMap;
```

Add the field to `struct Config` (after `roots`):

```rust
    /// Optional per-root label override, keyed by the canonical root path.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
```

Add `aliases: BTreeMap::new(),` to the `Default` impl's struct literal.

- [ ] **Step 4: Implement `resolve_labels` and `label_for_repo`**

Add to `impl Config` (create the block if absent; `Config` currently has only derives, so add a fresh `impl Config { … }` near `impl Default`):

```rust
impl Config {
    /// Resolves a stable label per root (alias, else basename). Errors when two
    /// roots resolve to the same label and no alias disambiguates them.
    pub fn resolve_labels(&self) -> Result<Vec<(std::path::PathBuf, String)>> {
        let mut out: Vec<(std::path::PathBuf, String)> = Vec::new();
        for root in &self.roots {
            let label = self
                .aliases
                .get(&root.to_string_lossy().into_owned())
                .cloned()
                .unwrap_or_else(|| {
                    root.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.to_string_lossy().into_owned())
                });
            if let Some((other, _)) = out.iter().find(|(_, l)| *l == label) {
                anyhow::bail!(
                    "roots {} and {} share label '{label}'; set an alias in config.toml",
                    other.display(),
                    root.display()
                );
            }
            out.push((root.clone(), label));
        }
        Ok(out)
    }
}

/// Label of the configured root that owns `repo` (longest path prefix wins).
pub fn label_for_repo(labels: &[(std::path::PathBuf, String)], repo: &std::path::Path) -> String {
    labels
        .iter()
        .filter(|(root, _)| repo.starts_with(root))
        .max_by_key(|(root, _)| root.as_os_str().len())
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| {
            repo.parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib config::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add root aliases and collision-checked label resolution"
```

---

### Task 7: `OpenLoop.root_label` + 3-segment `key()` + scan wiring `[complexa]`

**Files:**
- Modify: `src/scanner.rs`

**Interfaces:**
- Consumes: `config::label_for_repo`, `config::Config::resolve_labels`.
- Produces: `OpenLoop.root_label: String`; `OpenLoop::key()` → `root_label/repo_name/branch`; `open_loops(repo, root_label)`; `scan(roots, labels)`.

- [ ] **Step 1: Update the failing unit tests in `scanner.rs`**

In `scanner.rs` tests, update `open_loops_finds_unmerged_ignores_merged_and_default` to pass a label and assert the new key:

```rust
        let loops = open_loops(&repo, "root").unwrap();
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.branch, "feat/x");
        assert_eq!(l.repo_name, "app");
        assert_eq!(l.root_label, "root");
        assert_eq!(l.key(), "root/app/feat/x");
```

And update `scan_aggregates_repos_and_reports_warning_without_aborting` to build labels and call the new `scan`:

```rust
        let labels = vec![(tmp.path().to_path_buf(), "r".to_string())];
        let (loops, warnings) = scan(&[tmp.path().to_path_buf()], &labels);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].key(), "r/good/feat/ok");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("empty"));
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib scanner::tests::open_loops_finds_unmerged_ignores_merged_and_default`
Expected: FAIL — `root_label` missing; `open_loops`/`scan` arities wrong.

- [ ] **Step 3: Add the field and change `key()`**

In `struct OpenLoop`, add as the first field:

```rust
    pub root_label: String,
```

Change `key()`:

```rust
    /// Canonical key used in resume/ignore: "root-label/repo/branch".
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.root_label, self.repo_name, self.branch)
    }
```

- [ ] **Step 4: Thread `root_label` through `open_loops`**

Change the signature and the struct construction:

```rust
pub fn open_loops(repo: &Path, root_label: &str) -> Result<Vec<OpenLoop>> {
```

In the `result.push(OpenLoop { … })`, add `root_label: root_label.to_string(),` as the first field.

- [ ] **Step 5: Update `scan` to resolve labels per repo**

Change `scan` to accept the resolved labels and pass each repo's label:

```rust
pub fn scan(roots: &[PathBuf], labels: &[(PathBuf, String)]) -> (Vec<OpenLoop>, Vec<String>) {
    let repos = find_repos(roots);
    let results: Vec<Result<Vec<OpenLoop>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| {
                let label = crate::config::label_for_repo(labels, repo);
                s.spawn(move || open_loops(repo, &label))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("panic while scanning repository")))
            })
            .collect()
    });
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut loops) => all.append(&mut loops),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
        }
    }
    (all, warnings)
}
```

- [ ] **Step 6: Update the other `OpenLoop` constructors (keep the crate compiling)**

Adding a field to `OpenLoop` breaks every struct literal. Add `root_label` to
the test fixtures so the crate still builds:

In `src/cache.rs` `mod tests`, in `fake_loop`, add as the first field:

```rust
        OpenLoop {
            root_label: "work".into(),
            repo_name: "app".into(),
```

In `src/output.rs` `mod tests`, in `lp`, add as the first field:

```rust
        OpenLoop {
            root_label: "app".into(),
            repo_name: "app".into(),
```

- [ ] **Step 7: Rewire the two `scanner::scan` call sites in `src/cli.rs`**

`run_list` and `resolve_loop` both call `scanner::scan(&cfg.roots)`. Resolve
labels and pass them (behavior otherwise unchanged this task — query filtering
lands in Tasks 9–10). In `run_list`:

```rust
    let labels = cfg.resolve_labels()?;
    progress("scanning git repositories…");
    let (found, warnings) = scanner::scan(&cfg.roots, &labels);
```

In `resolve_loop`:

```rust
    let labels = cfg.resolve_labels()?;
    let (found, warnings) = scanner::scan(&cfg.roots, &labels);
```

- [ ] **Step 8: Fix the existing e2e key assertion**

In `tests/cli.rs`, in `full_flow_init_list_resume_cache_ignore`, the ignore step
passes a 2-segment key that no longer matches. The root dir is `projetos`, so the
key is now `projetos/meu-app/feat/login`. Change line ~98:

```rust
    loops(&home)
        .args(["ignore", "projetos/meu-app/feat/login"])
        .assert()
        .success();
```

(The `contains("meu-app/feat/login")` and `contains("app/feat/login")`
assertions elsewhere still pass — they are substrings of the 3-segment key.)

- [ ] **Step 9: Run the full suite**

Run: `cargo test`
Expected: PASS. The crate compiles and every test is green — keys are now
3-segment; `loops` still lists all (query filtering arrives in Task 9).

- [ ] **Step 10: Lint + commit**

```bash
cargo clippy --all-targets -- -D warnings && cargo fmt
git add src/scanner.rs src/cache.rs src/output.rs src/cli.rs tests/cli.rs
git commit -m "feat(scanner): add root_label and 3-segment canonical key"
```

---

### Task 8: distill cache path uses the `root_label` segment `[atômica]`

**Files:**
- Modify: `src/cache.rs`

**Interfaces:**
- Consumes: `OpenLoop.root_label` (Task 7).
- Produces: `Cache::path` → `cache/<root_label>/<repo>/<branch>@<sha>.md`.

- [ ] **Step 1: Add the failing test**

`fake_loop` already carries `root_label: "work"` (added in Task 7). Add a test
asserting that distinct labels keep distinct paths:

```rust
    #[test]
    fn path_includes_root_label_segment() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let lp = fake_loop("sha1");
        cache.put(&lp, "x").unwrap();
        // distinct labels for the same repo/branch must not collide
        let mut other = fake_loop("sha1");
        other.root_label = "personal".into();
        assert!(cache.get(&other).is_none());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --lib cache::tests::path_includes_root_label_segment`
Expected: FAIL — paths collide (label not in path) / `root_label` missing.

- [ ] **Step 3: Add the label segment in `Cache::path`**

```rust
    fn path(&self, lp: &OpenLoop) -> PathBuf {
        // branches contain '/', which cannot appear in a file name
        let branch = lp.branch.replace('/', "__");
        self.dir
            .join(&lp.root_label)
            .join(&lp.repo_name)
            .join(format!("{branch}@{}.md", lp.head_sha))
    }
```

- [ ] **Step 4: Run the cache tests**

Run: `cargo test --lib cache::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs
git commit -m "feat(cache): key distill cache by root_label to avoid name collisions"
```

---

### Task 9: clap `loops [query]` surface + `run_list` filtering `[complexa]`

**Files:**
- Modify: `src/cli.rs`, `src/main.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: `query::parse`, `query::Candidate`, `ScanPlan::matches`.
- Produces: `Cli { command: Option<Command>, query: Vec<String> }`; `run_list(base, query: &str)`; `main` dispatches the default list action with the joined query.

- [ ] **Step 1: Write the failing e2e test**

In `tests/cli.rs`, append (uses the existing `loops`/`git` helpers):

```rust
#[test]
fn list_filters_by_query_term() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    for name in ["api", "web"] {
        let repo = root.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "init"]);
        git(&repo, &["checkout", "-b", "feat/x"]);
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "wip"]);
    }
    loops(&home).arg("init").arg(&root).assert().success();

    // bare `loops` shows both; `loops api` shows only api, with 3-segment key
    loops(&home)
        .arg("api")
        .assert()
        .success()
        .stdout(predicate::str::contains("projects/api/feat/x"))
        .stdout(predicate::str::contains("web/feat/x").not());
}
```

(The root basename is `projects`, so the label is `projects` and the key is `projects/api/feat/x`.)

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --test cli list_filters_by_query_term`
Expected: FAIL — `api` is treated as an unknown subcommand (clap errors).

- [ ] **Step 3: Add the top-level positional in `src/cli.rs`**

Change `struct Cli`. `args_conflicts_with_subcommands` makes the query and the
subcommands mutually exclusive (so `loops resume …` routes to the subcommand,
`loops api …` to the query); `allow_hyphen_values` lets `-ignored` through as a
value instead of being parsed as a flag:

```rust
#[derive(Parser)]
#[command(name = "loops", version, about = "Recover the context of paused work")]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Filter the inventory (e.g. `loops api idle:>7d`). See ADR 0003 grammar.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub query: Vec<String>,
}
```

Shadow rule: a repo whose name equals a subcommand (`resume`, `ignore`, `wt`,
`worktrees`, `init`, `completions`) is shadowed — query it via `repo:<name>`.
If `cargo test` in Step 6 shows clap still rejecting `loops api`, the fallback
is `#[command(subcommand_negates_reqs = true)]` plus, as a last resort,
`allow_external_subcommands = true` merged into `query`; the Step 1 test is the
guard either way.

- [ ] **Step 4: Change `run_list` to take and apply the query in `src/cli.rs`**

```rust
pub fn run_list(base: &Path, query: &str) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let plan = crate::query::parse(query)?;
    let labels = cfg.resolve_labels()?;
    progress("scanning git repositories…");
    let (found, warnings) = scanner::scan(&cfg.roots, &labels);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let ignores = Ignores::load(base)?;
    let now = chrono::Utc::now();
    let visible: Vec<OpenLoop> = found
        .into_iter()
        .filter(|l| {
            let key = l.key();
            plan.matches(
                &crate::query::Candidate {
                    repo_name: &l.repo_name,
                    branch: &l.branch,
                    key: &key,
                    last_commit: l.last_commit,
                    ahead: Some(l.ahead),
                    behind: Some(l.behind),
                    ignored: ignores.contains(&key),
                },
                now,
            )
        })
        .collect();
    if visible.is_empty() && !query.trim().is_empty() {
        eprintln!("No loops match: {query}");
        eprintln!("(hint: run `loops` to list all)");
    }
    print!("{}", output::render_table(&visible, now));
    Ok(())
}
```

(`ahead`/`behind` are still `u32` this phase — wrapped in `Some`. Phase 2 makes
them `Option`, dropping the wrap.)

- [ ] **Step 5: Dispatch the query in `src/main.rs`**

Replace the `None` arm of the `match cli.command` block:

```rust
        None => cli::run_list(&base, &cli.query.join(" ")),
```

- [ ] **Step 6: Run the suite**

Run: `cargo test`
Expected: PASS, including `list_filters_by_query_term`. If clap rejects
`loops api`, apply the Step 3 fallback and re-run.

- [ ] **Step 7: Lint + commit**

```bash
cargo clippy --all-targets -- -D warnings && cargo fmt
git add src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat(cli): filter the inventory by query"
```

---

### Task 10: `resolve_loop` uses the parser + resume includes ignored `[complexa]`

**Files:**
- Modify: `src/cli.rs`

**Interfaces:**
- Consumes: `query::parse`, `query::Candidate`, `ScanPlan::matches`, `Ignores`.
- Produces: `resolve_loop(base, query)` parses the query, includes ignored, requires exactly 1 match. (`run_list` already filters — Task 9; label resolution already wired — Task 7.)

- [ ] **Step 1: Update `resolve_loop` to use the parser and include ignored**

Replace the body of `resolve_loop` (keep the signature `fn resolve_loop(base: &Path, query: &str) -> Result<OpenLoop>`):

```rust
fn resolve_loop(base: &Path, query: &str) -> Result<OpenLoop> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "no roots configured. Run: loops init <dir-with-your-repos>"
    );
    let mut plan = crate::query::parse(query)?;
    plan.include_ignored = true; // resume can target an ignored loop by key
    let labels = cfg.resolve_labels()?;
    let (found, warnings) = scanner::scan(&cfg.roots, &labels);
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let now = chrono::Utc::now();
    let matches: Vec<&OpenLoop> = found
        .iter()
        .filter(|l| {
            let key = l.key();
            plan.matches(
                &crate::query::Candidate {
                    repo_name: &l.repo_name,
                    branch: &l.branch,
                    key: &key,
                    last_commit: l.last_commit,
                    ahead: Some(l.ahead),
                    behind: Some(l.behind),
                    ignored: false,
                },
                now,
            )
        })
        .collect();
    match matches.len() {
        0 => bail!("no loop matches '{query}'. Run `loops` to see open ones."),
        1 => Ok(matches[0].clone()),
        _ => bail!(
            "ambiguous query, candidates:\n{}",
            matches
                .iter()
                .map(|l| format!("  {}", l.key()))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}
```

- [ ] **Step 2: Run the full suite**

Run: `cargo test`
Expected: PASS — including `resume_ambiguous_query_lists_candidates` (now matched
against 3-segment keys) and `resume` of an ignored loop.

- [ ] **Step 3: Lint + format**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs
git commit -m "feat(cli): resolve loops via the query parser, resume includes ignored"
```

---

### Task 11: docs, CHANGELOG, breaking-change note `[atômica]`

**Files:**
- Modify: `docs/features.md`, `docs/configuration.md`, `CHANGELOG.md`, `CLAUDE.md`

- [ ] **Step 1: Document the query in `docs/features.md`**

Under the `## \`loops\` — inventory` section, add after the existing description:

````markdown
### Filtering

```bash
loops api                 # repos/branches matching "api"
loops api idle:>7d        # plus idle more than 7 days
loops repo:api branch:fix/ behind:>0
loops +ignored            # include dismissed loops
```

Bare terms substring-match the repo, branch, or key (AND across terms).
Attributes: `repo:`, `branch:`, `key:`, `root:` (substring), `idle:` (needs a
comparator, e.g. `idle:>7d`; units m/h/d/w), `ahead:`/`behind:` (`>`,`<`,`>=`,
`<=`, or bare equality). Tags: `-ignored` (default), `+ignored`.
````

- [ ] **Step 2: Document aliases in `docs/configuration.md`**

Add a row to the config table:

```markdown
| `aliases` | table | `{}` | Per-root label override, keyed by canonical root path (resolves key collisions) |
```

And a short section:

````markdown
## Root labels

Keys are `root-label/repo/branch`. The label is the root's directory name, or an
alias when two roots share a name:

```toml
roots = ["/home/you/work", "/home/you/personal"]

[aliases]
"/home/you/work" = "w"
```

If two roots resolve to the same label and neither has an alias, `loops` exits
with an actionable error.
````

- [ ] **Step 3: Note the breaking key change + alias config in `CHANGELOG.md`**

Add under `## [Unreleased]` (create the section if absent):

```markdown
### Changed
- **Breaking:** loop keys are now `root-label/repo/branch` (was `repo/branch`).
  Existing `ignores.toml` entries and distill-cache paths no longer match;
  re-add ignores after upgrading.

### Added
- `loops [query]` — filter the inventory by terms and attributes (see features).
- `[aliases]` config table for per-root labels.
```

- [ ] **Step 4: Mention the query grammar source in `CLAUDE.md`**

In the `## Estrutura` list, add a line for the new module:

```markdown
- `src/query.rs` — parser de query → `ScanPlan` + avaliação em memória (ADR 0003)
```

- [ ] **Step 5: Full verification**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check && just cov`
Expected: tests pass, no warnings, no format diff, coverage ≥ 70% (core ≥ 85%). If `query.rs` drags coverage, add unit tests for any uncovered parse branch.

- [ ] **Step 6: Manual smoke test**

```bash
cargo run -- init ~/some/repo-root
cargo run -- api
cargo run -- idle:>1d
```

Expected: filtered tables with 3-segment keys; `loops` (no args) lists all.

- [ ] **Step 7: Commit**

```bash
git add docs/features.md docs/configuration.md CHANGELOG.md CLAUDE.md
git commit -m "docs: document loops query filtering and root aliases"
```

---

## Self-Review Notes

- **Spec coverage (ADR 0003 phases 1a/1b):** parser terms/attrs/tags + durations + errors (Tasks 2–4); `ScanPlan::matches` (Task 5); always-prefixed key + `OpenLoop.root_label` + `key()` (Task 7); `Cache::path` migration (Task 8); alias table + collision error (Task 6); clap `loops [query]` surface + shadow rule via `trailing_var_arg` (Task 9); `resolve_loop` 3-segment + resume includes ignored (Task 10); ignores break documented (Task 11). `root:` is parsed (Task 2) but resolved in Phase 2 — noted in `matches` and the scope section.
- **Deferred-by-design:** `@context`/`:report`/`+stale` error clearly (Task 4); push-down, two-phase git, and `ahead`/`behind: Option` are Phase 2 — `ahead/behind` stay `u32` and are wrapped in `Some(...)` at the two call sites (Tasks 10), a single mechanical edit when Phase 2 lands.
- **Type consistency:** `Candidate` fields (`repo_name`, `branch`, `key`, `last_commit`, `ahead: Option<u32>`, `behind: Option<u32>`, `ignored`) are identical in Tasks 1, 5, 10. `scan(roots, labels)` and `open_loops(repo, root_label)` signatures match between Task 7 (producer) and Task 10 (consumer). `resolve_labels` return type `Vec<(PathBuf, String)>` matches `label_for_repo`'s parameter in Tasks 6→7.
- **Green per task:** the cross-cutting key migration lands wholesale in Task 7 (struct + `key()` + `open_loops`/`scan` signatures + all `OpenLoop` constructors + both `cli` call sites + the existing e2e key assertion), so `cargo test` is green at the end of every task. Tasks 8–10 build additively on a compiling crate.
- **Existing-test impact (verified against `tests/cli.rs`):** only the `ignore` arg in `full_flow_init_list_resume_cache_ignore` needs the 3-segment key (Task 7 Step 8); all other `contains(...)` key assertions survive as substrings of the longer key.
- **clap risk:** `loops [query]` coexisting with subcommands needs `args_conflicts_with_subcommands` + `allow_hyphen_values` (Task 9 Step 3); the `list_filters_by_query_term` e2e is the guard, with a documented fallback if clap still mis-routes.
- **Out of scope honored:** no inventory cache, no contexts/reports, no worktree query in this plan.
