//! SQLite-backed disposable index for cached scan and session data.
//!
//! The index lives at `<base>/index.db` (WAL mode). It is a **cache** only —
//! git is the source of truth. Any open/migrate/integrity failure deletes the
//! db file (and its `-wal`/`-shm` siblings) and recreates it from scratch.
//! The program never panics or aborts on index failure.
//!
//! Schema is set to `user_version = 1` after the initial migration. Callers
//! in later tasks wire read/write logic on top of the tables created here.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// One cached open-loop row for a repo, persisted in the `loops` table.
///
/// Mirrors the heavy-phase output of `scanner::open_loops` for a single
/// unmerged branch. `ahead`/`behind` are `None` when the cached scan ran
/// without `need_ahead_behind` (light phase only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopRow {
    pub branch: String,
    pub head_sha: String,
    pub base_sha: String,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub last_commit: DateTime<Utc>,
    pub worktree_path: PathBuf,
}

/// SQLite-backed cache index.
///
/// Wraps a single `Connection`. All public methods in this module treat index
/// errors as non-fatal: they warn to stderr and fall back gracefully, matching
/// the tolerant pattern used in `inventory.rs`.
pub struct Index {
    conn: Connection,
}

impl Index {
    /// Opens (or creates) the index at `<base>/index.db`.
    ///
    /// Behaviour on failure at any stage (open, WAL pragma, migration,
    /// integrity check):
    /// 1. Print `warning: …` to stderr.
    /// 2. Delete `index.db`, `index.db-wal`, `index.db-shm` from `base`.
    /// 3. Attempt to create a fresh db in the same location.
    /// 4. If that also fails, fall back to an in-memory db so the command
    ///    continues without an index — never panic, never abort.
    pub fn open(base: &Path) -> Self {
        let db_path = base.join("index.db");
        match Self::try_open_disk(&db_path) {
            Ok(index) => index,
            Err(e) => {
                eprintln!("warning: index open/migrate failed ({e:#}); rebuilding");
                Self::delete_db_files(base);
                match Self::try_open_disk(&db_path) {
                    Ok(index) => index,
                    Err(e2) => {
                        eprintln!(
                            "warning: index rebuild also failed ({e2:#}); \
                             falling back to in-memory index"
                        );
                        // In-memory fallback so the command still runs.
                        Self::open_in_memory()
                    }
                }
            }
        }
    }

    /// Opens an in-memory index for tests (same migration, no disk I/O).
    pub fn open_in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("in-memory SQLite must always open");
        let mut index = Self { conn };
        // In-memory: migration cannot fail; panic only here (test/fallback path).
        index
            .apply_pragmas()
            .expect("in-memory pragma must succeed");
        index
            .run_migrations()
            .expect("in-memory migration must succeed");
        index
    }

    // -------------------------------------------------------------------------
    // Public cache accessors (Task 2)
    // -------------------------------------------------------------------------

    /// Returns `(common_dir_hash, common_dir)` cached for `path`, or `None` on
    /// miss or any index error.
    pub fn cached_common_dir(&self, path: &Path) -> Option<(String, PathBuf)> {
        let path_str = path.to_string_lossy();
        match self.conn.query_row(
            "SELECT common_dir_hash, common_dir FROM repos WHERE path = ?1",
            rusqlite::params![path_str.as_ref()],
            |row| {
                let hash: String = row.get(0)?;
                let cd: String = row.get(1)?;
                Ok((hash, PathBuf::from(cd)))
            },
        ) {
            Ok(pair) => Some(pair),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => {
                eprintln!("warning: index cached_common_dir query failed: {e:#}");
                None
            }
        }
    }

    /// Upserts `(path, common_dir_hash, common_dir)` into `repos`, leaving
    /// the remaining columns (default_branch, default_sha, refs_fingerprint,
    /// last_indexed) NULL. On any index error, prints a warning and continues.
    pub fn put_repo_common_dir(&self, path: &Path, common_dir_hash: &str, common_dir: &Path) {
        let path_str = path.to_string_lossy();
        let cd_str = common_dir.to_string_lossy();
        if let Err(e) = self.conn.execute(
            "INSERT INTO repos (common_dir_hash, path, common_dir)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET
                 common_dir_hash = excluded.common_dir_hash,
                 common_dir      = excluded.common_dir",
            rusqlite::params![common_dir_hash, path_str.as_ref(), cd_str.as_ref()],
        ) {
            eprintln!("warning: index put_repo_common_dir failed: {e:#}");
        }
    }

    // -------------------------------------------------------------------------
    // Public cache accessors (Task 3): refs-fingerprint gate
    // -------------------------------------------------------------------------

    /// Returns the cached loops for `hash`, but ONLY when the stored repo row
    /// proves the cache is still valid:
    ///
    /// 1. `repos.refs_fingerprint == refs_fp` (refs haven't changed), AND
    /// 2. `repos.default_sha == default_sha` (the base hasn't moved).
    ///
    /// Returns `None` on any mismatch, on a missing/un-populated repo row, or on
    /// any index error. A NULL `default_sha` / `refs_fingerprint` (a repo row
    /// inserted by `put_repo_common_dir` but never `put_loops`'d) is a clean
    /// miss — no warning is emitted, since it is the normal pre-`put_loops` state.
    pub fn cached_loops(
        &self,
        hash: &str,
        refs_fp: i64,
        default_sha: &str,
    ) -> Option<Vec<LoopRow>> {
        // Read the gate columns. NULL columns map to `None` so an un-populated
        // repos row is a clean miss rather than a warning.
        let gate: Option<(i64, String)> = match self.conn.query_row(
            "SELECT refs_fingerprint, default_sha FROM repos WHERE common_dir_hash = ?1",
            rusqlite::params![hash],
            |row| {
                let fp: Option<i64> = row.get(0)?;
                let sha: Option<String> = row.get(1)?;
                Ok(fp.zip(sha))
            },
        ) {
            Ok(g) => g,
            Err(rusqlite::Error::QueryReturnedNoRows) => return None,
            Err(e) => {
                eprintln!("warning: index cached_loops gate query failed: {e:#}");
                return None;
            }
        };

        let (stored_fp, stored_sha) = gate?;
        if stored_fp != refs_fp || stored_sha != default_sha {
            return None;
        }

        // Gate passed: load the loop rows.
        let mut stmt = match self.conn.prepare(
            "SELECT branch, head_sha, base_sha, ahead, behind, last_commit, worktree_path
             FROM loops WHERE common_dir_hash = ?1 ORDER BY branch",
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: index cached_loops prepare failed: {e:#}");
                return None;
            }
        };
        let rows = stmt.query_map(rusqlite::params![hash], |row| {
            let branch: String = row.get(0)?;
            let head_sha: String = row.get(1)?;
            let base_sha: String = row.get(2)?;
            let ahead: Option<i64> = row.get(3)?;
            let behind: Option<i64> = row.get(4)?;
            let last_commit_secs: i64 = row.get(5)?;
            let worktree_path: String = row.get(6)?;
            Ok(LoopRow {
                branch,
                head_sha,
                base_sha,
                ahead: ahead.map(|v| v as u32),
                behind: behind.map(|v| v as u32),
                last_commit: Utc
                    .timestamp_opt(last_commit_secs, 0)
                    .single()
                    .unwrap_or_default(),
                worktree_path: PathBuf::from(worktree_path),
            })
        });
        let rows = match rows {
            Ok(mapped) => mapped.collect::<Result<Vec<_>, _>>(),
            Err(e) => {
                eprintln!("warning: index cached_loops query failed: {e:#}");
                return None;
            }
        };
        match rows {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("warning: index cached_loops row decode failed: {e:#}");
                None
            }
        }
    }

    /// Write-through for a completed scan of one repo: upserts the `repos` row
    /// (default branch/SHA, refs fingerprint, last_indexed) and REPLACES the
    /// repo's `loops` rows — all in a single transaction.
    ///
    /// On any index error, prints a warning and continues (git is the source of
    /// truth; the index is disposable).
    #[allow(clippy::too_many_arguments)]
    pub fn put_loops(
        &self,
        hash: &str,
        path: &Path,
        common_dir: &Path,
        default_branch: &str,
        default_sha: &str,
        refs_fp: i64,
        rows: &[LoopRow],
    ) {
        if let Err(e) = self.put_loops_tx(
            hash,
            path,
            common_dir,
            default_branch,
            default_sha,
            refs_fp,
            rows,
        ) {
            eprintln!("warning: index put_loops failed: {e:#}");
        }
    }

    /// Inner fallible body of [`Self::put_loops`], run inside one transaction.
    #[allow(clippy::too_many_arguments)]
    fn put_loops_tx(
        &self,
        hash: &str,
        path: &Path,
        common_dir: &Path,
        default_branch: &str,
        default_sha: &str,
        refs_fp: i64,
        rows: &[LoopRow],
    ) -> Result<(), rusqlite::Error> {
        let path_str = path.to_string_lossy();
        let cd_str = common_dir.to_string_lossy();
        let now = Utc::now().timestamp();

        self.conn.execute_batch("BEGIN")?;
        let res = (|| -> Result<(), rusqlite::Error> {
            // Upsert the repos row. Key on common_dir_hash (PK) so a row that
            // already exists from put_repo_common_dir is updated in place; also
            // resolve a possible path UNIQUE conflict the same way.
            self.conn.execute(
                "INSERT INTO repos
                     (common_dir_hash, path, common_dir, default_branch,
                      default_sha, refs_fingerprint, last_indexed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(common_dir_hash) DO UPDATE SET
                     path             = excluded.path,
                     common_dir       = excluded.common_dir,
                     default_branch   = excluded.default_branch,
                     default_sha      = excluded.default_sha,
                     refs_fingerprint = excluded.refs_fingerprint,
                     last_indexed     = excluded.last_indexed",
                rusqlite::params![
                    hash,
                    path_str.as_ref(),
                    cd_str.as_ref(),
                    default_branch,
                    default_sha,
                    refs_fp,
                    now,
                ],
            )?;

            // Replace the repo's loops rows: delete then re-insert.
            self.conn.execute(
                "DELETE FROM loops WHERE common_dir_hash = ?1",
                rusqlite::params![hash],
            )?;
            for row in rows {
                self.conn.execute(
                    "INSERT INTO loops
                         (common_dir_hash, branch, head_sha, base_sha,
                          ahead, behind, last_commit, worktree_path)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        hash,
                        row.branch,
                        row.head_sha,
                        row.base_sha,
                        row.ahead.map(i64::from),
                        row.behind.map(i64::from),
                        row.last_commit.timestamp(),
                        row.worktree_path.to_string_lossy().as_ref(),
                    ],
                )?;
            }
            Ok(())
        })();

        match res {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                // Best-effort rollback; report the original error.
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    /// Attempts to open the db at `path`, apply pragmas, run migrations, and
    /// verify integrity. Returns an error string on any failure.
    fn try_open_disk(db_path: &Path) -> Result<Self, anyhow::Error> {
        // Ensure the parent directory exists.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("creating index dir {}: {e}", parent.display()))?;
        }

        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| anyhow::anyhow!("opening {}: {e}", db_path.display()))?;

        let mut index = Self { conn };
        index.apply_pragmas()?;
        index.run_migrations()?;
        index.check_integrity()?;
        Ok(index)
    }

    /// Sets WAL mode and enables foreign keys.
    fn apply_pragmas(&mut self) -> Result<(), anyhow::Error> {
        self.conn
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| anyhow::anyhow!("applying pragmas: {e}"))
    }

    /// Reads `user_version`; if < 1, creates all tables and bumps to 1.
    fn run_migrations(&mut self) -> Result<(), anyhow::Error> {
        let version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| anyhow::anyhow!("reading user_version: {e}"))?;

        if version < 1 {
            self.create_schema_v1()?;
        }
        Ok(())
    }

    /// Creates all four tables and sets `user_version = 1`.
    ///
    /// Executed in a single `execute_batch` so it is atomic.
    fn create_schema_v1(&mut self) -> Result<(), anyhow::Error> {
        self.conn
            .execute_batch(
                "
                BEGIN;

                CREATE TABLE repos (
                    common_dir_hash TEXT PRIMARY KEY,
                    path            TEXT NOT NULL UNIQUE,
                    common_dir      TEXT NOT NULL,
                    default_branch  TEXT,
                    default_sha     TEXT,
                    refs_fingerprint INTEGER,
                    last_indexed    INTEGER
                );

                CREATE TABLE loops (
                    common_dir_hash TEXT NOT NULL,
                    branch          TEXT NOT NULL,
                    head_sha        TEXT NOT NULL,
                    base_sha        TEXT NOT NULL,
                    ahead           INTEGER,
                    behind          INTEGER,
                    last_commit     INTEGER NOT NULL,
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
                    content=''
                );

                PRAGMA user_version = 1;

                COMMIT;
                ",
            )
            .map_err(|e| anyhow::anyhow!("creating schema v1: {e}"))
    }

    /// Runs `PRAGMA integrity_check` and returns an error if it reports problems.
    fn check_integrity(&self) -> Result<(), anyhow::Error> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|e| anyhow::anyhow!("integrity_check query failed: {e}"))?;

        if result != "ok" {
            return Err(anyhow::anyhow!("integrity_check: {result}"));
        }
        Ok(())
    }

    /// Deletes `index.db`, `index.db-wal`, and `index.db-shm` from `base`.
    ///
    /// Missing files are silently ignored (may already be absent on a fresh dir).
    fn delete_db_files(base: &Path) {
        for suffix in &["index.db", "index.db-wal", "index.db-shm"] {
            let path = base.join(suffix);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    eprintln!("warning: failed to remove {}: {e:#}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Returns every table/virtual-table name present in the connection.
    fn get_tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table') ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn all_four_tables_present(tables: &[String]) -> bool {
        ["loops", "repos", "sessions", "sessions_fts"]
            .iter()
            .all(|t| tables.iter().any(|n| n == t))
    }

    fn user_version(conn: &Connection) -> i32 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    // -----------------------------------------------------------------------
    // (a) Fresh dir: open creates db with all four tables
    // -----------------------------------------------------------------------

    #[test]
    fn open_fresh_dir_creates_all_four_tables() {
        let tmp = TempDir::new().unwrap();
        let index = Index::open(tmp.path());
        let tables = get_tables(&index.conn);
        assert!(
            all_four_tables_present(&tables),
            "expected repos, loops, sessions, sessions_fts — got: {tables:?}"
        );
        assert_eq!(user_version(&index.conn), 1);
        assert!(tmp.path().join("index.db").exists());
    }

    // -----------------------------------------------------------------------
    // (b) Reopening is idempotent (user_version stays 1, no error)
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        {
            let _first = Index::open(tmp.path());
        }
        // Drop first connection, then reopen.
        let second = Index::open(tmp.path());
        assert_eq!(user_version(&second.conn), 1);
        let tables = get_tables(&second.conn);
        assert!(
            all_four_tables_present(&tables),
            "tables missing after reopen: {tables:?}"
        );
    }

    // -----------------------------------------------------------------------
    // (c) Corrupt db file → open rebuilds and tables exist
    // -----------------------------------------------------------------------

    #[test]
    fn corrupt_db_is_rebuilt() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("index.db");

        // Write garbage bytes where the db would be.
        std::fs::write(&db_path, b"not a sqlite database at all").unwrap();

        // open must recover, not panic.
        let index = Index::open(tmp.path());
        let tables = get_tables(&index.conn);
        assert!(
            all_four_tables_present(&tables),
            "tables missing after corrupt-rebuild: {tables:?}"
        );
        assert_eq!(user_version(&index.conn), 1);
    }

    // -----------------------------------------------------------------------
    // (d) open_in_memory has the schema
    // -----------------------------------------------------------------------

    #[test]
    fn open_in_memory_has_schema() {
        let index = Index::open_in_memory();
        let tables = get_tables(&index.conn);
        assert!(
            all_four_tables_present(&tables),
            "in-memory index missing tables: {tables:?}"
        );
        assert_eq!(user_version(&index.conn), 1);
    }

    // -----------------------------------------------------------------------
    // (e) cached_common_dir / put_repo_common_dir round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn put_then_get_common_dir() {
        let index = Index::open_in_memory();
        let path = std::path::Path::new("/home/u/project");
        let common_dir = std::path::Path::new("/home/u/project/.git");
        let hash = "aabbccddeeff0011";

        // Miss before insert.
        assert!(index.cached_common_dir(path).is_none());

        index.put_repo_common_dir(path, hash, common_dir);

        let (got_hash, got_cd) = index.cached_common_dir(path).expect("should hit after put");
        assert_eq!(got_hash, hash);
        assert_eq!(got_cd, common_dir);
    }

    #[test]
    fn put_is_idempotent_upsert() {
        let index = Index::open_in_memory();
        let path = std::path::Path::new("/home/u/project");
        let cd1 = std::path::Path::new("/home/u/project/.git");
        let cd2 = std::path::Path::new("/home/u/project/.bare");

        index.put_repo_common_dir(path, "hash1", cd1);
        index.put_repo_common_dir(path, "hash2", cd2);

        let (h, cd) = index.cached_common_dir(path).unwrap();
        assert_eq!(h, "hash2");
        assert_eq!(cd, cd2);
    }

    // -----------------------------------------------------------------------
    // (f) Task 3: put_loops / cached_loops refs-fingerprint gate
    // -----------------------------------------------------------------------

    fn sample_rows() -> Vec<LoopRow> {
        vec![
            LoopRow {
                branch: "feat/a".into(),
                head_sha: "a".repeat(40),
                base_sha: "d".repeat(40),
                ahead: Some(3),
                behind: Some(1),
                last_commit: Utc.timestamp_opt(1_700_000_000, 0).single().unwrap(),
                worktree_path: PathBuf::from("/wt/a"),
            },
            LoopRow {
                branch: "feat/b".into(),
                head_sha: "b".repeat(40),
                base_sha: "d".repeat(40),
                ahead: Some(7),
                behind: Some(0),
                last_commit: Utc.timestamp_opt(1_700_000_100, 0).single().unwrap(),
                worktree_path: PathBuf::from("/wt/b"),
            },
        ]
    }

    #[test]
    fn put_loops_then_cached_loops_round_trip_on_matching_gate() {
        let index = Index::open_in_memory();
        let hash = "deadbeef00000000";
        let default_sha = "d".repeat(40);
        let rows = sample_rows();

        // Miss before any write.
        assert!(index.cached_loops(hash, 42, &default_sha).is_none());

        index.put_loops(
            hash,
            std::path::Path::new("/repo"),
            std::path::Path::new("/repo/.git"),
            "main",
            &default_sha,
            42,
            &rows,
        );

        let got = index
            .cached_loops(hash, 42, &default_sha)
            .expect("matching fingerprint + default_sha must hit");
        assert_eq!(got, rows);
    }

    #[test]
    fn cached_loops_misses_on_fingerprint_mismatch() {
        let index = Index::open_in_memory();
        let hash = "deadbeef00000001";
        let default_sha = "d".repeat(40);
        index.put_loops(
            hash,
            std::path::Path::new("/repo"),
            std::path::Path::new("/repo/.git"),
            "main",
            &default_sha,
            42,
            &sample_rows(),
        );
        // Different fingerprint → miss.
        assert!(index.cached_loops(hash, 43, &default_sha).is_none());
        // Same fingerprint → hit.
        assert!(index.cached_loops(hash, 42, &default_sha).is_some());
    }

    #[test]
    fn cached_loops_misses_on_default_sha_mismatch() {
        let index = Index::open_in_memory();
        let hash = "deadbeef00000002";
        index.put_loops(
            hash,
            std::path::Path::new("/repo"),
            std::path::Path::new("/repo/.git"),
            "main",
            &"d".repeat(40),
            42,
            &sample_rows(),
        );
        // Same fingerprint but a different default_sha (base moved) → miss.
        assert!(index.cached_loops(hash, 42, &"e".repeat(40)).is_none());
    }

    #[test]
    fn cached_loops_unpopulated_repos_row_is_clean_miss() {
        let index = Index::open_in_memory();
        let path = std::path::Path::new("/repo");
        let cd = std::path::Path::new("/repo/.git");
        let hash = "deadbeef00000003";
        // Insert a repos row WITHOUT loops data (NULL default_sha / fingerprint).
        index.put_repo_common_dir(path, hash, cd);
        // Must be a clean miss (no panic, no spurious behaviour).
        assert!(index.cached_loops(hash, 0, "").is_none());
        assert!(index.cached_loops(hash, 42, &"d".repeat(40)).is_none());
    }

    #[test]
    fn put_loops_replaces_previous_rows_in_one_transaction() {
        let index = Index::open_in_memory();
        let hash = "deadbeef00000004";
        let default_sha = "d".repeat(40);
        index.put_loops(
            hash,
            std::path::Path::new("/repo"),
            std::path::Path::new("/repo/.git"),
            "main",
            &default_sha,
            42,
            &sample_rows(), // 2 rows
        );
        // Re-write with a single row and a new fingerprint.
        let one = vec![LoopRow {
            branch: "feat/only".into(),
            head_sha: "c".repeat(40),
            base_sha: default_sha.clone(),
            ahead: Some(1),
            behind: Some(0),
            last_commit: Utc.timestamp_opt(1_700_000_500, 0).single().unwrap(),
            worktree_path: PathBuf::from("/wt/only"),
        }];
        index.put_loops(
            hash,
            std::path::Path::new("/repo"),
            std::path::Path::new("/repo/.git"),
            "main",
            &default_sha,
            99,
            &one,
        );
        let got = index.cached_loops(hash, 99, &default_sha).unwrap();
        assert_eq!(got, one, "old rows must be replaced, not appended");
    }

    #[test]
    fn put_loops_upgrades_existing_common_dir_row() {
        // A repos row created by put_repo_common_dir (Task 2, NULL gate columns)
        // must be upgraded in place by put_loops — same common_dir_hash PK — so
        // the gate hits afterwards and no duplicate row is created.
        let index = Index::open_in_memory();
        let path = std::path::Path::new("/repo");
        let cd = std::path::Path::new("/repo/.git");
        let hash = "deadbeef00000005";
        index.put_repo_common_dir(path, hash, cd);
        // Pre-upgrade: repos row exists but gate columns are NULL → clean miss.
        let default_sha = "d".repeat(40);
        assert!(index.cached_loops(hash, 7, &default_sha).is_none());

        index.put_loops(hash, path, cd, "main", &default_sha, 7, &sample_rows());

        // Exactly one repos row for this hash, now populated → gate hits.
        let repo_count: i64 = index
            .conn
            .query_row(
                "SELECT COUNT(*) FROM repos WHERE common_dir_hash = ?1",
                rusqlite::params![hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            repo_count, 1,
            "put_loops must upgrade in place, not duplicate"
        );
        assert!(index.cached_loops(hash, 7, &default_sha).is_some());
    }

    #[test]
    fn cached_loops_preserves_null_ahead_behind() {
        // Light-phase rows (no ahead/behind) round-trip as None.
        let index = Index::open_in_memory();
        let hash = "deadbeef00000006";
        let default_sha = "d".repeat(40);
        let rows = vec![LoopRow {
            branch: "feat/light".into(),
            head_sha: "a".repeat(40),
            base_sha: default_sha.clone(),
            ahead: None,
            behind: None,
            last_commit: Utc.timestamp_opt(1_700_000_000, 0).single().unwrap(),
            worktree_path: PathBuf::from("/wt/light"),
        }];
        index.put_loops(
            hash,
            std::path::Path::new("/repo"),
            std::path::Path::new("/repo/.git"),
            "main",
            &default_sha,
            1,
            &rows,
        );
        let got = index.cached_loops(hash, 1, &default_sha).unwrap();
        assert_eq!(got[0].ahead, None);
        assert_eq!(got[0].behind, None);
    }
}
