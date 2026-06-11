//! Config persistida em <base>/config.toml.
//! O caminho base vem de fora (main resolve OPEN_LOOPS_HOME ou ~/.open-loops)
//! para que testes injetem um tempdir — nada aqui lê variáveis de ambiente.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Diretórios onde os repositórios git são procurados.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Comando que recebe o prompt em stdin e devolve a resposta em stdout.
    #[serde(default = "default_llm_command")]
    pub llm_command: String,
    /// Diretório de sessões do Claude Code.
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: PathBuf,
    /// Máximo de sessões usadas na destilação.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    /// KB lidos do fim de cada sessão.
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
            std::fs::read_to_string(&path).with_context(|| format!("lendo {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("config.toml inválido em {}", path.display()))
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        std::fs::create_dir_all(&self.base)
            .with_context(|| format!("criando {}", self.base.display()))?;
        std::fs::write(self.config_path(), toml::to_string_pretty(config)?)?;
        Ok(())
    }

    pub fn add_roots(&self, paths: &[PathBuf]) -> Result<Config> {
        let mut config = self.load()?;
        for p in paths {
            let abs = std::fs::canonicalize(p)
                .with_context(|| format!("raiz inexistente: {}", p.display()))?;
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
    fn load_sem_arquivo_retorna_default() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let cfg = store.load().unwrap();
        assert!(cfg.roots.is_empty());
        assert_eq!(cfg.llm_command, "claude -p");
        assert_eq!(cfg.max_sessions, 3);
        assert_eq!(cfg.max_session_kb, 50);
    }

    #[test]
    fn save_e_load_roundtrip() {
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
    fn add_roots_canonicaliza_e_deduplica() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let root = tmp.path().join("projetos");
        std::fs::create_dir_all(&root).unwrap();
        store.add_roots(std::slice::from_ref(&root)).unwrap();
        let cfg = store.add_roots(std::slice::from_ref(&root)).unwrap();
        assert_eq!(cfg.roots.len(), 1);
        assert!(cfg.roots[0].is_absolute());
    }

    #[test]
    fn add_roots_falha_para_dir_inexistente() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let err = store
            .add_roots(&[tmp.path().join("nao-existe")])
            .unwrap_err();
        assert!(err.to_string().contains("raiz inexistente"));
    }
}
