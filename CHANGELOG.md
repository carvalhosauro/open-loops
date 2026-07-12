# Changelog
## unreleased

### Features
- Add a global `--verbose`/`-v` flag and `tracing`-based observability (WAVE 4.3).
  Progress phases (`scan`, `distill`, worktree matching) and recoverable
  diagnostics are now `tracing` events on stderr. Default level is `warn` (quiet
  runs, clean stdout for piping); `--verbose` raises the crate to `debug`, and an
  explicit `RUST_LOG` always takes precedence. ANSI colour is used only on an
  interactive stderr.

### Internals
- Add property-based tests for the query engine via `proptest` (WAVE 4.2):
  `parse` never panics, unknown `name:value` tokens stay bare terms,
  `parse_duration` accepts only `m|h|d|w`, and `idle:>N` is monotonic. Fills
  parser gaps in `scanner` (malformed and Windows-style worktree lines) and
  `distill` (multi-session prompt separators).
- Migrate progress and warning `eprintln!` calls across `cli`, `scanner`,
  `inventory`, and `index` to `tracing::info!`/`tracing::warn!`. The final
  `error: …` sink in `main` stays a plain `eprintln!` (user-facing failure
  output, not a log event).

## 1.6.2 - 2026-07-12

### Internals
- Migrate library API from anyhow to typed domain errors (WAVE 4.1) ([#45](https://github.com/carvalhosauro/open-loops/pull/45))

### Breaking
- **Library API:** public functions now return domain-specific `thiserror` enums
  (`QueryError`, `GitError`, `ConfigError`, …) wrapped by `OpenLoopsError` at the
  CLI boundary, instead of `anyhow::Result`. Consumers can `match` on variants;
  `anyhow` is no longer a dependency. See `src/error.rs` and PR
  [#45](https://github.com/carvalhosauro/open-loops/pull/45).

## 1.6.1 - 2026-07-11

### Internals
- Bump rusqlite from 0.32.1 to 0.39.0 ([#44](https://github.com/carvalhosauro/open-loops/pull/44))
- Bump clap_mangen from 0.2.33 to 0.3.0 ([#39](https://github.com/carvalhosauro/open-loops/pull/39))

## 1.6.0 - 2026-07-05

### Docs
- Document --path and the bounded scan/worktree pools

### Features
- Add --path column to the inventory ([#31](https://github.com/carvalhosauro/open-loops/pull/31))

### Fixes
- Abbreviate home via component-aware strip_prefix ([#31](https://github.com/carvalhosauro/open-loops/pull/31))

### Performance
- Probe worktrees on the bounded pool ([#18](https://github.com/carvalhosauro/open-loops/pull/18))
- Bound scan fan-out with a worker pool ([#16](https://github.com/carvalhosauro/open-loops/pull/16))

## 1.5.0 - 2026-06-29

### Docs
- De-link references to consolidated (deleted) sources
- Remove ADRs/plans/specs/audit consolidated into docs/architecture
- Add architecture index and repoint references
- Address review fixes for build-ci-release
- Address review fixes for config-state
- Address review fixes for cli-output
- Address review fixes for cache-index
- Add build-ci-release domain doc
- Add cli-output domain doc
- Add config-state domain doc
- Add cache-index domain doc
- Address review fixes for resume-distill
- Add resume-distill domain doc
- Address review fixes for inventory-evidence
- Add inventory-evidence domain doc
- Address review fixes for query-engine
- Add query-engine domain doc
- Address review fixes for sessions-attribution
- Add sessions-attribution domain doc
- Address review fixes for discovery
- Add discovery domain doc
- Address review fixes for overview
- Add overview and scaffold docs/architecture
- Add architecture-domain-docs spec and plan
- Add code-hygiene review report (2026-06-29)
- SQLite index migration plan (#13 #14 #15 #17)
- Clarify prune semantics and fix ADR 0008 collision
- ADR 0008 SQLite index + features/config/changelog/roadmap

### Features
- Wire SQLite index live + refresh rebuild/prune (#13, #14, #15, #17)
- Bound mention probe + stable ranking + FTS index (#14, #15)
- Refs-fingerprint gate caches loops to skip rev-list ([#13](https://github.com/carvalhosauro/open-loops/pull/13))
- Cache git common-dir in index to skip dedup rev-parse ([#17](https://github.com/carvalhosauro/open-loops/pull/17))
- SQLite index scaffold with tolerant open and schema

### Fixes
- Migrate stale contentless sessions_fts to user_version 2
- Reindex session on size change to close same-second FTS staleness
- Fold worktrees/ mtime into refs fingerprint so worktree changes invalidate the gate

### Internals
- Bump anyhow to 1.0.103 (RUSTSEC-2026-0190)
- Single-source attr names and reset rule, why-comments
- Dedupe orchestration and make comments explain why

### Performance
- Parallelize gate git-probes to fix cold many-repos regression

### Features
- SQLite index at `~/.open-loops/index.db` (disposable cache; git stays the source of truth)
- Refs-fingerprint gate caches loops so warm scans skip `rev-list`/`for-each-ref` (#13)
- Cache `git --git-common-dir` per scanned path to skip the dedup `rev-parse` (#17)
- Probe session branch mentions via a bounded-tail FTS index instead of whole-file reads (#14)
- Stable session ranking that filters empty sessions before the `max_sessions` cut (#15)
- `loops refresh` rebuilds the index for scoped repos and prunes rows for repos gone from disk
- Self-healing index: a corrupt or deleted `index.db` is rebuilt transparently on the next run

### Docs
- ADR 0008 — SQLite disposable index (schema, refs-fingerprint gate, tolerant rebuild)
- `features.md` / `configuration.md`: index location, `loops refresh` rebuild, self-healing

## 1.4.0 - 2026-06-29

### Docs
- Contexts @ syntax and configuration (ADR 0003 phase 4)

### Features
- Persist active context in state.toml
- Wire @context resolution in list, resume, refresh
- Intersect multiple root filters in ScanPlan
- Resolve @context tokens into ScanPlan
- Add contexts table and default_context

### Fixes
- Treat empty LOOPS_CONTEXT as unset

### Internals
- @context scoping e2e

### Features
- `@` context scopes: `[contexts.X]` in config, active context in `state.toml` (ADR 0003 phase 4)

## 1.3.0 - 2026-06-27

### Docs
- Changelog for origin/HEAD fallback and prune labeling fixes
- Changelog for inventory cache fixes
- Inventory cache and refresh (ADR 0003 phase 3)
- Add ADR phase 3 inventory cache implementation plan

### Features
- Add loops refresh and --fresh flag
- Integrate inventory memo in heavy phase
- Add inventory_ttl_secs field (default 0 = SHA-only)
- Add SHA-validated ahead/behind memo store

### Fixes
- Single prune message and silence concurrent-remove ENOENT
- Label unreadable inventory distinctly when pruning
- Fall back to main/master on stale origin/HEAD
- Per-process tmp filename and global-GC prune doc
- Scope loops refresh to the repos the query matches
- Prune orphans only when repo path is gone

### Internals
- Reuse repo_with_feature and cover in-scope corrupt rewrite
- Cover origin/HEAD honour and stale fallback
- Refresh branch-filter scoping, no-match, worktree sharing, concurrency
- Extract scan_with_inventory to dedup scan setup
- Extract INVENTORY_EXT const and scope path_for_hash
- Cover refresh scoping, orphan prune, and cache hit/fresh
- Inventory cache write-through

### Performance
- Resolve default ref once; skip memo on blank base sha

### Features
- `inventory.rs`: SHA-validated ahead/behind memo store at `~/.open-loops/inventory/<fnv64hex>.json`
- `loops --fresh`: bypass inventory memo and recompute ahead/behind from git
- `loops refresh [query]`: full reindex with lazy orphan cleanup
- `inventory_ttl_secs` config option (default 0 = SHA-only validation)
- Write-through on every scan including filtered queries (`loops api`)
- Atomic inventory writes (tmp + rename) for concurrent-safe operation

### Fixes
- `loops refresh <term>` now scopes the reindex to the repos the query matches;
  bare terms and `branch:`/`key:`/`idle:`/`ahead:`/`behind:` filters were ignored,
  so every repo was reindexed (only `repo:`/`root:` were honored)
- Inventory tmp file is now named per-process, avoiding a rename race (and its
  spurious ENOENT warnings) when two `loops` runs write the same repo concurrently
- A stale/`--single-branch` `origin/HEAD` pointing at a branch with no local ref
  no longer hides the whole repo; default-branch detection falls back to main/master
- `loops refresh` now reports an unreadable (corrupt/empty) inventory file as
  `unreadable` rather than mislabeling it `orphan` when reclaiming it

### Internals
- `ScanOptions` struct threads inventory context through scanner
- `scan()` returns inventory updates for CLI-side write-through
- FNV-1a 64-bit hex hash of common-dir path (no new crate dependency)

## 1.2.0 - 2026-06-26

### Docs
- Add WAVE 2/3 implementation plan and honest ROADMAP status
- ADR 0007 and release-plz workflow documentation

### Features
- Automate version bump and crates.io publish via release-plz
- Bundle completions and man page in release archives
- Generate shell completions and man page at compile time
- Wire ScanPlan push-down and light/heavy git split

### Fixes
- Add rust toolchain to release-plz and exclude docs from Jekyll
- Mirror cliff.toml into release-plz and gate dist-artifacts copy
- Skip dist-artifacts staging during cargo publish verify
- Resolve cli_command module path from cli.rs
- Canonical path match for root: filter on Windows
- Tighten root: filter matching to avoid path false positives
- Address review — root tilde-expand, common-dir repo filter

### Internals
- Update Cargo.lock for clap_mangen build dependency
- Add crates.io metadata and release profile
- Extract shared clap command for build.rs
- Apply cargo fmt
- Reuse repo_name from dedup for repo: push-down
- Add case-insensitive repo filter tests and ensure no matches yield empty results

### CI
- Cross-OS test matrix (ubuntu, macos arm64, windows) with `fail-fast: false`
- Dedicated MSRV job on Rust 1.89 (`cargo check --locked --all-targets`)
- `--locked` on clippy, test, msrv, and coverage jobs
- `Swatinem/rust-cache` on compile jobs; global CI env (`RUSTFLAGS: "-D warnings"`, etc.)
- `cargo-deny` supply-chain audit (advisories non-blocking; licenses/bans/sources blocking)
- SHA-pinned GitHub Actions; `concurrency` with `cancel-in-progress`
- Dependabot for Cargo and GitHub Actions (weekly, grouped)

### Docs
- Add ADR 0006 for CI, MSRV, and cross-OS testing
- Add CI badges to README (CI, crates.io, MSRV, license)
- Add Cursor Cloud setup notes to AGENTS.md
- Add implementation plan for scanner bare+worktree discovery (Fase A)
- Add ADR 0005 for git-based repo discovery

### Features
- Wire ScanPlan push-down and light/heavy git split in scanner
- Attribute AI sessions to the branch's worktree
- Derive repo_name from git common-dir
- Resolve git common-dir via rev-parse
- Detect repos via .git file and bare probe
- Deduplicate repo candidates by git common-dir
- Add configurable scan_depth (default 4)
- Name repos from git common-dir in open_loops
- Pass scan_depth through scan and merge discovery warnings
- Thread scan_depth from config through scan and worktrees

### Internals
- Share git worktree --porcelain parser between scanner and worktrees
- Update CHANGELOG and add roadmap and specifications
- Bump version to 1.0.0 for v1.0.0 tag
- Add bare and bare+worktree git fixtures
- Cover bare, worktree, and normal discovery
- List loops in bare+worktree layout
## v1.0.0 - 2026-06-25

### Docs
- Rewrite README with progress lines and audit flow
- Add ADR 0003 for query engine and inventory cache
- Add Phase 2 evidence snapshot and retention policy
- Sharpen query engine cache model and canonical key
- Add query engine phase 1 implementation plan
- Document loops query filtering and root aliases
- Clarify split_cmp expect invariants
- Document the breaking 3-segment key migration

### Features
- Add ScanPlan, AttrFilter, Candidate types
- Parse bare terms and substring attributes
- Parse idle/ahead/behind comparators and durations
- Handle ignored tags, reserve contexts/reports/stale
- Evaluate ScanPlan against candidate loops
- Add root aliases and collision-checked label resolution
- Add root_label and 3-segment canonical key
- Key distill cache by root_label to avoid name collisions
- Filter the inventory by query
- Resolve loops via the query parser, resume includes ignored
## v0.1.0 - 2026-06-24

### Docs
- Add open-loops MVP design spec
- Add DX section to spec (pre-commit, coverage gates, justfile)
- Add open-source posture section to spec
- Add documentation requirements section to spec
- Add phase 3 (multi-harness support) to spec roadmap
- Add MVP implementation plan
- Add execution status snapshot for session resume
- Correct status — task 6 implemented, reviews pending
- Add readme, agent map, user docs, adrs and github templates
- Document release flow
- Spec for worktree inventory, EN-first CLI, completions
- Implementation plan for worktree inventory + EN migration
- English test names in worktree plan
- English/minimal comments rule and quality flow tests in plan
- English docs, worktrees + completions, error-language rule

### Features
- Add config store with toml persistence
- Add persistent ignore list for dead loops
- Add git shell-out helper and default branch detection
- Discover repos and unmerged branches with context helpers
- Add session source trait and claude code adapter
- Add distillation cache keyed by branch and head sha
- Build evidence prompt and run configurable llm command
- Render inventory table sorted by staleness
- Wire list, init, ignore and resume commands
- Add completions command for shell autocomplete
- Add Worktree model and deterministic verdict
- Enumerate and classify repo worktrees
- Render worktree inventory table with cleanup commands
- Add worktrees command (alias wt)
- Add resume dry-run, confidence score, and v0.1.0 release prep

### Fixes
- Update repository links in documentation to reflect new ownership
- Address quality review (no expect, no toctou, docs)
- Strict origin prefix strip and stronger error assertion
- Warn on malformed for-each-ref lines instead of silently dropping
- Tolerate broken pipe when llm exits before reading stdin
- Exclude default branch from merged set

### Internals
- Add rust-best-practices and rust-testing skills
- Scaffold rust crate with dual MIT/Apache-2.0 license
- Add justfile, lefthook hooks and ci with 70% coverage gate
- Add e2e flow covering init, list, resume, cache and ignore
- Add cargo-dist release pipeline and git-cliff changelog
- Add hygiene review report and mark mvp status complete
- Add 10 coverage gap tests and audit report
- Add worktree helper to testutil
- Migrate CLI output, errors, comments, and test names to English
- English test strings in claude_code session tests
- Produce resume output in English
- Add quality-focused worktree and completion flow tests
- English comment in session tail reader
- English test names and comments in tests/cli.rs
