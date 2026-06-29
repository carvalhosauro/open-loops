# ADR 0008: SQLite index — disposable cache for scan + session data

Date: 2026-06-29 · Status: accepted

## Context

`loops` shells out to git on every invocation. The light phase (`for-each-ref`,
`branch --merged`, default-branch resolution, `git worktree list`) and the heavy
phase (`rev-list` for ahead/behind) run for **every** repo on **every** call.
Session matching (`loops resume`) read whole session files to probe for a branch
mention. The JSON inventory (ADR 0003 phase 3) memoised only ahead/behind by
`(branch, head_sha, default_sha)`; the rest re-ran each time.

This left four concrete latency/IO problems open:

- **#13** — serial per-branch `rev-list` even when nothing changed since last scan.
- **#14** — whole-file read of each session just to test for a branch mention.
- **#15** — unstable session sort and truncate-before-filter, which could drop the
  real session below a `max_sessions` cut.
- **#17** — serial `git rev-parse --git-common-dir` per candidate during dedup.

Git is — and remains — the **source of truth**. Any cache we add must be safe to
delete at any moment and rebuild from git, and must never be able to abort a
command.

## Decision

Add a single SQLite database at `<base>/index.db` (`<base>` = `OPEN_LOOPS_HOME`
or `~/.open-loops`), wrapped by one `Index` struct (`src/index/mod.rs`). The
index is a **disposable cache**, not a store of record:

- **git = truth, sqlite = disposable index.** Every value in the db is derivable
  from git. The db can be deleted between runs; the next run rebuilds whatever it
  needs as a byproduct of scanning.
- **Tolerant by construction.** Opening, migrating, or any read/write that fails
  degrades to a recompute and prints a one-line `warning:` — it never aborts. On
  open/migrate/integrity failure the db file (and its `-wal`/`-shm` siblings) is
  deleted and recreated; if disk truly fails, an in-memory db is used for that one
  run.
- **`Option<&Index>` threading.** Index access is an optional parameter through
  the scan and session layers (`scan_indexed`, `open_loops_indexed`,
  `excerpts_indexed`). `None` reproduces the pre-index behaviour byte-for-byte, so
  every pre-existing test passes unchanged. The CLI passes `Some(&index)` live.
- **The JSON inventory (`inventory.rs`) is kept.** Its module and tests stay; it
  remains the `Option<&Index> == None` memo. On the live path the SQLite
  refs-fingerprint gate supersedes it for warm hits (see Consequences).

### Schema (`user_version = 2`)

```sql
CREATE TABLE repos (
  common_dir_hash  TEXT PRIMARY KEY,   -- FNV-1a of the absolute --git-common-dir
  path             TEXT NOT NULL UNIQUE,-- the scanned dir (container / worktree)
  common_dir       TEXT NOT NULL,       -- absolute --git-common-dir
  default_branch   TEXT,
  default_sha      TEXT,
  refs_fingerprint INTEGER,             -- cheap staleness signal (see gate)
  last_indexed     INTEGER              -- unix secs
);

CREATE TABLE loops (
  common_dir_hash TEXT NOT NULL,
  branch          TEXT NOT NULL,
  head_sha        TEXT NOT NULL,
  base_sha        TEXT NOT NULL,
  ahead           INTEGER,              -- NULL when scanned light-phase only
  behind          INTEGER,
  last_commit     INTEGER NOT NULL,     -- unix secs
  worktree_path   TEXT NOT NULL,
  PRIMARY KEY (common_dir_hash, branch)
);

CREATE TABLE sessions (
  path      TEXT PRIMARY KEY,
  repo_path TEXT NOT NULL,
  mtime     INTEGER NOT NULL,
  size      INTEGER NOT NULL
);

-- Contentful FTS5 (no content=''): owns its text rows so row-level
-- DELETE ... WHERE rowid=? works when a session file is reindexed.
CREATE VIRTUAL TABLE sessions_fts USING fts5(
  text,
  path UNINDEXED
);
```

`sessions.rowid` ↔ `sessions_fts.rowid` correlate the metadata row to its FTS
entry. The table is **contentful** (not `content=''`) so that the reindex path
can issue `DELETE … WHERE rowid = ?` when a session file changes; the per-row
text is a bounded tail, so storage cost is negligible.

**Migration history:**
- `user_version = 1` — initial schema; an intermediate build of the branch used
  `content=''` (contentless) for `sessions_fts`, which silently rejected
  row-level DELETEs needed for reindex.
- `user_version = 2` — FTS heal: a DB at version 1 has its `sessions_fts`
  dropped and recreated as a contentful table; `repos`, `loops`, and `sessions`
  rows are untouched. A fresh DB goes straight to version 2.

`rusqlite` is pinned with the `bundled` feature (no system `libsqlite3`
dependency), opened in **WAL** mode so two `loops` processes can read/write
concurrently without corrupting each other.

### The refs-fingerprint gate (#13)

The gate lets a warm scan skip `for-each-ref` + `branch --merged` +
per-branch `rev-list` entirely when a repo's refs are unchanged.

`refs_fingerprint(common_dir)` = the **MAX mtime (unix nanoseconds)** of:

- `<common_dir>/HEAD`
- `<common_dir>/packed-refs` (when present)
- the newest entry anywhere under `<common_dir>/refs/`
- the newest entry anywhere under `<common_dir>/worktrees/`

Missing files contribute 0. The cached loops are served only when **both**
`repos.refs_fingerprint == fingerprint` **and** `repos.default_sha == default_sha`
(the base hasn't moved). Otherwise it is a clean miss → recompute → write-through.

Rationale for the precise choices:

- **Nanosecond precision, not whole seconds.** A branch created or advanced in the
  same wall-clock second as the previous index write would leave a second-grained
  fingerprint unchanged and silently serve stale loops (e.g. a brand-new branch
  would not appear). Nanoseconds close that window. `i64` nanos-since-epoch
  overflow only in the year 2262; filesystems with second-only mtimes simply have
  a 0 sub-second part and degrade to whole-second behaviour — never worse.
- **Folding `worktrees/` into the fingerprint.** `git worktree add`/`remove`
  mutates `<common_dir>/worktrees/`; folding it in means any worktree change
  invalidates the gate, so fresh `worktree_path` values are always served.
- **Paired with `default_sha`.** A moved default branch invalidates the cache even
  if mtimes somehow collide.
- **`git gc`/repacking rewrites `packed-refs`**, bumping the fingerprint without a
  semantic change. Acceptable: it forces exactly one recompute, never stale data.

Because `rusqlite::Connection` is `Send` but `!Sync`, the gate is evaluated
**sequentially on the calling thread** (a hit is a few `stat`s + one SQLite read);
every repo that **misses** the gate is recomputed **in parallel** exactly as
before, then written through to the index on the calling thread.

### Sessions: bounded read + FTS probe (#14) and stable ranking (#15)

- **#14** — the mention probe no longer reads whole files. Each candidate
  session's **bounded tail** (`max_session_kb`) is read once and upserted into
  `sessions_fts` (skipping unchanged `(path, mtime)` rows); the probe is then a
  `sessions_fts MATCH` scoped by `repo_path`, with no file reads. When no index is
  present, the in-memory path also reads only the bounded tail (never the whole
  file).
- **#15** — ranking is a stable total order (`mentions_branch DESC, in_window
  DESC, modified DESC, path ASC`) and sessions with **no extractable text are
  filtered out BEFORE** the `max_sessions` truncate, so a real session is never
  dropped because an empty peer consumed its slot.

### Rebuild + prune (`loops refresh`)

`loops refresh` already scans with `fresh: true`, which bypasses the gate but
keeps the write-through — so the scoped repos' `repos`/`loops` rows are rebuilt on
that scan. After writing, `refresh` calls `Index::prune_missing_repos()`, which
deletes `repos` rows (and their dependent `loops`) whose scanned `path` **and**
`common_dir` are both gone from disk — **stricter** than
`inventory::prune_orphans`, which prunes on a single `repo_path` check (and also
reclaims unreadable files); the index requires BOTH the path AND the common_dir
to be gone, so a worktree-vs-bare-store split never drops a still-live repo. A
worktree dir removed while its shared bare store
survives is **not** an orphan (its branches are still real), so the row is kept.
Removal is self-healing: a returning repo is simply re-indexed on the next scan.

## What this resolves

| Issue | Before | After |
|---|---|---|
| **#13** | serial `rev-list` per branch every run | warm scan with unchanged refs serves cached loops, skips for-each-ref/merged/rev-list |
| **#14** | whole session file read for the mention probe | bounded tail indexed once into FTS; probe is `MATCH`, no file read |
| **#15** | unstable sort + truncate-before-filter could drop the real session | stable total order; empty sessions filtered before `max_sessions` |
| **#17** | serial `git rev-parse --git-common-dir` per candidate during dedup | common-dir cached in `repos`; cache hit skips the shell-out |

## What this does NOT resolve (out of scope)

- **#16** — thread fan-out / parallelism strategy of the scan itself. The gate
  reduces *work*, not the fan-out model; phase 2 still spawns one thread per
  gate-miss repo.
- **#31** — a `LOOP` path column in the `loops` output table. Purely presentational
  and untouched here.

These remain open and are tracked in the roadmap.

## Consequences

**Positive**

- Warm scans on many-branch repos skip the heavy git phase entirely (#13).
- `loops resume` no longer reads whole session files; big-session RSS drops (#14).
- Session selection is deterministic and never drops the real session (#15).
- Dedup skips a `git rev-parse` per known candidate (#17).
- WAL mode allows concurrent `loops` processes.
- The cache is genuinely disposable: corruption or deletion self-heals, and git
  stays the only source of truth.

**Negative / risks**

- A second on-disk cache alongside the JSON inventory. On the live path the
  SQLite gate supersedes the JSON memo for warm hits (the JSON inventory is now
  written through but only read on a gate miss); `inventory.rs` is retained as the
  documented `None`-index behaviour and for its still-meaningful tests.
- The fingerprint is mtime-based: `git gc` repacking forces one harmless recompute.
- `sessions_fts` indexes a bounded tail, so a branch mention only in the *head* of
  a very large session beyond `max_session_kb` is not matched — identical bound to
  the pre-index behaviour, by design.
- The Claude Code `.jsonl` format is still not a public API (spec risk 1); parsing
  stays tolerant (bad line → skip + warning).

## Relationship to prior ADRs

- **0001 / 0002** — pull model and git/LLM-via-shell-out are unchanged; the index
  only memoises what git already produces.
- **0003 (phase 3)** — the JSON inventory introduced there is kept; the SQLite gate
  is a strict superset of its ahead/behind memo on the live path.
- **0004** — the lazy orphan-prune pattern (prune on `refresh`, self-heal on
  return) is mirrored by `Index::prune_missing_repos`.
