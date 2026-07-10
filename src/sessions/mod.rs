//! AI session sources. Each harness (Claude Code, and future ones like Codex,
//! OpenCode) becomes an adapter of this trait — the rest of the code does not
//! know the session format or location.
use crate::error::SessionError;
use chrono::{DateTime, Utc};
use std::path::Path;

pub mod claude_code;

type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone)]
pub struct SessionExcerpt {
    /// Session file name (shown in the Sources section).
    pub source: String,
    pub modified: DateTime<Utc>,
    /// Extracted text (user/assistant messages), already truncated.
    pub text: String,
    /// Session mtime falls within the branch commit window (±7 days).
    pub in_window: bool,
    /// Session content mentions the branch name.
    pub mentions_branch: bool,
}

pub trait SessionSource {
    /// Excerpts of the sessions most relevant to the branch.
    /// `window`: commit time range of the branch (sessions outside it that do
    /// not mention the branch are discarded).
    ///
    /// # Errors
    ///
    /// Returns an error if the projects directory cannot be read.
    fn excerpts(
        &self,
        repo_path: &Path,
        branch: &str,
        window: (DateTime<Utc>, DateTime<Utc>),
        max_sessions: usize,
        max_kb: u64,
    ) -> Result<Vec<SessionExcerpt>>;
}
