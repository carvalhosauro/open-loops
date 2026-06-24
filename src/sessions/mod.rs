//! AI session sources. Each harness (Claude Code, and future ones like Codex,
//! OpenCode) becomes an adapter of this trait — the rest of the code does not
//! know the session format or location.
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

pub mod claude_code;

#[derive(Debug)]
pub struct SessionExcerpt {
    /// Session file name (shown in the Sources section).
    pub source: String,
    pub modified: DateTime<Utc>,
    /// Extracted text (user/assistant messages), already truncated.
    pub text: String,
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
