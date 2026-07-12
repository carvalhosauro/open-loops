//! Distillation cache at <base>/cache/<repo>/<branch>@<head-sha>.md.
//! Keying by the HEAD SHA makes the cache self-invalidate when the branch advances.
use crate::error::CacheError;
use crate::scanner::OpenLoop;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, CacheError>;

/// Distillation cache persisted to disk.
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// Creates a `Cache` whose files live under `base/cache/`.
    pub fn new(base: &Path) -> Self {
        Self {
            dir: base.join("cache"),
        }
    }

    fn path(&self, lp: &OpenLoop) -> PathBuf {
        // branches contain '/', which cannot appear in a file name
        let branch = lp.branch.replace('/', "__");
        self.dir
            .join(&lp.root_label)
            .join(&lp.repo_name)
            .join(format!("{branch}@{}.md", lp.head_sha))
    }

    /// Returns the cached content for `lp`, or `None` if it does not exist.
    pub fn get(&self, lp: &OpenLoop) -> Option<String> {
        std::fs::read_to_string(self.path(lp)).ok()
    }

    /// Persists `content` as the distillation of `lp`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the directories cannot be created or the file cannot be written.
    pub fn put(&self, lp: &OpenLoop, content: &str) -> Result<()> {
        let path = self.path(lp);
        // `path` is always `self.dir/<label>/<repo>/<file>.md`, so it always has
        // a parent.
        let parent = path.parent().expect("cache path always has a parent");
        std::fs::create_dir_all(parent)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fake_loop(sha: &str) -> OpenLoop {
        OpenLoop {
            root_label: "work".into(),
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: sha.into(),
            last_commit: Utc::now(),
            ahead: Some(1),
            behind: Some(0),
        }
    }

    #[test]
    fn miss_then_put_then_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let lp = fake_loop("abc123");
        assert!(cache.get(&lp).is_none());
        cache.put(&lp, "distilled context").unwrap();
        assert_eq!(cache.get(&lp).unwrap(), "distilled context");
    }

    #[test]
    fn new_head_self_invalidates() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        cache.put(&fake_loop("old-sha"), "old").unwrap();
        assert!(cache.get(&fake_loop("new-sha")).is_none());
    }

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
}
