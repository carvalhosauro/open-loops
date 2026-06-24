//! Config persisted at <base>/config.toml.
//! The base path comes from outside (main resolves OPEN_LOOPS_HOME or ~/.open-loops)
//! so tests can inject a tempdir — nothing here reads environment variables.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Directories where git repositories are searched.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Command that receives the prompt on stdin and returns the answer on stdout.
    #[serde(default = "default_llm_command")]
    pub llm_command: String,
    /// Claude Code sessions directory.
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: PathBuf,
    /// Maximum number of sessions used in distillation.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    /// KB read from the tail of each session.
    #[serde(default = "default_max_session_kb")]
    pub max_session_kb: u64,
}

fn default_llm_command() -> String {
    "claude -p".into()
}

fn default_sessions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/projects")
}

fn default_max_sessions() -> usize {
    3
}

fn default_max_session_kb() -> u64 {
    50
}

impl Default for Config {
    fn default() -> Self {
        Self {
            roots: vec![],
            llm_command: default_llm_command(),
            sessions_dir: default_sessions_dir(),
            max_sessions: default_max_sessions(),
            max_session_kb: default_max_session_kb(),
        }
    }
}

pub struct Store {
    base: PathBuf,
}

impl Store {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    pub fn config_path(&self) -> PathBuf {
        self.base.join("config.toml")
    }

    pub fn load(&self) -> Result<Config> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("invalid config.toml at {}", path.display()))
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        std::fs::create_dir_all(&self.base)
            .with_context(|| format!("creating {}", self.base.display()))?;
        std::fs::write(self.config_path(), toml::to_string_pretty(config)?)?;
        Ok(())
    }

    pub fn add_roots(&self, paths: &[PathBuf]) -> Result<Config> {
        let mut config = self.load()?;
        for p in paths {
            let abs = std::fs::canonicalize(p)
                .with_context(|| format!("nonexistent root: {}", p.display()))?;
            if !config.roots.contains(&abs) {
                config.roots.push(abs);
            }
        }
        self.save(&config)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_without_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let cfg = store.load().unwrap();
        assert!(cfg.roots.is_empty());
        assert_eq!(cfg.llm_command, "claude -p");
        assert_eq!(cfg.max_sessions, 3);
        assert_eq!(cfg.max_session_kb, 50);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let cfg = Config {
            llm_command: "cat".into(),
            ..Config::default()
        };
        store.save(&cfg).unwrap();
        assert_eq!(store.load().unwrap().llm_command, "cat");
    }

    #[test]
    fn add_roots_canonicalizes_and_deduplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let root = tmp.path().join("projects");
        std::fs::create_dir_all(&root).unwrap();
        store.add_roots(std::slice::from_ref(&root)).unwrap();
        let cfg = store.add_roots(std::slice::from_ref(&root)).unwrap();
        assert_eq!(cfg.roots.len(), 1);
        assert!(cfg.roots[0].is_absolute());
    }

    #[test]
    fn add_roots_fails_for_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let err = store
            .add_roots(&[tmp.path().join("does-not-exist")])
            .unwrap_err();
        assert!(err.to_string().contains("nonexistent root"));
    }
}
