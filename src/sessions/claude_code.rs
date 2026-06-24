//! Adapter for Claude Code sessions (~/.claude/projects/<path-encoded>/*.jsonl).
//! WARNING: internal Claude Code format, not a public API — may change.
//! Parsing is therefore tolerant: a bad line is skipped, never aborts (spec risk 1).
use super::{SessionExcerpt, SessionSource};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

pub struct ClaudeCode {
    pub projects_dir: PathBuf,
}

/// Claude Code encodes the project path by replacing '/' and '.' with '-'.
/// e.g. /home/g/repo/x -> -home-g-repo-x
pub fn encode_project_path(p: &Path) -> String {
    p.to_string_lossy().replace(['/', '.'], "-")
}

/// Extracts text from a session jsonl line. None for non-message,
/// corrupted, or empty lines (tolerant parsing).
pub fn extract_text(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let role = v.get("type")?.as_str()?;
    if role != "user" && role != "assistant" {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(format!("[{role}] {text}"))
    }
}

/// Reads the last `max_bytes` of the file and extracts message text.
/// The end of the conversation concentrates the "where I left off" signal (spec decision).
fn read_tail_text(path: &Path, max_bytes: u64) -> Result<String> {
    let raw = std::fs::read(path)?;
    let start = raw.len().saturating_sub(max_bytes as usize);
    let tail = String::from_utf8_lossy(&raw[start..]);
    let mut lines = tail.lines();
    if start > 0 {
        lines.next(); // first line may be cut mid-way
    }
    Ok(lines
        .filter_map(extract_text)
        .collect::<Vec<_>>()
        .join("\n"))
}

impl SessionSource for ClaudeCode {
    /// Excerpts of the sessions most relevant to the branch.
    ///
    /// # Errors
    ///
    /// Returns an error if the project directory cannot be read.
    fn excerpts(
        &self,
        repo_path: &Path,
        branch: &str,
        window: (DateTime<Utc>, DateTime<Utc>),
        max_sessions: usize,
        max_kb: u64,
    ) -> Result<Vec<SessionExcerpt>> {
        let dir = self.projects_dir.join(encode_project_path(repo_path));
        if !dir.is_dir() {
            return Ok(vec![]);
        }
        let pad = Duration::days(7);
        let (start, end) = (window.0 - pad, window.1 + pad);
        let mut candidates: Vec<(DateTime<Utc>, PathBuf, bool, bool)> = Vec::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let modified: DateTime<Utc> = modified.into();
            let in_window = modified >= start && modified <= end;
            let mentions_branch = std::fs::read_to_string(&path)
                .map(|c| c.contains(branch))
                .unwrap_or(false);
            // spec heuristic: in the time window OR mentions the branch
            if in_window || mentions_branch {
                candidates.push((modified, path, in_window, mentions_branch));
            }
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0)); // most recent first
        candidates.truncate(max_sessions);
        let mut out = Vec::new();
        for (modified, path, in_window, mentions_branch) in candidates {
            let text = read_tail_text(&path, max_kb * 1024)?;
            if text.is_empty() {
                continue;
            }
            let source = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(SessionExcerpt {
                source,
                modified,
                text,
                in_window,
                mentions_branch,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionSource;
    use chrono::{Duration, Utc};
    use std::path::Path;

    #[test]
    fn encode_project_path_matches_claude_code_format() {
        assert_eq!(
            encode_project_path(Path::new("/home/g/repo/me/open-loops")),
            "-home-g-repo-me-open-loops"
        );
        assert_eq!(
            encode_project_path(Path::new("/home/g/my.app")),
            "-home-g-my-app"
        );
    }

    #[test]
    fn extract_text_captures_user_assistant_and_ignores_rest() {
        let user = r#"{"type":"user","message":{"content":"quero implementar login"}}"#;
        let asst = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"vou criar feat/login"}]}}"#;
        let meta = r#"{"type":"summary","summary":"x"}"#;
        assert_eq!(
            extract_text(user).unwrap(),
            "[user] quero implementar login"
        );
        assert_eq!(
            extract_text(asst).unwrap(),
            "[assistant] vou criar feat/login"
        );
        assert!(extract_text(meta).is_none());
        assert!(extract_text("corrupted non-json line").is_none());
    }

    #[test]
    fn excerpts_selects_by_window_tolerates_garbage_and_limits_count() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sessao1.jsonl"),
            concat!(
                r#"{"type":"user","message":{"content":"quero implementar login"}}"#, "\n",
                "lixo nao-json\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"proximo passo: validar token"}]}}"#, "\n",
            ),
        )
        .unwrap();
        // files of other formats are ignored
        std::fs::write(dir.join("nota.txt"), "nada").unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));
        let ex = src.excerpts(repo, "feat/login", window, 3, 50).unwrap();
        assert_eq!(ex.len(), 1);
        assert!(ex[0].text.contains("[user] quero implementar login"));
        assert!(ex[0].text.contains("proximo passo: validar token"));
        assert_eq!(ex[0].source, "sessao1.jsonl");
    }

    #[test]
    fn excerpts_empty_when_project_dir_does_not_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let src = ClaudeCode {
            projects_dir: tmp.path().to_path_buf(),
        };
        let now = Utc::now();
        let ex = src
            .excerpts(Path::new("/nao/existe"), "b", (now, now), 3, 50)
            .unwrap();
        assert!(ex.is_empty());
    }

    #[test]
    fn excerpts_includes_session_outside_window_if_it_mentions_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("antiga.jsonl"),
            concat!(
                r#"{"type":"user","message":{"content":"implementando feat/login agora"}}"#,
                "\n",
            ),
        )
        .unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = Utc::now();
        // window two years ago — file mtime is now (outside the window)
        let passado = now - Duration::days(730);
        let window = (passado - Duration::days(1), passado);
        let ex = src.excerpts(repo, "feat/login", window, 3, 50).unwrap();
        assert_eq!(ex.len(), 1, "mention heuristic must include the session");
        assert!(ex[0].text.contains("feat/login"));
    }

    #[test]
    fn excerpts_truncates_large_file_and_skips_cut_line() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();

        // padding with summary lines (not extracted) to force file > 1 KB
        let pad_line = format!("{{\"type\":\"summary\",\"x\":\"{}\"}}\n", "A".repeat(80));
        let mut content = pad_line.repeat(15); // ~1500 bytes
        content.push_str(r#"{"type":"user","message":{"content":"contexto final"}}"#);
        content.push('\n');
        assert!(content.len() > 1024);

        std::fs::write(dir.join("grande.jsonl"), &content).unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));
        // max_kb=1 forces truncation: start > 0 → first line of the tail is skipped
        let ex = src.excerpts(repo, "feat/x", window, 3, 1).unwrap();
        assert_eq!(ex.len(), 1);
        assert!(ex[0].text.contains("contexto final"));
    }

    #[test]
    fn excerpts_skips_session_with_only_messages_without_text() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();
        // only summary and tool_result lines — extract_text returns None for all of them
        std::fs::write(
            dir.join("vazia.jsonl"),
            concat!(
                r#"{"type":"summary","summary":"nada util"}"#,
                "\n",
                r#"{"type":"tool_result","content":[]}"#,
                "\n",
            ),
        )
        .unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));
        let ex = src.excerpts(repo, "feat/x", window, 3, 50).unwrap();
        assert!(
            ex.is_empty(),
            "session with no extractable text must be skipped"
        );
    }
}
