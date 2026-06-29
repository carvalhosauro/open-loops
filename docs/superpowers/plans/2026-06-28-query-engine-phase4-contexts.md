# ADR Fase 4 — Contexts `@` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persistent query scopes via `@context` tokens, `default_context` / `LOOPS_CONTEXT`, and `[contexts.X]` in config — so `loops` can run in a daily universe (work vs personal) without repeating `root:` every time.

**Architecture:** Keep `query::parse` pure (no I/O). Add `query::resolve_plan(input, cfg, opts)` that applies default context, expands `@` tokens from config, AND-merges sub-plans, then returns a single `ScanPlan`. CLI replaces bare `parse()` calls in list/resume/refresh. Root push-down intersects when multiple `root:` filters merge.

**Tech Stack:** Rust 1.89, existing `config` / `query` / `cli` layers, serde for new config tables.

**Spec:** [ADR 0003 phase 4](docs/decisions/0003-query-engine.md) · [ROADMAP](ROADMAP.md) §104

**Out of scope (this plan):**

- Reports (`:nome`) and `+stale` — phase 5
- `loops help query` — phase 5
- `@` or `:` inside a context `filter` string — error in phase 4 (reports may embed `@` in phase 5)
- More than one `@` token in the same user query — error with actionable hint
- `worktrees [query]` — phase 6

---

## Acceptance criteria (ROADMAP §104)

- [ ] `@nome` resolves `[contexts.nome].filter` from config
- [ ] `[contexts.X] filter = "..."` in `config.toml`
- [ ] `@none` / `@all` clear the default context for one invocation
- [ ] `default_context` (config) + `LOOPS_CONTEXT` (env) apply only when the query has no `@` token
- [ ] Explicit `@ctx` replaces `default_context` (Taskwarrior-style)
- [ ] Remove parser error `"contexts (@…) are not supported yet"`
- [ ] `:report` and `+stale` still error clearly (phase 5)

---

## Behaviour reference

### Config example

```toml
default_context = "work"

[contexts.work]
filter = "root:~/work"

[contexts.recent-work]
filter = "root:~/work idle:<=30d"

[contexts.personal]
filter = "root:~/personal"
```

### Resolution algorithm

```
tokens = split_whitespace(user_query)
has_at = any token starts with '@'

plans = []

if !has_at:
    ctx = LOOPS_CONTEXT env OR cfg.default_context
    if ctx: plans.push(parse(lookup(ctx).filter))

for tok in tokens:
    if tok starts with '@':
        name = rest
        if name in (none, all): skip   # clears default; adds nothing
        else: plans.push(parse(lookup(name).filter))
    else: user_tokens.push(tok)

plans.push(parse(join(user_tokens)))
return merge(plans)   # AND across all parts
```

### Precedence (ADR 0003)

| Input | Effective scope |
|---|---|
| `loops` + `default_context = "work"` | `[contexts.work]` |
| `loops api` + default | work ∧ term `api` |
| `loops @personal` + default=work | personal only (explicit replaces default) |
| `loops @none` + default=work | full universe (default ignored) |
| `LOOPS_CONTEXT=personal` + no `@` | same as `default_context = "personal"`; env wins over config |

Context `filter` is a full sub-query: `root:`, `repo:`, `branch:`, `idle:>7d`, bare terms, `+ignored`, etc.

---

## File map

| File | Responsibility |
|---|---|
| `src/config.rs` | `ContextDef`, `contexts` table, `default_context`, lookup helper |
| `src/query.rs` | `merge_scan_plans`, `resolve_plan`, `root_filters` vec, drop `@` bail |
| `src/cli.rs` | Wire `resolve_plan` in list / resume / refresh; read `LOOPS_CONTEXT` |
| `docs/configuration.md` | Document contexts, default, env |
| `docs/features.md` | Usage examples |
| `ROADMAP.md`, `CHANGELOG.md` | Mark phase 4 done |
| `tests/cli.rs` | E2E scoping with two roots |

---

### Task 1: Config — contexts table + default

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add types**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextDef {
    pub filter: String,
}

// In Config:
#[serde(default)]
pub default_context: Option<String>,
#[serde(default)]
pub contexts: BTreeMap<String, ContextDef>,
```

- [ ] **Step 2: Lookup helper**

```rust
impl Config {
    /// Returns the filter string for a named context.
    pub fn context_filter(&self, name: &str) -> Result<&str> {
        self.contexts
            .get(name)
            .map(|c| c.filter.as_str())
            .ok_or_else(|| /* actionable: unknown context '@name'; define [contexts.name] */)
    }
}
```

- [ ] **Step 3: Unit tests**

- Roundtrip TOML with `[contexts.work]` + `default_context`
- `context_filter("work")` ok; unknown name errors with `contexts.` hint
- Empty `contexts` default deserializes

- [ ] **Step 4: Commit** `feat(config): add contexts table and default_context`

---

### Task 2: Query — merge + resolve_plan

**Files:**
- Modify: `src/query.rs`

- [ ] **Step 1: Replace `root_filter: Option<String>` with `root_filters: Vec<String>`**

- `parse`: on `root:` attr, push onto `root_filters` (last `root:` in a single parse still allowed; multiple `root:` in one query = AND at resolve time)
- Update unit tests and `config::resolve_scan_roots` caller

- [ ] **Step 2: `merge_scan_plans(base, overlay) -> ScanPlan` (AND semantics)**

| Field | Merge rule |
|---|---|
| `terms` | extend |
| `repo_filter`, `branch_filter`, `key_filter` | if both `Some`, keep both constraints (evaluator already ANDs) — see Step 3 |
| `root_filters` | extend (intersect at resolve) |
| `attr_filters` | extend |
| `include_ignored` | `\|` (either side true) |
| `need_ahead_behind` | `\|` |

For optional substring filters (`repo`, `branch`, `key`): when merging two `Some`, concatenate is wrong. **Rule:** keep both by storing merged plan fields; extend `matches()` to accept a slice of optional filters OR merge into a single combined check — simplest v1 approach: **only one `repo_filter` per merged plan** is insufficient. Instead, add `repo_filters: Vec<String>` (same pattern as roots) OR run `matches` twice. **Recommended:** add `_filters` vecs for repo/branch/key mirroring `root_filters`; `parse` pushes; `matches` requires all entries to match.

Minimal alternative (acceptable for v1): context filters rarely repeat the same attr as the user query. Document that duplicate attr keys in a merged plan last-wins is **not** acceptable. Implement vec fields for `repo`, `branch`, `key` filters (mirror `root_filters`).

- [ ] **Step 3: Update `ScanPlan::matches`**

- Each `*_filters` vec: every entry must match (AND)
- Empty vec = no constraint

- [ ] **Step 4: `ResolveOptions` + `resolve_plan`**

```rust
pub struct ResolveOptions<'a> {
    /// LOOPS_CONTEXT env or cfg.default_context — caller resolves precedence.
    pub default_context: Option<&'a str>,
}

pub fn resolve_plan(input: &str, cfg: &Config, opts: &ResolveOptions) -> Result<ScanPlan>
```

Implementation:

1. Tokenize `input`; detect `has_at`
2. Build `Vec<ScanPlan>` as per algorithm above
3. For each context name, `cfg.context_filter(name)?` then validate filter string does not contain `@` or `:` — bail with `"context 'work' filter cannot contain '@' or ':' (reports are phase 5)"`
4. `parse(filter)` for each context part; `parse(user_tokens)` for remainder
5. Fold with `merge_scan_plans`
6. Leave `:…` and `+stale` errors inside `parse()` unchanged

- [ ] **Step 5: Unit tests**

| Test | Expect |
|---|---|
| `resolve_plan("api", cfg default=work)` | equivalent to `parse("root:~/work api")` |
| `resolve_plan("@personal api", default=work)` | personal only + api |
| `resolve_plan("@none api", default=work)` | `parse("api")` only |
| `resolve_plan("@work", unknown ctx)` | actionable error |
| context filter `root:~/work idle:<=30d` | both attrs present |
| filter containing `@work` | error |
| two `@` tokens | error: `only one @context per query` |
| `:hot` | still report error |
| merge two `root:` | both in `root_filters` |

- [ ] **Step 6: Remove** `reserved_context_report_stale_error_clearly` test for `@work`; replace with resolve tests. Keep `:hot` and `+stale` error tests on bare `parse`.

- [ ] **Step 7: Commit** `feat(query): resolve @context tokens into ScanPlan`

---

### Task 3: Config — intersect root filters

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Update `resolve_scan_roots`**

```rust
pub fn resolve_scan_roots(&self, plan: &ScanPlan) -> Result<Vec<PathBuf>> {
    if plan.root_filters.is_empty() {
        return Ok(self.roots.clone());
    }
    let labels = self.resolve_labels()?;
    let mut acc: Option<std::collections::HashSet<PathBuf>> = None;
    for filter in &plan.root_filters {
        let subset = self.roots_matching_filter(filter, &labels)?;
        acc = Some(match acc {
            None => subset.into_iter().collect(),
            Some(prev) => prev.intersection(&subset).cloned().collect(),
        });
    }
    Ok(acc.unwrap().into_iter().collect())
}
```

Extract `roots_matching_filter` from current single-filter logic.

- [ ] **Step 2: Tests**

- Two filters where intersection is empty → empty vec (not error)
- Single filter unchanged behaviour vs phase 2

- [ ] **Step 3: Commit** `feat(config): intersect multiple root filters in ScanPlan`

---

### Task 4: CLI wire-up

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Helper for default context precedence**

```rust
fn effective_default_context(cfg: &Config) -> Option<String> {
    std::env::var("LOOPS_CONTEXT")
        .ok()
        .or_else(|| cfg.default_context.clone())
}
```

Env wins over config (document in configuration.md).

- [ ] **Step 2: Replace `parse(query)?` in three call sites**

- `run_list`
- `resolve_loop` (used by `run_resume`)
- `run_refresh`

```rust
let default = effective_default_context(&cfg);
let plan = query::resolve_plan(
    query,
    &cfg,
    &query::ResolveOptions {
        default_context: default.as_deref(),
    },
)?;
```

- [ ] **Step 3: `resolve_loop` post-hook unchanged**

`plan.include_ignored = true` after resolve.

- [ ] **Step 4: Commit** `feat(cli): wire @context resolution in list, resume, refresh`

---

### Task 5: E2E tests

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Step 1: `context_default_scopes_roots`**

1. Temp home; two roots `work/` and `personal/` each with a repo+branch
2. Write config with `[contexts.work] filter = "root:<work>"`, `[contexts.personal] filter = "root:<personal>"`, `default_context = "work"`
3. `loops` → stdout contains work repo key, not personal
4. `LOOPS_CONTEXT=personal loops` → personal only
5. `loops @none` → both repos visible

- [ ] **Step 2: `context_explicit_overrides_default`**

- default=work; `loops @personal` → personal only

- [ ] **Step 3: `context_with_idle_filter`**

- `[contexts.recent] filter = "root:<work> idle:<=30d"` + old branch outside window → excluded

- [ ] **Step 4: `context_unknown_errors`**

- `loops @nope` → exit 1, stderr mentions `[contexts.nope]`

- [ ] **Step 5: `refresh_honours_context`**

- default=work; personal repo gets fresh inventory only on `loops refresh @personal` or `loops refresh` with both — verify scoped refresh skips out-of-scope repos (reuse existing refresh test patterns)

- [ ] **Step 6: Commit** `test(cli): @context scoping e2e`

---

### Task 6: Docs + ROADMAP

**Files:**
- Modify: `docs/configuration.md`, `docs/features.md`, `ROADMAP.md`, `CHANGELOG.md`

- [ ] **configuration.md** — add rows:

| Key | Type | Default | Description |
|---|---|---|---|
| `default_context` | string | — | Named context applied when the query has no `@` token |
| `[contexts.X]` | table | — | Saved scope; `filter` is a query string |

Document `LOOPS_CONTEXT` env (overrides `default_context`).

- [ ] **features.md** — new section **Contexts**:

```bash
loops @work              # explicit scope
loops                     # uses default_context when set
loops @none               # full universe, ignore default
loops @work api idle:>7d  # compose context + ad-hoc filters
```

Clarify: context = daily universe; not the same as reports (`:` — coming in phase 5).

- [ ] **ROADMAP.md** — mark phase 4 items `[x]`

- [ ] **CHANGELOG.md** — Unreleased entry: feat(contexts): `@` scopes, default_context, LOOPS_CONTEXT

- [ ] **Commit** `docs: contexts @ syntax and configuration (ADR 0003 phase 4)`

---

### Task 7: Verification

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] Manual smoke (optional):

```bash
export OPEN_LOOPS_HOME=/tmp/loops-ctx-test
# init two roots, set default_context, run loops / loops @none / loops @work
```

---

## Design decisions (locked)

1. **Default vs explicit `@`:** `default_context` / `LOOPS_CONTEXT` make `@` optional for daily use; `@` is for override or `@none` reset — not like reports which are always explicit (`:`).
2. **Context filter richness:** any parseable query fragment (`idle:<=30d`, `repo:api`, …); no `@` or `:` inside the filter string in phase 4.
3. **One `@` per user query:** keeps merge predictable; use ad-hoc filters to narrow further.
4. **Env location:** `LOOPS_CONTEXT` read in `cli.rs`, not `config.rs` (consistent with `OPEN_LOOPS_HOME` pattern).
5. **Reports unchanged:** `:nome` and `+stale` remain phase-5 errors from `parse()`.

---

## Commit sequence (suggested)

1. `feat(config): add contexts table and default_context`
2. `feat(query): resolve @context tokens into ScanPlan`
3. `feat(config): intersect multiple root filters in ScanPlan`
4. `feat(cli): wire @context resolution in list, resume, refresh`
5. `test(cli): @context scoping e2e`
6. `docs: contexts @ syntax and configuration (ADR 0003 phase 4)`
