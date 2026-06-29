# Plan — SQLite index migration (open-loops)

Migrate the live scan/session caches to a SQLite index. As a byproduct this
resolves issues **#13** (serial rev-list per branch), **#14** (whole-file
session mention read), **#15** (unstable session sort + truncate-before-filter),
and **#17** (serial git_common_dir dedup). Issues #16 (thread fan-out) and #31
(LOOP path column) are explicitly OUT OF SCOPE.

Base branch: `feat/sqlite-index` off `main` @ `b224913`.
Worktree: `/home/gustavo/repo/me/open-loops-sqlite`.

## Context

- Rust CLI, binary `loops`, MSRV **1.89** (enforced by a dedicated CI `msrv` job).
- Git is the **source of truth**; the index is a **disposable cache**, always
  rebuildable from git via `loops refresh`.
- Today: `inventory/<hash>.json` memoises only ahead/behind; the light git phase
  (for-each-ref, --merged, worktree list, default branch) runs **every**
  invocation. Sessions are read whole for the mention probe.
- `src/lib.rs` exports every module with `pub mod` → public-API items are not
  `dead_code`-warned even when internally unused. So `inventory.rs` stays as-is.

## Global Constraints (binding — copy verbatim into every reviewer prompt)

1. **NEVER remove or weaken an existing test.** Tests are additive only. You may
   add tests; you may NOT delete a test or delete assertions. If a correctness
   fix genuinely changes an existing assertion's expected value, flag it
   explicitly in your report with justification — do not silently edit it.
2. **All acceptance criteria must pass before a task is done**, including the
   full existing suite: `cargo test` green, `cargo clippy --all-targets -- -D
   warnings` clean, `cargo fmt --check` clean.
3. **Git is the source of truth; the SQLite index is a disposable cache.** Any
   corruption, open failure, or integrity failure of the index → delete + rebuild,
   never abort the command. Mirror the tolerant pattern in `inventory.rs`
   (corrupt JSON → warn + ignore).
4. **MSRV 1.89.** Every new dependency must build on rustc 1.89. `rusqlite` MUST
   use the `bundled` feature (no system libsqlite3 dependency).
5. **Additive wiring, no breaking signatures where avoidable.** When a function
   needs index access, prefer an `Option<&Index>` parameter (or a new function
   that the old one delegates to with `None`) so existing call sites and existing
   tests compile and pass unchanged. `None` index → exact current behavior.
6. **Error messages in English, actionable.** Tolerant parsing: a bad row/line
   is skipped with a warning, never an abort.
7. **Docs are part of Definition of Done** for the task that ships user-visible
   behavior (Task 5): `docs/features.md`, `docs/configuration.md`, `CHANGELOG.md`.
8. **Conventional Commits.** Each task commits its own work with a scoped message.

## Architecture (locked decisions — do not re-litigate)

- New module `src/index/` (`pub mod index;` in `lib.rs`). Single struct `Index`
  wrapping one `rusqlite::Connection` at `<base>/index.db`, opened in **WAL**
  mode. `<base>` is `OPEN_LOOPS_HOME` or `~/.open-loops` (already resolved by
  callers; the module never reads env vars — same rule as `config.rs`).
- One schema migration to `user_version = 1` creating ALL tables up front (they
  are cohesive); later tasks only wire read/write logic.
- `inventory.rs` is **kept intact** (module + all tests). `common_dir_hash` is
  reused by the index. The JSON `InventoryStore` remains public API (its tests
  stay meaningful); the live scan path stops depending on it but it is not
  deleted.
- Index access is threaded as `Option<&Index>`. `None` everywhere = today's
  behavior, so every pre-existing test passes without modification.

## Schema (created in Task 1, `user_version = 1`)

```sql
CREATE TABLE repos (
  common_dir_hash TEXT PRIMARY KEY,
  path            TEXT NOT NULL UNIQUE,   -- the scanned repo dir (container/worktree entry)
  common_dir      TEXT NOT NULL,          -- absolute --git-common-dir
  default_branch  TEXT,
  default_sha     TEXT,
  refs_fingerprint INTEGER,               -- cheap staleness signal (see Task 3)
  last_indexed    INTEGER                 -- unix secs
);

CREATE TABLE loops (
  common_dir_hash TEXT NOT NULL,
  branch          TEXT NOT NULL,
  head_sha        TEXT NOT NULL,
  base_sha        TEXT NOT NULL,
  ahead           INTEGER,
  behind          INTEGER,
  last_commit     INTEGER NOT NULL,       -- unix secs
  worktree_path   TEXT NOT NULL,
  PRIMARY KEY (common_dir_hash, branch)
);

CREATE TABLE sessions (
  path        TEXT PRIMARY KEY,
  repo_path   TEXT NOT NULL,
  mtime       INTEGER NOT NULL,
  size        INTEGER NOT NULL
);

CREATE VIRTUAL TABLE sessions_fts USING fts5(
  text,
  path UNINDEXED,
  content=''                              -- contentless external-content-free FTS
);
```

`sessions.path` ↔ `sessions_fts` correlated by storing `path` as an unindexed
FTS column (contentless FTS keeps the DB small; we only need MATCH + rowid→path).

---

## Task 1 — rusqlite dependency + index scaffold (schema + tolerant open)

**Goal:** add the dependency and a working, tolerant `Index` with the full schema.

**Files:** `Cargo.toml`, `Cargo.lock`, `src/lib.rs` (add `pub mod index;`),
`src/index/mod.rs` (new).

**Steps:**
1. Add to `Cargo.toml` `[dependencies]`:
   `rusqlite = { version = "0.32", features = ["bundled"] }`
   Then `cargo build` to confirm it compiles on rustc 1.89 and to update
   `Cargo.lock`. If 0.32 fails MSRV, pin the highest minor that builds on 1.89
   and note the version chosen in your report.
2. `src/index/mod.rs`: `pub struct Index { conn: rusqlite::Connection }`.
   - `pub fn open(base: &Path) -> Index` — opens `<base>/index.db`. Sets
     `PRAGMA journal_mode=WAL;` and `PRAGMA foreign_keys=ON;`. Runs migrations
     (read `PRAGMA user_version`; if `< 1`, create the schema above and set
     `user_version = 1`). On ANY failure to open/migrate/integrity-check
     (`PRAGMA integrity_check`), delete the db file (and `-wal`/`-shm`
     siblings) and recreate from scratch. `open` returns an `Index` (a usable
     in-memory fallback is acceptable if disk truly fails — never panic, never
     abort the program). Print a one-line `warning: ...` on rebuild, matching
     the inventory.rs tone.
   - `pub fn open_in_memory() -> Index` for tests (`:memory:`), runs the same
     migration.
3. Register `pub mod index;` in `src/lib.rs` (keep modules alphabetic-ish where
   they already are).

**Acceptance criteria:**
- `cargo build` succeeds on rustc 1.89 (the `rust-toolchain.toml` pins 1.89).
- `cargo clippy --all-targets -- -D warnings` clean; `cargo fmt --check` clean.
- All pre-existing tests still pass (`cargo test`), none removed.
- New tests (in `src/index/mod.rs`): (a) `open` on a fresh dir creates the db and
  all four tables exist (query `sqlite_master`); (b) reopening is idempotent
  (`user_version` stays 1, no error); (c) a corrupt db file (write garbage bytes
  to `<base>/index.db`) → `open` rebuilds it and the tables exist; (d)
  `open_in_memory` has the schema.
- Report the chosen `rusqlite` version and confirm `cargo deny check` (licenses
  + advisories) passes, or note if `cargo deny` is unavailable.

---

## Task 2 — repos table + common_dir cache (resolves #17)

**Goal:** cache `--git-common-dir` per scanned path so repeated scans skip the
serial `git rev-parse` in dedup.

**Files:** `src/index/mod.rs` (methods), `src/scanner.rs` (wire into dedup).

**Interfaces (from Task 1):** `Index` exists with the `repos` table.

**Steps:**
1. `Index` methods:
   - `pub fn cached_common_dir(&self, path: &Path) -> Option<(String, PathBuf)>`
     → `(common_dir_hash, common_dir)` for a known `path`.
   - `pub fn put_repo_common_dir(&self, path, common_dir_hash, common_dir)` —
     upsert into `repos` (leave default_branch/sha/fingerprint NULL here; Task 3
     fills them). Use `INSERT ... ON CONFLICT(path) DO UPDATE`.
2. In `scanner.rs`: add `find_repos_cached(roots, scan_depth, index: Option<&Index>)`
   and make the existing `find_repos(roots, scan_depth)` delegate with `None`.
   Plumb the index into `dedup_candidates` (add an `Option<&Index>` param to a
   new `dedup_candidates_cached`, keep the old one delegating with `None`).
   In dedup: for each candidate, first try `index.cached_common_dir(path)`; on
   hit use it and SKIP the `git_common_dir` shell-out; on miss call
   `git_common_dir`, then `put_repo_common_dir`.
3. The cache key is the scanned `path`; a path's common-dir is stable, so no
   staleness check is needed here (a moved/deleted repo is pruned in Task 5).

**Acceptance criteria:**
- Existing `scanner.rs` tests untouched and green (they call the `None` path).
- New tests: (a) `find_repos_cached` with a fresh in-memory index populates
  `repos.common_dir`; (b) a second `dedup_candidates_cached` call on the same
  candidate reads the cached common_dir — assert reuse by pre-seeding the index
  with a SENTINEL hash for that path and asserting the returned `RepoCandidate`
  carries the sentinel (proving git was not consulted); (c) dedup correctness
  (N worktrees → 1 repo) is preserved on the cached path.
- clippy/fmt clean; whole suite green.

---

## Task 3 — loops table + refs-fingerprint gate (resolves #13)

**Goal:** when a repo's refs are unchanged since last index, return cached loops
and skip `for-each-ref`, `branch --merged`, and per-branch `rev-list` entirely.

**Files:** `src/index/mod.rs` (methods + fingerprint helper), `src/scanner.rs`
(`open_loops` gate).

**Steps:**
1. `pub fn refs_fingerprint(common_dir: &Path) -> i64` in `scanner.rs` (or index):
   combine the mtimes (unix secs) of `<common_dir>/HEAD`,
   `<common_dir>/packed-refs` (if present), and the `<common_dir>/refs` directory
   tree's newest entry. A simple, robust choice: the MAX of those mtimes. Missing
   files contribute 0. Document that `git gc` repacking may bump the fingerprint
   without a semantic change — acceptable (just recomputes).
2. `Index` methods:
   - `pub fn cached_loops(&self, hash, refs_fp, default_sha) -> Option<Vec<LoopRow>>`
     — returns rows ONLY when the stored `repos.refs_fingerprint == refs_fp` AND
     `repos.default_sha == default_sha`; else `None`.
   - `pub fn put_loops(&self, hash, path, common_dir, default_branch, default_sha,
     refs_fp, rows: &[LoopRow])` — upsert `repos` (fingerprint/default) and
     replace the repo's `loops` rows in a single transaction.
   - `LoopRow { branch, head_sha, base_sha, ahead, behind, last_commit:
     DateTime<Utc>, worktree_path: PathBuf }`.
3. `open_loops`: add index access **without touching `ScanOptions`** (adding a
   borrow there would force a `ScanOptions<'a>` lifetime and break every call
   site). Instead add `pub fn open_loops_indexed(repo, root_label, opts,
   index: Option<&Index>)` and make the existing `open_loops(repo, root_label,
   opts)` delegate with `None`. Likewise add `scan_indexed(..., index:
   Option<&Index>)` and make `scan` delegate with `None`. Existing tests call the
   old names → unchanged.
   - When `index` is `Some` AND not `opts.fresh`: compute `refs_fingerprint`,
     resolve `default_branch_and_sha` (cheap), then `cached_loops`. On hit →
     build `OpenLoop`s from the rows and RETURN, skipping for-each-ref/merged/
     rev-list. On miss → run the existing logic, then `put_loops` write-through.
   - When `index` is `None` → exactly today's code path (existing tests green).
   - `opts.fresh` bypasses the gate (still write-through so the next call hits).

**Acceptance criteria:**
- Existing `scanner.rs` tests untouched and green (None path + inventory tests).
- New tests: (a) second `open_loops` with an index and unchanged refs makes ZERO
  `rev-list` calls — assert via a real git repo where you record loops, then
  corrupt/remove the ability to compute (e.g. assert returned ahead/behind equal
  the cached values even after you rewrite history in a way that WOULD change
  them, proving git was not re-run); (b) advancing HEAD (new commit on a branch)
  changes the fingerprint → recompute → fresh ahead/behind; (c) changing the
  default branch SHA invalidates the cache; (d) `fresh: true` bypasses the gate;
  (e) a brand-new branch appears after caching → fingerprint changes → it shows.
- clippy/fmt clean; whole suite green.

---

## Task 4 — sessions FTS index + stable ranking (resolves #14 and #15)

**Goal:** (a) #14 — stop reading whole session files for the mention probe; index
bounded text once and probe via FTS. (b) #15 — stable, correct ranking that never
drops the real session.

**Files:** `src/index/mod.rs` (session methods), `src/sessions/claude_code.rs`
(excerpts), possibly `src/sessions/mod.rs` (adapter ctor).

**Steps:**
1. **#15 fix in the existing in-memory path (applies with or without index):** in
   `claude_code.rs::excerpts`, make the sort a STABLE tie-break
   (`mtime DESC, then path ASC`) and FILTER OUT sessions with no extractable text
   BEFORE applying `truncate(max_sessions)`. Prefer sessions that
   `mentions_branch`/`in_window` when truncating. Do NOT delete the existing
   tests; add new ones for the tie-break and the empty-before-truncate behavior.
   If an existing test's expectation legitimately changes, flag it in the report.
2. **#14 FTS path (used when an index is available):** do NOT add a borrowed
   `Index` field to the adapter (it would force a lifetime on the adapter type).
   Keep the `SessionSource::excerpts` trait signature unchanged; add a free
   function or inherent method `excerpts_indexed(&self, ..., index:
   Option<&Index>)` that the trait `excerpts` delegates to with `None`, and have
   the caller (cli/distill) invoke the indexed variant with `Some(&index)`.
   `Index` methods:
   - `pub fn upsert_session(&self, path, repo_path, mtime, size, text)` —
     reindex only when `(path, mtime)` differs from the stored row; the `text` is
     the bounded tail (`max_session_kb`), read ONCE.
   - `pub fn session_mentions(&self, repo_path, branch) -> HashSet<PathBuf>` —
     `sessions_fts MATCH` scoped to a repo_path; no file reads.
   - When an index is present, the mention probe uses `session_mentions` instead
     of `read_to_string(path).contains(branch)`; the bounded tail is read once at
     index time and reused.
   - When no index is present, the current (now #15-fixed) file path is used.
3. Keep `max_session_kb` as the read bound everywhere (no unbounded reads).

**Acceptance criteria:**
- Existing `claude_code.rs` tests green (only additive changes; any changed
  assertion explicitly justified in the report).
- New tests: (a) two session files with the SAME mtime → selection is
  deterministic across runs (stable tie-break); (b) a session with no extractable
  text is excluded BEFORE the `max_sessions` limit, so a real session further
  down is not dropped (the #15 regression); (c) with an in-memory index, the
  mention probe finds a branch-mentioning session via FTS without a full-file
  read (assert by indexing a bounded tail and matching); (d) re-running excerpts
  does not re-index an unchanged (path,mtime) session.
- clippy/fmt clean; whole suite green.

---

## Task 5 — wire CLI, refresh/prune, docs + ADR (end-to-end)

**Goal:** make the index live for real commands, rebuildable and pruned, and
document it.

**Files:** `src/cli.rs`, `src/index/mod.rs` (prune/rebuild), `docs/features.md`,
`docs/configuration.md`, `CHANGELOG.md`, `docs/decisions/0008-sqlite-index.md`
(new ADR), `ROADMAP.md`.

**Steps:**
1. `cli.rs`: open the `Index` once per command (via `Index::open(base)`), thread
   `Some(&index)` into the scan (`run_list`, `run_resume`, `run_refresh`) and into
   the Claude Code session adapter. Keep the JSON inventory write-through OR drop
   it from the live path (your choice) — but do not delete `inventory.rs`.
2. `loops refresh`: rebuild the index for the scoped repos (clear + repopulate
   loops; reindex sessions lazily on next resume). Add `Index` prune:
   `pub fn prune_missing_repos(&self)` deletes `repos` (and their `loops`) whose
   `path`/`common_dir` no longer exists on disk — same orphan semantics as
   `inventory::prune_orphans`.
3. ADR `0008-sqlite-index.md`: decision (git = truth, sqlite = disposable index),
   schema, fingerprint-gate rationale, tolerant rebuild, what it resolves
   (#13/#14/#15/#17), and what it does NOT (#16/#31). Add a ROADMAP entry. Update
   `features.md`/`configuration.md` (index location, `loops refresh` rebuild,
   self-healing on corruption) and `CHANGELOG.md`.

**Acceptance criteria:**
- `cargo build --release` works; `loops`, `loops resume --dry-run`,
  `loops resume` (with `llm_command=cat`), `loops refresh`, `loops worktrees`,
  `loops completions` all run end-to-end without error against a real tempdir
  repo.
- `bash scripts/stress/regress.sh` PASSES (behavior contract intact).
- Deleting the index db between two `loops` runs is transparent (self-heals).
- clippy/fmt clean; whole suite green; coverage gate (≥70%, core target 85%) not
  regressed.
- Docs + ADR + CHANGELOG + ROADMAP updated.

---

## Task 6 — final benchmark + extensive flow verification (proves the win)

**Goal:** prove improvement vs the baseline and exercise many real flows.

**Files:** none in `src/` (verification + a comparison note under
`.superpowers/sdd/` or `docs/`).

**Steps:**
1. Rebuild release; run `bash scripts/stress/bench.sh --out
   .superpowers/sdd/bench-after-default.txt` and
   `bash scripts/stress/bench.sh --heavy --out
   .superpowers/sdd/bench-after-heavy.txt`. Note: the gate's win shows on the
   SECOND scan (warm cache); the bench times a single invocation — to show the
   warm-cache effect, run the relevant scenario twice with a persistent
   `OPEN_LOOPS_HOME` and record both cold and warm wall times.
2. Compare to `bench-baseline-*.txt`. Expected: many-branches warm scan markedly
   faster (#13 gate), big-session max_rss lower (#14 bounded/FTS). Record the
   delta table.
3. Extensive flows (use a fresh `OPEN_LOOPS_HOME` tempdir, real git repos via the
   gen_fixtures or hand-built): init; list; bare term / `repo:` / `branch:` /
   `idle:` / `ahead:` / `behind:` filters; `@context` set/`@none`; `+ignored`;
   `resume --dry-run` and `resume` with `llm_command=cat`; `ignore`; `worktrees`;
   `refresh`; `completions`; `--fresh`; index-deletion self-heal; two concurrent
   `loops` processes (WAL). Record each as OK/observation.

**Acceptance criteria:**
- A before/after delta table showing improvement on at least the #13
  (many-branches warm) and #14 (big-session RSS) scenarios, with no regression on
  the others beyond noise.
- All extensive flows behave as expected (documented OK/observations).
- `regress.sh` green; full `cargo test` green; clippy/fmt clean.
