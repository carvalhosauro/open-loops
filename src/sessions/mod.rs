//! Fontes de sessão de IA. Cada harness (Claude Code, futuramente Codex,
//! OpenCode) vira um adapter deste trait — o resto do código não conhece
//! formato nem localização de sessão.
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

pub mod claude_code;

#[derive(Debug)]
pub struct SessionExcerpt {
    /// Nome do arquivo de sessão (exibido na seção Fontes).
    pub source: String,
    pub modified: DateTime<Utc>,
    /// Texto extraído (mensagens user/assistant), já truncado.
    pub text: String,
}

pub trait SessionSource {
    /// Trechos das sessões mais relevantes para a branch.
    /// `window`: intervalo dos commits da branch (sessões fora dele e que não
    /// mencionam a branch são descartadas).
    ///
    /// # Errors
    ///
    /// Retorna erro se não for possível ler o diretório de projetos.
    fn excerpts(
        &self,
        repo_path: &Path,
        branch: &str,
        window: (DateTime<Utc>, DateTime<Utc>),
        max_sessions: usize,
        max_kb: u64,
    ) -> Result<Vec<SessionExcerpt>>;
}
