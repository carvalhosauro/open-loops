//! SHA-validated ahead/behind memo store.
//!
//! One JSON file per git common-dir lives at
//! `~/.open-loops/inventory/<fnv64hex>.json`. The heavy git phase
//! (`rev-list`) is memoised per `(branch, head_sha, ab_base_sha)` pair.
//! Reads are tolerant; writes are atomic (tmp → rename).
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const INVENTORY_EXT: &str = "json";

/// One memoised ahead/behind entry for a branch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopMemo {
    pub branch: String,
    /// HEAD SHA of the branch at the time of the computation.
    pub head_sha: String,
    /// HEAD SHA of the default branch at the time of the computation.
    pub ab_base_sha: String,
    pub ahead: u32,
    pub behind: u32,
}

/// Per-repo inventory file serialised to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryFile {
    /// Absolute path of the repo root used for identity confirmation.
    pub repo_path: PathBuf,
    /// Timestamp of the last write (used for TTL validation).
    pub indexed_at: DateTime<Utc>,
    /// Memoised entries, one per unmerged branch.
    pub loops: Vec<LoopMemo>,
}

/// Thin wrapper around the inventory directory.
#[derive(Debug, Clone)]
pub struct InventoryStore {
    /// Directory containing `<hash>.json` files.
    pub dir: PathBuf,
}

impl InventoryStore {
    /// Creates a store whose directory is `<base>/inventory`.
    pub fn new(base: &Path) -> Self {
        Self {
            dir: base.join("inventory"),
        }
    }

    /// Loads the inventory file for `hash`, or `None` when absent or corrupt.
    pub fn load(&self, hash: &str) -> Option<InventoryFile> {
        let path = path_for_hash(&self.dir, hash);
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<InventoryFile>(&raw) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!(
                    "warning: corrupt inventory file {}: {e:#}; ignoring",
                    path.display()
                );
                None
            }
        }
    }

    /// Atomically writes `file` to `<dir>/<hash>.json` via a tmp file + rename.
    pub fn save(&self, hash: &str, file: &InventoryFile) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating inventory dir {}", self.dir.display()))?;
        let final_path = path_for_hash(&self.dir, hash);
        let tmp_path = tmp_path_for_hash(&self.dir, hash);
        let json = serde_json::to_string_pretty(file).context("serialising inventory file")?;
        std::fs::write(&tmp_path, &json)
            .with_context(|| format!("writing tmp inventory {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, &final_path)
            .with_context(|| format!("renaming inventory tmp to {}", final_path.display()))?;
        Ok(())
    }

    /// Reclaims inventory files whose `repo_path` no longer exists on disk.
    ///
    /// This is a global garbage-collect, intentionally NOT scoped to the current
    /// refresh query: a repo gone from disk is an orphan regardless of which
    /// query triggered the refresh, so its stale memo is always removed. Repos
    /// that are merely outside the query but still present on disk are kept —
    /// they are not orphans. Removal is self-healing: a returning repo is simply
    /// recomputed on the next scan.
    ///
    /// Called lazily from `loops refresh` only (ADR 0004 pattern).
    pub fn prune_orphans(&self) -> Result<()> {
        if !self.dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&self.dir)
            .with_context(|| format!("reading inventory dir {}", self.dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != INVENTORY_EXT) {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Skip tmp files that lack a proper stem.
            if stem.starts_with('.') {
                continue;
            }
            // A loadable file whose repo is gone is an orphan. An unreadable
            // file (corrupt/empty) can't prove its repo exists, so reclaim it
            // too — but label it accurately instead of calling it an orphan.
            let reason = match self.load(&stem) {
                Some(f) if f.repo_path.exists() => continue,
                Some(_) => "orphan",
                None => "unreadable",
            };
            match std::fs::remove_file(&path) {
                Ok(()) => eprintln!("warning: removed {reason} inventory {}", path.display()),
                Err(e) => eprintln!(
                    "warning: failed to remove {reason} inventory {}: {e:#}",
                    path.display()
                ),
            }
        }
        Ok(())
    }
}

/// FNV-1a 64-bit hash of the absolute common-dir path, returned as 16 hex chars.
///
/// Using FNV-1a avoids adding a new crate dependency. The hash is stable across
/// processes as long as the path string representation is identical.
pub fn common_dir_hash(common_dir: &Path) -> String {
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET;
    for byte in common_dir.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// Returns the path for a given hash in `dir`.
pub(crate) fn path_for_hash(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("{hash}.{INVENTORY_EXT}"))
}

/// Per-process temporary path used by [`InventoryStore::save`].
///
/// The pid keeps the tmp name unique so two `loops` processes writing the same
/// repo never race on one tmp file. The atomic rename already guarantees a
/// reader never sees a partial file, but a *shared* tmp name made one writer's
/// rename fail with ENOENT after the other renamed it away. Extension stays
/// `tmp` (not `json`) so prune and listing skip it.
fn tmp_path_for_hash(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!(".{hash}.{}.json.tmp", std::process::id()))
}

/// Looks up the cached ahead/behind for a branch, validating SHA keys and TTL.
///
/// Returns `None` when:
/// - No matching entry exists.
/// - `ttl_secs > 0` and the file is older than the TTL.
pub fn lookup_ahead_behind(
    file: &InventoryFile,
    branch: &str,
    head_sha: &str,
    ab_base_sha: &str,
    ttl_secs: u64,
    now: DateTime<Utc>,
) -> Option<(u32, u32)> {
    if ttl_secs > 0 {
        let age_secs = now.signed_duration_since(file.indexed_at).num_seconds();
        if age_secs < 0 || age_secs as u64 > ttl_secs {
            return None;
        }
    }
    file.loops
        .iter()
        .find(|m| m.branch == branch && m.head_sha == head_sha && m.ab_base_sha == ab_base_sha)
        .map(|m| (m.ahead, m.behind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_file(repo_path: &str, memos: Vec<LoopMemo>) -> InventoryFile {
        InventoryFile {
            repo_path: PathBuf::from(repo_path),
            indexed_at: Utc::now(),
            loops: memos,
        }
    }

    fn make_memo(branch: &str, head: &str, base: &str, ahead: u32, behind: u32) -> LoopMemo {
        LoopMemo {
            branch: branch.to_string(),
            head_sha: head.to_string(),
            ab_base_sha: base.to_string(),
            ahead,
            behind,
        }
    }

    #[test]
    fn common_dir_hash_is_16_hex_chars() {
        let h = common_dir_hash(std::path::Path::new("/home/user/proj/.git"));
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn common_dir_hash_is_deterministic() {
        let p = std::path::Path::new("/home/user/proj/.git");
        assert_eq!(common_dir_hash(p), common_dir_hash(p));
    }

    #[test]
    fn common_dir_hash_differs_for_different_paths() {
        let a = common_dir_hash(std::path::Path::new("/a/.git"));
        let b = common_dir_hash(std::path::Path::new("/b/.git"));
        assert_ne!(a, b);
    }

    #[test]
    fn path_for_hash_joins_correctly() {
        let p = path_for_hash(std::path::Path::new("/inv"), "abc123");
        assert_eq!(p, PathBuf::from("/inv/abc123.json"));
    }

    #[test]
    fn lookup_returns_none_for_unknown_branch() {
        let file = make_file("/repo", vec![make_memo("main", "aaa", "bbb", 1, 0)]);
        let result = lookup_ahead_behind(&file, "feat/x", "aaa", "bbb", 0, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn lookup_returns_values_when_shas_match() {
        let file = make_file("/repo", vec![make_memo("feat/x", "head1", "base1", 3, 1)]);
        let result = lookup_ahead_behind(&file, "feat/x", "head1", "base1", 0, Utc::now());
        assert_eq!(result, Some((3, 1)));
    }

    #[test]
    fn lookup_returns_none_when_head_sha_changed() {
        let file = make_file("/repo", vec![make_memo("feat/x", "head1", "base1", 3, 1)]);
        let result = lookup_ahead_behind(&file, "feat/x", "head2", "base1", 0, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn lookup_returns_none_when_base_sha_changed() {
        let file = make_file("/repo", vec![make_memo("feat/x", "head1", "base1", 3, 1)]);
        let result = lookup_ahead_behind(&file, "feat/x", "head1", "base2", 0, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn lookup_respects_ttl_when_file_is_stale() {
        use chrono::Duration;
        let old_time = Utc::now() - Duration::seconds(200);
        let file = InventoryFile {
            repo_path: PathBuf::from("/repo"),
            indexed_at: old_time,
            loops: vec![make_memo("feat/x", "h", "b", 1, 0)],
        };
        // TTL 100s but file is 200s old → None
        let result = lookup_ahead_behind(&file, "feat/x", "h", "b", 100, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn lookup_returns_value_when_within_ttl() {
        use chrono::Duration;
        let recent = Utc::now() - Duration::seconds(50);
        let file = InventoryFile {
            repo_path: PathBuf::from("/repo"),
            indexed_at: recent,
            loops: vec![make_memo("feat/x", "h", "b", 1, 0)],
        };
        // TTL 100s, file is 50s old → hit
        let result = lookup_ahead_behind(&file, "feat/x", "h", "b", 100, Utc::now());
        assert_eq!(result, Some((1, 0)));
    }

    #[test]
    fn lookup_ignores_ttl_when_zero() {
        use chrono::Duration;
        let very_old = Utc::now() - Duration::days(365);
        let file = InventoryFile {
            repo_path: PathBuf::from("/repo"),
            indexed_at: very_old,
            loops: vec![make_memo("feat/x", "h", "b", 2, 3)],
        };
        // TTL 0 → SHA-only validation, always hit if SHAs match
        let result = lookup_ahead_behind(&file, "feat/x", "h", "b", 0, Utc::now());
        assert_eq!(result, Some((2, 3)));
    }

    #[test]
    fn store_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        let file = make_file("/repo", vec![make_memo("feat/x", "h1", "b1", 4, 2)]);
        let hash = "test0123456789ab";
        store.save(hash, &file).unwrap();

        let loaded = store.load(hash).unwrap();
        assert_eq!(loaded.repo_path, file.repo_path);
        assert_eq!(loaded.loops.len(), 1);
        assert_eq!(loaded.loops[0].ahead, 4);
        assert_eq!(loaded.loops[0].behind, 2);
    }

    #[test]
    fn store_load_returns_none_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        assert!(store.load("nonexistent0000000").is_none());
    }

    #[test]
    fn store_load_returns_none_for_corrupt_json() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        std::fs::create_dir_all(&store.dir).unwrap();
        let hash = "corrupt000000000";
        std::fs::write(path_for_hash(&store.dir, hash), b"{not json}").unwrap();
        assert!(store.load(hash).is_none());
    }

    #[test]
    fn store_save_is_atomic_via_tmp_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        let file = make_file("/repo", vec![]);
        let hash = "atomic0123456789";
        store.save(hash, &file).unwrap();

        // No tmp file (any pid suffix) should remain after a successful save.
        let leftover_tmp = std::fs::read_dir(&store.dir)
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "tmp"));
        assert!(!leftover_tmp, "tmp file should be renamed away");
        assert!(path_for_hash(&store.dir, hash).exists());
    }

    #[test]
    fn save_tmp_name_is_unique_per_process() {
        let p = tmp_path_for_hash(std::path::Path::new("/inv"), "abc123");
        let name = p.file_name().unwrap().to_string_lossy();
        // pid keeps two concurrent writers off the same tmp file (BUG-2 fix).
        assert!(name.contains(&std::process::id().to_string()));
        assert!(name.starts_with(".abc123."));
        assert!(name.ends_with(".json.tmp"));
        // Extension is `tmp`, so prune/listing (which key on `json`) skip it.
        assert_eq!(p.extension().unwrap(), "tmp");
    }

    #[test]
    fn lookup_returns_none_for_future_indexed_at() {
        use chrono::Duration;
        // Clock skew: indexed_at is in the future, so age is negative.
        let future = Utc::now() + Duration::seconds(100);
        let file = InventoryFile {
            repo_path: PathBuf::from("/repo"),
            indexed_at: future,
            loops: vec![make_memo("feat/x", "h", "b", 1, 0)],
        };
        let result = lookup_ahead_behind(&file, "feat/x", "h", "b", 50, Utc::now());
        assert!(result.is_none(), "negative age must be treated as a miss");
    }

    #[test]
    fn store_load_returns_none_for_zero_byte_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        std::fs::create_dir_all(&store.dir).unwrap();
        let hash = "zerobyte00000000";
        std::fs::write(path_for_hash(&store.dir, hash), b"").unwrap();
        assert!(store.load(hash).is_none());
    }

    #[test]
    fn store_load_tolerates_unknown_extra_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        std::fs::create_dir_all(&store.dir).unwrap();
        let hash = "extrafields00000";
        // Forward-compat: unknown top-level and per-memo fields are ignored.
        let raw = r#"{
            "repo_path": "/repo",
            "indexed_at": "2020-01-01T00:00:00Z",
            "future_field": 42,
            "loops": [
                {"branch":"feat/x","head_sha":"h","ab_base_sha":"b",
                 "ahead":1,"behind":2,"bogus":true}
            ]
        }"#;
        std::fs::write(path_for_hash(&store.dir, hash), raw).unwrap();
        let loaded = store.load(hash).expect("unknown fields must not fail load");
        assert_eq!(loaded.loops[0].ahead, 1);
        assert_eq!(loaded.loops[0].behind, 2);
    }

    #[test]
    fn store_load_returns_none_when_path_is_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        std::fs::create_dir_all(&store.dir).unwrap();
        let hash = "isadirectory0000";
        // A directory sitting where the JSON file would be must not panic.
        std::fs::create_dir(path_for_hash(&store.dir, hash)).unwrap();
        assert!(store.load(hash).is_none());
    }

    #[test]
    fn prune_orphans_skips_non_json_and_tmp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        std::fs::create_dir_all(&store.dir).unwrap();

        // An orphan JSON (repo_path missing) — must be removed.
        let orphan = make_file("/nonexistent/repo", vec![]);
        store.save("orphan0000000000", &orphan).unwrap();
        // Non-JSON and tmp files — must survive (not inventory files).
        let notes = store.dir.join("notes.txt");
        std::fs::write(&notes, b"keep me").unwrap();
        let leftover_tmp = store.dir.join(".something.json.tmp");
        std::fs::write(&leftover_tmp, b"in-flight").unwrap();

        store.prune_orphans().unwrap();

        assert!(!path_for_hash(&store.dir, "orphan0000000000").exists());
        assert!(notes.exists(), "non-json files must be left alone");
        assert!(leftover_tmp.exists(), "tmp files must be left alone");
    }

    #[test]
    fn prune_orphans_reclaims_unreadable_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        std::fs::create_dir_all(&store.dir).unwrap();
        // A corrupt file can't prove its repo exists, so prune reclaims it (it is
        // labelled "unreadable" rather than misreported as an "orphan").
        let hash = "corruptlive00000";
        std::fs::write(path_for_hash(&store.dir, hash), b"{ broken json").unwrap();

        store.prune_orphans().unwrap();

        assert!(!path_for_hash(&store.dir, hash).exists());
    }

    #[test]
    fn prune_orphans_removes_file_when_repo_path_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        let file = make_file("/nonexistent/repo/path", vec![]);
        let hash = "orphan0123456789";
        store.save(hash, &file).unwrap();

        store.prune_orphans().unwrap();

        assert!(!path_for_hash(&store.dir, hash).exists());
    }

    #[test]
    fn prune_orphans_keeps_file_when_repo_path_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let store = InventoryStore::new(tmp.path());
        let file = make_file(tmp.path().to_str().unwrap(), vec![]);
        let hash = "active0123456789";
        store.save(hash, &file).unwrap();

        store.prune_orphans().unwrap();

        assert!(path_for_hash(&store.dir, hash).exists());
    }
}
