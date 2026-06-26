//! Config persisted at <base>/config.toml.
//! The base path comes from outside (main resolves OPEN_LOOPS_HOME or ~/.open-loops)
//! so tests can inject a tempdir — nothing here reads environment variables.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Directories where git repositories are searched.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Optional per-root label override, keyed by the canonical root path.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
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
    /// Maximum directory depth (from each root) to search for git repositories.
    #[serde(default = "default_scan_depth")]
    pub scan_depth: usize,
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

fn default_scan_depth() -> usize {
    4
}

impl Default for Config {
    fn default() -> Self {
        Self {
            roots: vec![],
            aliases: BTreeMap::new(),
            llm_command: default_llm_command(),
            sessions_dir: default_sessions_dir(),
            max_sessions: default_max_sessions(),
            max_session_kb: default_max_session_kb(),
            scan_depth: default_scan_depth(),
        }
    }
}

impl Config {
    /// Resolves a stable label per root (alias, else basename). Errors when two
    /// roots resolve to the same label and no alias disambiguates them.
    pub fn resolve_labels(&self) -> Result<Vec<(std::path::PathBuf, String)>> {
        let mut out: Vec<(std::path::PathBuf, String)> = Vec::new();
        for root in &self.roots {
            let label = self
                .aliases
                .get(&root.to_string_lossy().into_owned())
                .cloned()
                .unwrap_or_else(|| {
                    root.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.to_string_lossy().into_owned())
                });
            if let Some((other, _)) = out.iter().find(|(_, l)| *l == label) {
                anyhow::bail!(
                    "roots {} and {} share label '{label}'; set an alias in config.toml",
                    other.display(),
                    root.display()
                );
            }
            out.push((root.clone(), label));
        }
        Ok(out)
    }

    /// Subset of configured roots matching `plan.root_filter`. Path values are
    /// tilde-expanded and canonicalized, then matched as a prefix against roots
    /// (ADR 0003). Label/path substring match is a fallback for short aliases.
    pub fn resolve_scan_roots(
        &self,
        plan: &crate::query::ScanPlan,
    ) -> Result<Vec<std::path::PathBuf>> {
        let labels = self.resolve_labels()?;
        let Some(filter) = &plan.root_filter else {
            return Ok(self.roots.clone());
        };
        let mut prefix = expand_tilde(filter);
        if prefix.exists() {
            if let Ok(canon) = std::fs::canonicalize(&prefix) {
                prefix = canon;
            }
        }
        let needle = filter.to_lowercase();
        Ok(labels
            .into_iter()
            .filter(|(root, label)| root_matches_filter(root, label, filter, &needle, &prefix))
            .map(|(root, _)| root)
            .collect())
    }
}

/// True when `root`/`label` match a `root:` filter (ADR 0003).
fn root_matches_filter(
    root: &std::path::Path,
    label: &str,
    filter: &str,
    needle: &str,
    prefix: &std::path::Path,
) -> bool {
    // Canonical path prefix after tilde-expand (e.g. root:~/work).
    if root.starts_with(prefix) {
        return true;
    }
    // Alias shortcut (e.g. root:w) — exact label, not substring (avoids "w" ⊂ "personal").
    if label.eq_ignore_ascii_case(filter) {
        return true;
    }
    // Path component / suffix (e.g. root:personal) — basename only, not the full temp path.
    if root
        .file_name()
        .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(needle))
    {
        return true;
    }
    let root_str = root.to_string_lossy().to_lowercase();
    root_str.ends_with(needle) || root_str.contains(&format!("/{needle}"))
}

/// Expands a leading `~` to the home directory (ADR `root:` filter).
fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| std::path::PathBuf::from(path))
    } else if path == "~" {
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(path))
    } else {
        std::path::PathBuf::from(path)
    }
}

/// Label of the configured root that owns `repo` (longest path prefix wins).
pub fn label_for_repo(labels: &[(std::path::PathBuf, String)], repo: &std::path::Path) -> String {
    labels
        .iter()
        .filter(|(root, _)| repo.starts_with(root))
        .max_by_key(|(root, _)| root.as_os_str().len())
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| {
            repo.parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
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
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
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

    #[test]
    fn resolve_labels_uses_basename_then_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let work = tmp.path().join("work");
        let personal = tmp.path().join("personal");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&personal).unwrap();
        let mut cfg = Config {
            roots: vec![work.clone(), personal.clone()],
            ..Config::default()
        };
        let labels = cfg.resolve_labels().unwrap();
        assert!(labels.contains(&(work.clone(), "work".to_string())));
        // alias overrides basename
        cfg.aliases
            .insert(personal.to_string_lossy().into_owned(), "p".into());
        let labels = cfg.resolve_labels().unwrap();
        assert!(labels.contains(&(personal.clone(), "p".to_string())));
        let _ = store;
    }

    #[test]
    fn config_scan_depth_defaults_to_four() {
        let cfg = Config::default();
        assert_eq!(cfg.scan_depth, 4);
    }

    #[test]
    fn config_scan_depth_roundtrips_from_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let cfg = Config {
            scan_depth: 6,
            ..Config::default()
        };
        store.save(&cfg).unwrap();
        assert_eq!(store.load().unwrap().scan_depth, 6);
    }

    #[test]
    fn resolve_scan_roots_filters_by_label_and_path() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        let personal = tmp.path().join("personal");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&personal).unwrap();
        let mut cfg = Config {
            roots: vec![work.clone(), personal.clone()],
            ..Config::default()
        };
        cfg.aliases
            .insert(work.to_string_lossy().into_owned(), "w".into());

        let all = cfg
            .resolve_scan_roots(&crate::query::ScanPlan::default())
            .unwrap();
        assert_eq!(all.len(), 2);

        let by_label = cfg
            .resolve_scan_roots(&crate::query::parse("root:w").unwrap())
            .unwrap();
        assert_eq!(by_label, vec![work.clone()]);

        let by_path = cfg
            .resolve_scan_roots(&crate::query::parse("root:personal").unwrap())
            .unwrap();
        assert_eq!(by_path, vec![personal]);
    }

    #[test]
    fn resolve_scan_roots_short_alias_does_not_match_unrelated_path_noise() {
        // Temp dirs like `.tmp02Wc68` contain the letter 'w'; a loose full-path
        // substring match must not pull in every root when filtering root:w.
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        let personal = tmp.path().join("personal");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&personal).unwrap();
        let mut cfg = Config {
            roots: vec![work.clone(), personal.clone()],
            ..Config::default()
        };
        cfg.aliases
            .insert(work.to_string_lossy().into_owned(), "w".into());

        let matched = cfg
            .resolve_scan_roots(&crate::query::parse("root:w").unwrap())
            .unwrap();
        assert_eq!(matched, vec![work]);
    }

    #[test]
    fn resolve_scan_roots_tilde_expands_to_prefix_match() {
        let home = dirs::home_dir().expect("home dir");
        let tmp = tempfile::tempdir().unwrap();
        let work = home.join(format!(
            ".loops-test-{}",
            tmp.path().file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&work).unwrap();
        let cfg = Config {
            roots: vec![work.clone()],
            ..Config::default()
        };
        let filter = format!("~/{}", work.file_name().unwrap().to_string_lossy());
        let matched = cfg
            .resolve_scan_roots(&crate::query::parse(&format!("root:{filter}")).unwrap())
            .unwrap();
        assert_eq!(matched, vec![work.clone()]);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn resolve_labels_errors_on_collision_without_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a/repos");
        let b = tmp.path().join("b/repos");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let cfg = Config {
            roots: vec![a, b],
            ..Config::default()
        };
        let err = cfg.resolve_labels().unwrap_err().to_string();
        assert!(err.contains("share label"), "got: {err}");
        assert!(err.contains("alias"), "got: {err}");
    }
}
