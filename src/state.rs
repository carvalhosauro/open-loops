//! Runtime state at `<base>/state.toml` (separate from declarative `config.toml`).
//! Holds the active `@context` chosen via the CLI.
use crate::error::StateError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, StateError>;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct StateFile {
    #[serde(default)]
    current_context: Option<String>,
}

pub struct State {
    path: PathBuf,
    current_context: Option<String>,
}

impl State {
    /// Loads `<base>/state.toml`. Missing file yields empty state.
    ///
    /// On first run, migrates `current_context` or legacy `default_context` from
    /// `config.toml` if present, then writes `state.toml`.
    pub fn load(base: &Path) -> Result<Self> {
        let path = base.join("state.toml");
        if path.exists() {
            let raw = std::fs::read_to_string(&path).map_err(|source| StateError::ReadFailed {
                path: path.clone(),
                source,
            })?;
            let file: StateFile =
                toml::from_str(&raw).map_err(|source| StateError::InvalidStateToml {
                    path: path.clone(),
                    source,
                })?;
            return Ok(Self {
                path,
                current_context: file.current_context,
            });
        }

        let mut state = Self {
            path,
            current_context: None,
        };
        if let Some(legacy) = legacy_context_from_config(base)? {
            state.current_context = Some(legacy);
            state.save()?;
        }
        Ok(state)
    }

    pub fn current_context(&self) -> Option<&str> {
        self.current_context.as_deref()
    }

    pub fn set_current_context(&mut self, name: Option<String>) -> Result<()> {
        if self.current_context == name {
            return Ok(());
        }
        self.current_context = name;
        self.save()
    }

    fn save(&self) -> Result<()> {
        let parent = self.path.parent().ok_or_else(|| StateError::NoParentDir {
            path: self.path.clone(),
        })?;
        std::fs::create_dir_all(parent)?;
        let file = StateFile {
            current_context: self.current_context.clone(),
        };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?)?;
        Ok(())
    }
}

fn legacy_context_from_config(base: &Path) -> Result<Option<String>> {
    let config_path = base.join("config.toml");
    let raw = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StateError::ReadFailed {
                path: config_path,
                source,
            })
        }
    };
    let table: toml::Value =
        toml::from_str(&raw).map_err(|source| StateError::InvalidConfigToml {
            path: config_path.clone(),
            source,
        })?;
    Ok(table
        .get("current_context")
        .or_else(|| table.get("default_context"))
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_when_file_missing_and_no_legacy_config() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State::load(tmp.path()).unwrap();
        assert_eq!(state.current_context(), None);
        assert!(!tmp.path().join("state.toml").exists());
    }

    #[test]
    fn set_persists_current_context() {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = State::load(tmp.path()).unwrap();
        state.set_current_context(Some("work".into())).unwrap();
        let loaded = State::load(tmp.path()).unwrap();
        assert_eq!(loaded.current_context(), Some("work"));
    }

    #[test]
    fn clear_removes_current_context() {
        let tmp = tempfile::tempdir().unwrap();
        let mut state = State::load(tmp.path()).unwrap();
        state.set_current_context(Some("work".into())).unwrap();
        state.set_current_context(None).unwrap();
        let loaded = State::load(tmp.path()).unwrap();
        assert_eq!(loaded.current_context(), None);
    }

    #[test]
    fn migrates_legacy_default_context_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.toml"),
            r#"
roots = []

default_context = "work"
"#,
        )
        .unwrap();
        let state = State::load(tmp.path()).unwrap();
        assert_eq!(state.current_context(), Some("work"));
        assert!(tmp.path().join("state.toml").exists());
    }
}
