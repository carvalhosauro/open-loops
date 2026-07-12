//! Loops discarded by the user ("not worth continuing").
//! Persisted at <base>/ignores.toml, keys in "repo/branch" format.
use crate::error::IgnoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, IgnoreError>;

#[derive(Debug, Default, Serialize, Deserialize)]
struct IgnoreFile {
    #[serde(default)]
    ignored: BTreeSet<String>,
}

pub struct Ignores {
    path: PathBuf,
    set: BTreeSet<String>,
}

impl Ignores {
    /// Loads the ignore list from `<base>/ignores.toml`.
    ///
    /// A missing file is treated as an empty list.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or is not
    /// valid TOML in the expected format.
    pub fn load(base: &Path) -> Result<Self> {
        let path = base.join("ignores.toml");
        let set = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                toml::from_str::<IgnoreFile>(&raw)
                    .map_err(|source| IgnoreError::InvalidToml {
                        path: path.clone(),
                        source: Box::new(source),
                    })?
                    .ignored
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(source) => {
                return Err(IgnoreError::ReadFailed {
                    path: path.clone(),
                    source,
                })
            }
        };
        Ok(Self { path, set })
    }

    /// Adds `key` to the ignore list and immediately persists it to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the base directory cannot be created or if writing
    /// the file fails.
    pub fn add(&mut self, key: &str) -> Result<()> {
        self.set.insert(key.to_string());
        // `self.path` is `<base>/ignores.toml`, so it always has a parent.
        let parent = self
            .path
            .parent()
            .expect("ignores path always has a parent");
        std::fs::create_dir_all(parent).map_err(|source| IgnoreError::CreateDirFailed {
            path: parent.to_path_buf(),
            source,
        })?;
        let file = IgnoreFile {
            ignored: self.set.clone(),
        };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?).map_err(|source| {
            IgnoreError::WriteFailed {
                path: self.path.clone(),
                source,
            }
        })?;
        Ok(())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_when_file_does_not_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let ig = Ignores::load(tmp.path()).unwrap();
        assert!(!ig.contains("repo/branch"));
    }

    #[test]
    fn add_persists_and_contains_finds() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ig = Ignores::load(tmp.path()).unwrap();
        ig.add("app/feat/x").unwrap();
        let reloaded = Ignores::load(tmp.path()).unwrap();
        assert!(reloaded.contains("app/feat/x"));
        assert!(!reloaded.contains("app/feat/y"));
    }
}
