//! Distillation cache at <base>/cache/<repo>/<branch>@<head-sha>.md.
//! Keying by the HEAD SHA makes the cache self-invalidate when the branch advances.
use crate::scanner::OpenLoop;
use anyhow::Result;
use std::path::{Path, PathBuf};

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
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("cache path has no parent directory"))?,
        )?;
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
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: sha.into(),
            last_commit: Utc::now(),
            ahead: 1,
            behind: 0,
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
}
