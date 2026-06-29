//! SQLite-backed disposable index for cached scan and session data.
//!
//! The index lives at `<base>/index.db` (WAL mode). It is a **cache** only —
//! git is the source of truth. Any open/migrate/integrity failure deletes the
//! db file (and its `-wal`/`-shm` siblings) and recreates it from scratch.
//! The program never panics or aborts on index failure.
//!
//! Schema is set to `user_version = 1` after the initial migration. Callers
//! in later tasks wire read/write logic on top of the tables created here.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;

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
}
