//! Adapter for Claude Code sessions (~/.claude/projects/<path-encoded>/*.jsonl).
//! WARNING: internal Claude Code format, not a public API — may change.
//! Parsing is therefore tolerant: a bad line is skipped, never aborts (spec risk 1).
use super::{SessionExcerpt, SessionSource};
use crate::index::Index;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

pub struct ClaudeCode {
    pub projects_dir: PathBuf,
}

/// Claude Code encodes the project path by replacing path separators and '.' with '-'.
/// e.g. /home/g/repo/x -> -home-g-repo-x, C:\Users\g\app -> C--Users-g-app
pub fn encode_project_path(p: &Path) -> String {
    let raw = p.to_string_lossy();
    // Windows canonicalize() may add \\?\ — Claude Code encodes the normal path.
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    raw.replace(['/', '\\', '.', ':'], "-")
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

impl ClaudeCode {
    /// Core excerpt logic, optionally accelerated by an FTS index.
    ///
    /// `None` index = in-memory path (reads bounded tail for every candidate).
    /// `Some(index)` = FTS path (uses `session_mentions` for the mention probe;
    /// still reads the bounded tail for the final selected sessions).
    ///
    /// # Errors
    ///
    /// Returns an error if the project directory cannot be read.
    pub fn excerpts_indexed(
        &self,
        repo_path: &Path,
        branch: &str,
        window: (DateTime<Utc>, DateTime<Utc>),
        max_sessions: usize,
        max_kb: u64,
        index: Option<&Index>,
    ) -> Result<Vec<SessionExcerpt>> {
        let dir = self.projects_dir.join(encode_project_path(repo_path));
        if !dir.is_dir() {
            return Ok(vec![]);
        }
        let pad = Duration::days(7);
        let (start, end) = (window.0 - pad, window.1 + pad);

        // When an index is available, get FTS-based mention set up-front.
        // On index error, fall back to None (in-memory mention probe below).
        let fts_mentions: Option<std::collections::HashSet<PathBuf>> = index.map(|idx| {
            // Upsert every candidate file's bounded tail into the index.
            // This is safe to call here: upsert skips unchanged (path,mtime) rows.
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_none_or(|e| e != "jsonl") {
                        continue;
                    }
                    let Ok(meta) = entry.metadata() else { continue };
                    let Ok(modified) = meta.modified() else {
                        continue;
                    };
                    let mtime = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let size = meta.len() as i64;
                    // Read bounded tail for indexing (skip on read error).
                    if let Ok(tail) = read_tail_text(&path, max_kb * 1024) {
                        idx.upsert_session(&path, repo_path, mtime, size, &tail);
                    }
                }
            }
            idx.session_mentions(repo_path, branch)
        });

        // Collect candidates: (modified, path, in_window, mentions_branch).
        // For in-memory path: read the bounded tail ONCE for both mention probe
        // AND excerpt text (fixes #14 — no unbounded whole-file read).
        struct Candidate {
            modified: DateTime<Utc>,
            path: PathBuf,
            in_window: bool,
            mentions_branch: bool,
            /// Tail text read during candidate collection (in-memory path only).
            tail: Option<String>,
        }

        let mut candidates: Vec<Candidate> = Vec::new();
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

            if let Some(ref fts_set) = fts_mentions {
                // FTS path: mention signal comes from the index, no file read here.
                let mentions_branch = fts_set.contains(&path);
                if in_window || mentions_branch {
                    candidates.push(Candidate {
                        modified,
                        path,
                        in_window,
                        mentions_branch,
                        tail: None,
                    });
                }
            } else {
                // In-memory path: read bounded tail ONCE for mention probe (#14 fix).
                let tail = read_tail_text(&path, max_kb * 1024).unwrap_or_default();
                let mentions_branch = tail.contains(branch);
                if in_window || mentions_branch {
                    candidates.push(Candidate {
                        modified,
                        path,
                        in_window,
                        mentions_branch,
                        tail: Some(tail),
                    });
                }
            }
        }

        // Stable total-order sort: mentions_branch DESC, in_window DESC,
        // modified DESC, path ASC (#15 fix — deterministic tie-break).
        candidates.sort_by(|a, b| {
            b.mentions_branch
                .cmp(&a.mentions_branch)
                .then(b.in_window.cmp(&a.in_window))
                .then(b.modified.cmp(&a.modified))
                .then(a.path.cmp(&b.path))
        });

        // Build output: filter empty-text sessions BEFORE truncate (#15 fix).
        let mut out = Vec::new();
        for cand in candidates {
            if out.len() >= max_sessions {
                break;
            }
            // Get the text: already read (in-memory path) or read now (FTS path).
            let text = if let Some(t) = cand.tail {
                t
            } else {
                read_tail_text(&cand.path, max_kb * 1024).unwrap_or_default()
            };
            if text.is_empty() {
                // Skip empty sessions — do NOT count them toward the max_sessions
                // limit (this is the #15 regression fix).
                continue;
            }
            let source = cand
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(SessionExcerpt {
                source,
                modified: cand.modified,
                text,
                in_window: cand.in_window,
                mentions_branch: cand.mentions_branch,
            });
        }
        Ok(out)
    }
}

impl SessionSource for ClaudeCode {
    /// Excerpts of the sessions most relevant to the branch.
    ///
    /// Delegates to [`ClaudeCode::excerpts_indexed`] with no index (in-memory path).
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
        self.excerpts_indexed(repo_path, branch, window, max_sessions, max_kb, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
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
    #[cfg(windows)]
    fn encode_project_path_handles_windows_separators() {
        assert_eq!(
            encode_project_path(Path::new(r"C:\Users\g\app")),
            "C--Users-g-app"
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

    // -----------------------------------------------------------------------
    // (a) #15 — same mtime: stable tie-break by path ASC
    // -----------------------------------------------------------------------

    #[test]
    fn excerpts_same_mtime_deterministic_order_by_path() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();

        let line = r#"{"type":"user","message":{"content":"trabalho"}}"#.to_string() + "\n";
        // Write two sessions with the same content; they will have the same mtime
        // (filesystem resolution may differ, so we use max_sessions=1 and verify
        // that the one selected is always the lexicographically first path).
        std::fs::write(dir.join("zzz.jsonl"), &line).unwrap();
        std::fs::write(dir.join("aaa.jsonl"), &line).unwrap();

        // Force identical mtime on both files.
        let now_sys = std::time::SystemTime::now();
        filetime::set_file_mtime(
            dir.join("aaa.jsonl"),
            filetime::FileTime::from_system_time(now_sys),
        )
        .unwrap();
        filetime::set_file_mtime(
            dir.join("zzz.jsonl"),
            filetime::FileTime::from_system_time(now_sys),
        )
        .unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = chrono::Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));

        // max_sessions=1: stable tie-break must pick "aaa.jsonl" every time.
        let ex = src.excerpts(repo, "feat/x", window, 1, 50).unwrap();
        assert_eq!(ex.len(), 1, "must return exactly 1 session");
        assert_eq!(ex[0].source, "aaa.jsonl", "tie-break must pick path ASC");
    }

    // -----------------------------------------------------------------------
    // (b) #15 — empty session excluded BEFORE truncate, real session survives
    // -----------------------------------------------------------------------

    #[test]
    fn excerpts_empty_session_excluded_before_max_sessions_truncate() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();

        // Session with no extractable text (only summary lines).
        std::fs::write(
            dir.join("vazia.jsonl"),
            "{\"type\":\"summary\",\"summary\":\"nada\"}\n",
        )
        .unwrap();
        // A real session with extractable text.
        std::fs::write(
            dir.join("real.jsonl"),
            "{\"type\":\"user\",\"message\":{\"content\":\"trabalho real\"}}\n",
        )
        .unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = chrono::Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));

        // max_sessions=1: if empty session consumed the slot the real one would
        // be dropped — the fixed code must exclude empties BEFORE truncating.
        let ex = src.excerpts(repo, "feat/x", window, 1, 50).unwrap();
        assert_eq!(ex.len(), 1, "real session must survive despite empty peer");
        assert!(
            ex[0].text.contains("trabalho real"),
            "must be the real session"
        );
    }

    // -----------------------------------------------------------------------
    // (c) FTS mention probe via in-memory index
    // -----------------------------------------------------------------------

    #[test]
    fn excerpts_indexed_uses_fts_for_mention_probe() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();

        let branch = "feat/fts-probe";
        let session_path = dir.join("fts.jsonl");
        let content =
            format!("{{\"type\":\"user\",\"message\":{{\"content\":\"working on {branch}\"}}}}\n");
        std::fs::write(&session_path, &content).unwrap();

        let src = ClaudeCode {
            projects_dir: projects,
        };
        let now = chrono::Utc::now();
        // Window that does NOT cover this file's mtime — must rely on mention probe.
        let passado = now - Duration::days(730);
        let window = (passado - Duration::days(1), passado);

        // Index the session's bounded tail.
        let index = Index::open_in_memory();
        let tail = read_tail_text(&session_path, 50 * 1024).unwrap();
        let mtime = std::fs::metadata(&session_path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let size = std::fs::metadata(&session_path).unwrap().len() as i64;
        index.upsert_session(&session_path, repo, mtime, size, &tail);

        // excerpts_indexed with Some(index) must find the session via FTS.
        let ex = src
            .excerpts_indexed(repo, branch, window, 3, 50, Some(&index))
            .unwrap();
        assert_eq!(
            ex.len(),
            1,
            "FTS probe must find the branch-mentioning session"
        );
        assert!(ex[0].text.contains(branch));
    }

    // -----------------------------------------------------------------------
    // (d) upsert_session is idempotent — unchanged (path,mtime) is not re-indexed
    // -----------------------------------------------------------------------

    #[test]
    fn upsert_session_skips_reindex_when_path_mtime_unchanged() {
        let index = Index::open_in_memory();
        let path = Path::new("/fake/session.jsonl");
        let repo = Path::new("/home/g/app");
        let mtime: i64 = 1_700_000_000;
        let size: i64 = 42;
        let text1 = "[user] first index";
        let text2 = "[user] second index should not overwrite";

        // First upsert: stores text1.
        index.upsert_session(path, repo, mtime, size, text1);

        // Second upsert with SAME (path, mtime): must NOT overwrite the FTS row.
        index.upsert_session(path, repo, mtime, size, text2);

        // The FTS index must still contain text1 but NOT text2.
        let mentions = index.session_mentions(repo, "first");
        assert!(
            mentions.contains(&path.to_path_buf()),
            "first index text must be retrievable"
        );

        let mentions2 = index.session_mentions(repo, "second");
        assert!(
            !mentions2.contains(&path.to_path_buf()),
            "second upsert must have been skipped (same mtime)"
        );
    }
}
