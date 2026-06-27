# ADR Fase 3 — Inventory Cache + Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Memoize expensive `rev-list` ahead/behind per repo in `~/.open-loops/inventory/`, with SHA-validated reuse, write-through on every scan, `--fresh` bypass, and `loops refresh` full reindex + lazy orphan cleanup.

**Architecture:** New `src/inventory.rs` owns persistence (JSON per common-dir hash). `scanner::open_loops` gains optional `InventoryContext` (store + fresh flag + ttl); light phase always runs, heavy phase consults memo keyed by `(head_sha, ab_base_sha)` before spawning `rev-list`. CLI wires write-through after scan and adds `loops refresh [query]` + `--fresh` on list/resume.

**Tech Stack:** Rust 1.89, serde_json, existing scanner/config/cli layers. Stable FNV-1a hex hash of absolute common-dir (no new crypto dep).

**Spec:** [ADR 0003 phase 3](docs/decisions/0003-query-engine.md) · [ROADMAP](ROADMAP.md)

**Out of scope (phase 4):** `@ctx` context resolution on `loops refresh` — accept optional query with existing filters (`repo:`, `branch:`, etc.) only.

---

## Acceptance criteria (ROADMAP)

- [ ] `inventory.rs`: file per repo at `~/.open-loops/inventory/<hash-common-dir>.json`
- [ ] ahead/behind memo validated by `(head_sha, ab_base_sha)`
- [ ] write-through on every scan (including filtered `loops api`)
- [ ] atomic write (tmp + rename)
- [ ] `--fresh` ignores memo; `loops refresh [query]` full reindex
- [ ] `inventory_ttl_secs` in config (default 0 = SHA validation only)
- [ ] lazy orphan cleanup on `refresh` (ADR 0004 pattern)

---

## File map

| File | Responsibility |
|---|---|
| `src/inventory.rs` | Types, hash key, load/save, lookup, write-through, prune orphans |
| `src/scanner.rs` | Integrate memo in `open_loops`; thread `ScanOptions` |
| `src/config.rs` | `inventory_ttl_secs` field + default |
| `src/cli_command.rs` | `Refresh` subcommand; `--fresh` on default list + Resume |
| `src/cli.rs` | `run_refresh`, pass options to scan, write-through |
| `src/main.rs` | Route `Refresh` |
| `src/lib.rs` | `pub mod inventory` |
| `docs/configuration.md`, `docs/features.md`, `ROADMAP.md`, `CHANGELOG.md` | Docs |
| `tests/cli.rs` | E2E: cache hit skips redundant work observable via inventory file |

---

### Task 1: `inventory.rs` core

**Files:**
- Create: `src/inventory.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add module + types**

```rust
// InventoryEntry, InventoryFile { repo_path, indexed_at, loops: Vec<LoopMemo> }
// LoopMemo { branch, head_sha, ab_base_sha, ahead, behind }
// InventoryStore { dir: PathBuf }
// fn common_dir_hash(common_dir: &Path) -> String  // FNV-1a 64 → 16 hex chars
// fn path_for_hash(dir: &Path, hash: &str) -> PathBuf
```

- [ ] **Step 2: Load / save with atomic write**

- `load(hash) -> Option<InventoryFile>` — missing file → None; corrupt JSON → warn + None
- `save(hash, &InventoryFile)` — write `.{hash}.json.tmp` then `rename`
- On save collision: if loaded `repo_path` differs from incoming, warn and overwrite

- [ ] **Step 3: Lookup + TTL**

```rust
pub fn lookup_ahead_behind(
    file: &InventoryFile,
    branch: &str,
    head_sha: &str,
    ab_base_sha: &str,
    ttl_secs: u64,
    now: DateTime<Utc>,
) -> Option<(u32, u32)>
```

- TTL: if `ttl_secs > 0` and `now - indexed_at > ttl`, return None
- Else match branch entry where `head_sha` and `ab_base_sha` equal

- [ ] **Step 4: `prune_orphans(inventory_dir, active_common_dirs: &HashSet<String>)`**

- List `inventory/*.json`; for each file, if hash not in active set OR `repo_path` missing on disk → delete + eprintln warning

- [ ] **Step 5: Unit tests** (tempdir, fake JSON, atomic save, lookup validation, TTL, prune)

- [ ] **Step 6: Commit** `feat(inventory): add SHA-validated ahead/behind memo store`

---

### Task 2: Wire scanner

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Add `ScanOptions`**

```rust
pub struct ScanOptions {
    pub need_ahead_behind: bool,
    pub fresh: bool,
    pub inventory: Option<&InventoryStore>,
    pub inventory_ttl_secs: u64,
}
```

- [ ] **Step 2: Change `open_loops` signature**

`open_loops(repo, root_label, opts: &ScanOptions) -> Result<(Vec<OpenLoop>, Option<InventoryFile>)>`

- Always compute `default_sha` via `git rev-parse refs/heads/{default}`
- Light phase unchanged
- Heavy: if `opts.need_ahead_behind`:
  - load inventory by common_dir hash (unless `opts.fresh`)
  - for each branch, try lookup; on miss run `rev-list`, update memo vec
- Return updated `InventoryFile` for write-through (even when `need_ahead_behind` false, still return file with empty loops update? **No** — only return Some when heavy ran or file existed and light phase touched repo; simplest: always build/update InventoryFile when inventory store present and need_ahead_behind)

- [ ] **Step 3: Update `scan()` to pass `ScanOptions`, collect inventory files per repo**

Return `(Vec<OpenLoop>, Vec<String>, Vec<InventoryFile>)` or write-through inside scan — **prefer CLI write-through** after scan returns map `hash → InventoryFile`.

- [ ] **Step 4: Update existing scanner tests** for new signature

- [ ] **Step 5: Add test** `open_loops_reuses_inventory_when_shas_match` — two calls, second should not need rev-list if we could spy; instead assert inventory file written and second call with same shas reads memo (mock by checking file content + optional: count git calls is hard; verify loops have correct ahead/behind from file-only path by pre-seeding inventory file)

- [ ] **Step 6: Commit** `feat(scanner): integrate inventory memo in heavy phase`

---

### Task 3: Config + CLI surface

**Files:**
- Modify: `src/config.rs`, `src/cli_command.rs`, `src/main.rs`, `src/cli.rs`

- [ ] **Step 1: `inventory_ttl_secs: u64` default 0** in Config + tests

- [ ] **Step 2: CLI**

```rust
// Cli: #[arg(long)] fresh: bool  on default command
// Resume: #[arg(long)] fresh: bool
// Command::Refresh { query: Vec<String> }  // joins to query string
```

- [ ] **Step 3: `run_list` / `resolve_loop`**

Build `ScanOptions { need_ahead_behind, fresh, inventory: Some(...), ttl }`, after scan call `inventory.write_through(updates)`.

- [ ] **Step 4: `run_refresh`**

- Parse optional query like list
- Force `need_ahead_behind = true`, `fresh = true`
- Scan scope from query push-down
- Write-through all touched repos
- `inventory.prune_orphans(&hashes_from_scan)`
- Print `refreshed N repos` on stderr

- [ ] **Step 5: Commit** `feat(cli): add loops refresh and --fresh flag`

---

### Task 4: Docs + ROADMAP

**Files:**
- Modify: `docs/configuration.md`, `docs/features.md`, `ROADMAP.md`, `CHANGELOG.md`

- [ ] Document `inventory_ttl_secs`, `inventory/` state dir, `loops refresh`, `--fresh`
- [ ] Mark ROADMAP phase 3 items `[x]`
- [ ] CHANGELOG entry under Unreleased

- [ ] **Commit** `docs: inventory cache and refresh (ADR 0003 phase 3)`

---

### Task 5: E2E test

**Files:**
- Modify: `tests/cli.rs`

- [ ] **Test `inventory_write_through_on_list`**

1. init + repo with branch
2. `loops` → assert `inventory/*.json` exists with ahead/behind
3. `loops --fresh` → still works
4. `loops refresh` → stderr contains refreshed

- [ ] **Commit** `test(cli): inventory cache write-through`

---

### Task 6: Verification

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `just regress` (install just if needed, or `bash scripts/stress/regress.sh`)
