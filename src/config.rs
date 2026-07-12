//! Config persisted at <base>/config.toml.
//! The base path comes from outside (main resolves OPEN_LOOPS_HOME or ~/.open-loops)
//! so tests can inject a tempdir — nothing here reads environment variables.
use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextDef {
    pub filter: String,
}

/// A saved query invoked ad-hoc with `:name`. Same shape as a context but not
/// persisted to `state.toml`; MVP filters cannot embed `@context` or `:report`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportDef {
    pub filter: String,
}

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
    /// Seconds before an inventory entry is considered expired regardless of SHA
    /// match. 0 (default) means SHA-only validation with no time-based expiry.
    #[serde(default)]
    pub inventory_ttl_secs: u64,
    /// Named query scopes (`@name` in queries) mapped to filter strings.
    #[serde(default)]
    pub contexts: BTreeMap<String, ContextDef>,
    /// Saved queries invoked ad-hoc with `:name`, mapped to filter strings.
    #[serde(default)]
    pub reports: BTreeMap<String, ReportDef>,
    /// Idle threshold the `+stale` query shortcut expands to (query duration
    /// syntax, e.g. `14d`): `+stale` is sugar for `idle:>{stale_threshold}`.
    #[serde(default = "default_stale_threshold")]
    pub stale_threshold: String,
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

fn default_stale_threshold() -> String {
    "14d".into()
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
            inventory_ttl_secs: 0,
            contexts: BTreeMap::new(),
            reports: BTreeMap::new(),
            stale_threshold: default_stale_threshold(),
        }
    }
}

impl Config {
    /// Returns the filter string for a named context.
    pub fn context_filter(&self, name: &str) -> Result<&str> {
        self.contexts
            .get(name)
            .map(|c| c.filter.as_str())
            .ok_or_else(|| ConfigError::UnknownContext {
                name: name.to_string(),
            })
    }

    /// Returns the filter string for a named report (`:name`).
    pub fn report_filter(&self, name: &str) -> Result<&str> {
        self.reports
            .get(name)
            .map(|r| r.filter.as_str())
            .ok_or_else(|| ConfigError::UnknownReport {
                name: name.to_string(),
            })
    }

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
                return Err(ConfigError::LabelCollision {
                    root_a: other.clone(),
                    root_b: root.clone(),
                    label,
                });
            }
            out.push((root.clone(), label));
        }
        Ok(out)
    }

    /// Subset of configured roots matching `plan.root_filters`. Path values are
    /// tilde-expanded and canonicalized, then matched as a prefix against roots
    /// (ADR 0003). Label/path substring match is a fallback for short aliases.
    /// Multiple filters are ANDed (intersection).
    pub fn resolve_scan_roots(
        &self,
        plan: &crate::query::ScanPlan,
    ) -> Result<Vec<std::path::PathBuf>> {
        if plan.root_filters.is_empty() {
            return Ok(self.roots.clone());
        }
        let labels = self.resolve_labels()?;
        let mut acc: Option<HashSet<PathBuf>> = None;
        for filter in &plan.root_filters {
            let subset = self.roots_matching_filter(filter, &labels)?;
            acc = Some(match acc {
                None => subset,
                Some(prev) => prev.intersection(&subset).cloned().collect(),
            });
        }
        Ok(acc.unwrap().into_iter().collect())
    }

    fn roots_matching_filter(
        &self,
        filter: &str,
        labels: &[(PathBuf, String)],
    ) -> Result<HashSet<PathBuf>> {
        let mut prefix = expand_tilde(filter);
        if prefix.exists() {
            if let Ok(canon) = std::fs::canonicalize(&prefix) {
                prefix = canon;
            }
        }
        let needle = filter.to_lowercase();
        Ok(labels
            .iter()
            .filter(|(root, label)| root_matches_filter(root, label, filter, &needle, &prefix))
            .map(|(root, _)| root.clone())
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
    if path_prefix_match(root, prefix) {
        return true;
    }
    // Alias shortcut (e.g. root:w) — exact label, not substring (avoids "w" ⊂ "personal").
    if label.eq_ignore_ascii_case(filter) {
        return true;
    }
    // Path tail after optional ~/ (e.g. root:~/work or root:personal).
    let path_needle = needle
        .strip_prefix("~/")
        .or_else(|| needle.strip_prefix('~'))
        .unwrap_or(needle);
    // Path component — basename only, not the full temp path.
    if root
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(path_needle))
    {
        return true;
    }
    let root_str = root.to_string_lossy().to_lowercase();
    root_str.ends_with(path_needle)
        || root_str.contains(&format!("/{path_needle}"))
        || root_str.contains(&format!("\\{path_needle}"))
}

/// Prefix/equality match tolerant of canonical vs non-canonical paths (Windows `\\?\`).
fn path_prefix_match(root: &Path, prefix: &Path) -> bool {
    if root == prefix || root.starts_with(prefix) || prefix.starts_with(root) {
        return true;
    }
    let root_ok = root.exists();
    let prefix_ok = prefix.exists();
    if root_ok && prefix_ok {
        if let (Ok(r), Ok(p)) = (std::fs::canonicalize(root), std::fs::canonicalize(prefix)) {
            if r == p || r.starts_with(&p) || p.starts_with(&r) {
                return true;
            }
        }
    }
    false
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
        let raw = std::fs::read_to_string(&path).map_err(|source| ConfigError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&raw).map_err(|source| ConfigError::InvalidToml {
            path,
            source: Box::new(source),
        })
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        std::fs::create_dir_all(&self.base).map_err(|source| ConfigError::CreateDirFailed {
            path: self.base.clone(),
            source,
        })?;
        let path = self.config_path();
        std::fs::write(&path, toml::to_string_pretty(config)?)
            .map_err(|source| ConfigError::WriteFailed { path, source })?;
        Ok(())
    }

    pub fn add_roots(&self, paths: &[PathBuf]) -> Result<Config> {
        let mut config = self.load()?;
        for p in paths {
            let abs = std::fs::canonicalize(p).map_err(|source| ConfigError::NonexistentRoot {
                path: p.clone(),
                source,
            })?;
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
        assert!(
            matches!(err, ConfigError::NonexistentRoot { .. }),
            "got: {err:?}"
        );
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
    fn config_inventory_ttl_secs_defaults_to_zero() {
        let cfg = Config::default();
        assert_eq!(cfg.inventory_ttl_secs, 0);
    }

    #[test]
    fn config_inventory_ttl_secs_roundtrips_from_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let cfg = Config {
            inventory_ttl_secs: 3600,
            ..Config::default()
        };
        store.save(&cfg).unwrap();
        assert_eq!(store.load().unwrap().inventory_ttl_secs, 3600);
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
    fn resolve_scan_roots_intersection_empty_when_filters_disjoint() {
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

        let plan = crate::query::ScanPlan {
            root_filters: vec!["w".into(), "personal".into()],
            ..Default::default()
        };
        let matched = cfg.resolve_scan_roots(&plan).unwrap();
        assert!(matched.is_empty());
    }

    #[test]
    fn resolve_scan_roots_single_filter_matches_same_as_one_root_token() {
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

        let via_parse = cfg
            .resolve_scan_roots(&crate::query::parse("root:w").unwrap())
            .unwrap();
        let via_vec = cfg
            .resolve_scan_roots(&crate::query::ScanPlan {
                root_filters: vec!["w".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(via_vec, via_parse);
        assert_eq!(via_vec, vec![work]);
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
    fn expand_tilde_handles_prefix_bare_and_literal() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(expand_tilde("~/work"), home.join("work"));
        assert_eq!(expand_tilde("~"), home);
        // no leading tilde → returned verbatim, never touches $HOME
        assert_eq!(
            expand_tilde("/abs/path"),
            std::path::PathBuf::from("/abs/path")
        );
        // a tilde mid-string is NOT a home marker
        assert_eq!(expand_tilde("a~b"), std::path::PathBuf::from("a~b"));
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
        let err = cfg.resolve_labels().unwrap_err();
        assert!(
            matches!(err, ConfigError::LabelCollision { ref label, .. } if label == "repos"),
            "got: {err:?}"
        );
        // The actionable hint lives only in the #[error] message; guard it so a
        // future edit can't silently drop the "set an alias" guidance.
        let msg = err.to_string();
        assert!(msg.contains("share label"), "got: {msg}");
        assert!(msg.contains("set an alias in config.toml"), "got: {msg}");
    }

    #[test]
    fn config_contexts_default_empty() {
        let cfg = Config::default();
        assert!(cfg.contexts.is_empty());
    }

    #[test]
    fn config_contexts_roundtrip_from_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let cfg = Config {
            contexts: BTreeMap::from([(
                "work".into(),
                ContextDef {
                    filter: "root:work".into(),
                },
            )]),
            ..Config::default()
        };
        store.save(&cfg).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.contexts.get("work"),
            Some(&ContextDef {
                filter: "root:work".into(),
            })
        );
    }

    #[test]
    fn context_filter_returns_filter_for_known_context() {
        let cfg = Config {
            contexts: BTreeMap::from([(
                "work".into(),
                ContextDef {
                    filter: "root:work".into(),
                },
            )]),
            ..Config::default()
        };
        assert_eq!(cfg.context_filter("work").unwrap(), "root:work");
    }

    #[test]
    fn context_filter_errors_for_unknown_context() {
        let cfg = Config::default();
        let err = cfg.context_filter("missing").unwrap_err();
        assert!(
            matches!(err, ConfigError::UnknownContext { ref name } if name == "missing"),
            "got: {err:?}"
        );
    }
}
