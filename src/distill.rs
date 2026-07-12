//! Distillation: builds the prompt with evidence (git + sessions) and calls the
//! LLM via a configurable command (default "claude -p"). Injectable command means
//! tests use `cat` and users can swap LLMs without changing code.
use crate::error::DistillError;
use crate::output;
use crate::scanner::OpenLoop;
use crate::sessions::SessionExcerpt;
use std::io::Write;
use std::process::{Command, Stdio};

type Result<T> = std::result::Result<T, DistillError>;

/// How well AI sessions align with git evidence for a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Sessions overlap commit window and mention the branch name.
    High,
    /// Sessions matched heuristically but alignment is uncertain.
    Medium,
    /// No AI sessions — context comes from git only.
    Low,
}

/// Derives a confidence level from matched session excerpts.
pub fn compute_confidence(excerpts: &[SessionExcerpt]) -> Confidence {
    if excerpts.is_empty() {
        return Confidence::Low;
    }
    if excerpts.iter().any(|e| e.in_window && e.mentions_branch) {
        Confidence::High
    } else {
        Confidence::Medium
    }
}

fn confidence_label(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn confidence_explanation(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "AI sessions align with branch commits",
        Confidence::Medium => {
            "AI sessions found but alignment uncertain — audit Sources before trusting"
        }
        Confidence::Low => "no AI sessions matched — context from git only",
    }
}

fn format_confidence_line(c: Confidence) -> String {
    format!(
        "**Confidence:** {} — {}",
        confidence_label(c),
        confidence_explanation(c)
    )
}

/// Builds the context-reconstruction prompt for an open loop.
///
/// Includes branch, commits, diffstat, and AI session excerpts.
/// When there are no sessions, explicitly declares none found.
pub fn build_prompt(
    lp: &OpenLoop,
    default_branch: &str,
    commits: &str,
    diffstat: &str,
    excerpts: &[SessionExcerpt],
) -> String {
    let mut p = format!(
        "You reconstruct the context of a paused work branch.\n\
         Answer in markdown, in English, with exactly these sections:\n\n\
         ## Why\n## Done\n## Remaining\n## Next step\n\n\
         Be concrete and direct. Rely ONLY on the evidence below.\n\
         If the evidence is insufficient for a section, write \"insufficient evidence\".\n\n\
         # Branch\n{key} (base: {default_branch})\n\n\
         # Commits (base..branch)\n{commits}\n\n\
         # Diffstat\n{diffstat}\n",
        key = lp.key(),
    );
    if excerpts.is_empty() {
        p.push_str("\n# AI sessions\nnone found\n");
    } else {
        for e in excerpts {
            p.push_str(&format!(
                "\n# Session {} (modified {})\n{}\n",
                e.source,
                e.modified.format("%Y-%m-%d"),
                e.text
            ));
        }
    }
    p
}

/// Runs the LLM command with the prompt on stdin and returns stdout.
///
/// The command is interpreted via `sh -c`, so it may contain pipes and
/// redirections (e.g. `"claude -p | tee /tmp/output.md"`).
///
/// # Errors
///
/// Returns `Err` if the process cannot be started or exits with a non-zero
/// status (e.g. LLM not installed, missing credential).
pub fn run_llm(llm_command: &str, prompt: &str) -> Result<String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(llm_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| DistillError::SpawnFailed {
            command: llm_command.to_string(),
            source,
        })?;
    child
        .stdin
        .take()
        .ok_or(DistillError::NoStdin)?
        .write_all(prompt.as_bytes())
        .or_else(|e| {
            // broken pipe means the LLM exited before reading all of stdin — that's fine
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(DistillError::WriteFailed { source: e })
            }
        })?;
    let out = child
        .wait_with_output()
        .map_err(|source| DistillError::WaitFailed { source })?;
    if !out.status.success() {
        return Err(DistillError::CommandFailed {
            command: llm_command.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Appends the `## Sources` section to the LLM-generated document.
///
/// Lets the user audit the evidence used in the reconstruction
/// (mitigates hallucination risk — see spec §Risks).
pub fn with_sources(
    answer: &str,
    lp: &OpenLoop,
    excerpts: &[SessionExcerpt],
    confidence: Confidence,
) -> String {
    let short_sha = &lp.head_sha[..7.min(lp.head_sha.len())];
    let mut doc = format!(
        "# {}\n\n{}\n\n{}\n\n## Sources\n\n- git: branch {} (HEAD {})\n",
        lp.key(),
        format_confidence_line(confidence),
        answer.trim(),
        lp.branch,
        short_sha
    );
    for e in excerpts {
        doc.push_str(&format!(
            "- AI session: {} (modified {})\n",
            e.source,
            e.modified.format("%Y-%m-%d")
        ));
    }
    doc
}

fn session_match_tags(e: &SessionExcerpt) -> String {
    let mut tags = Vec::new();
    if e.in_window {
        tags.push("in commit window");
    }
    if e.mentions_branch {
        tags.push("mentions branch");
    }
    if tags.is_empty() {
        "matched by heuristic".into()
    } else {
        tags.join(", ")
    }
}

fn format_ab(ahead: Option<u32>, behind: Option<u32>) -> String {
    format!(
        "{}, behind: {}",
        output::fmt_count(ahead),
        output::fmt_count(behind)
    )
}

/// Shows git and session evidence that would feed distillation, without calling the LLM.
pub fn format_dry_run(
    lp: &OpenLoop,
    default_branch: &str,
    commits: &str,
    diffstat: &str,
    excerpts: &[SessionExcerpt],
    confidence: Confidence,
) -> String {
    let short_sha = &lp.head_sha[..7.min(lp.head_sha.len())];
    let mut out = format!(
        "# {}\n\n{}\n\n\
         ## Git\n\n\
         - branch: {} (HEAD {})\n\
         - base: {}\n\
         - ahead: {}\n\n\
         ### Commits (base..branch)\n{}\n\n\
         ### Diffstat\n{}\n\n\
         ## AI sessions\n",
        lp.key(),
        format_confidence_line(confidence),
        lp.branch,
        short_sha,
        default_branch,
        format_ab(lp.ahead, lp.behind),
        commits.trim_end(),
        diffstat.trim_end(),
    );
    if excerpts.is_empty() {
        out.push_str("none matched\n");
    } else {
        for e in excerpts {
            out.push_str(&format!(
                "- {} (modified {}) [{}]\n",
                e.source,
                e.modified.format("%Y-%m-%d"),
                session_match_tags(e),
            ));
        }
    }
    out.push_str("\n---\nDry run — LLM not invoked. Run without `--dry-run` to distill.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use crate::sessions::SessionExcerpt;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fake_loop() -> OpenLoop {
        OpenLoop {
            root_label: "app".into(),
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: "abcdef1234567890".into(),
            last_commit: Utc::now(),
            ahead: Some(2),
            behind: Some(1),
        }
    }

    fn fake_excerpt() -> SessionExcerpt {
        SessionExcerpt {
            source: "sessao1.jsonl".into(),
            modified: Utc::now(),
            text: "[user] implementa login".into(),
            in_window: true,
            mentions_branch: true,
        }
    }

    #[test]
    fn compute_confidence_levels() {
        assert_eq!(compute_confidence(&[]), Confidence::Low);
        let medium = SessionExcerpt {
            in_window: true,
            mentions_branch: false,
            ..fake_excerpt()
        };
        assert_eq!(compute_confidence(&[medium]), Confidence::Medium);
        assert_eq!(compute_confidence(&[fake_excerpt()]), Confidence::High);
    }

    #[test]
    fn build_prompt_includes_evidence_and_sections() {
        let p = build_prompt(
            &fake_loop(),
            "main",
            "abc feat: wip",
            "x.txt | 2 +",
            &[fake_excerpt()],
        );
        assert!(p.contains("## Why"));
        assert!(p.contains("## Next step"));
        assert!(p.contains("app/feat/login"));
        assert!(p.contains("abc feat: wip"));
        assert!(p.contains("[user] implementa login"));
    }

    #[test]
    fn build_prompt_without_sessions_declares_absence() {
        let p = build_prompt(&fake_loop(), "main", "", "", &[]);
        assert!(p.contains("none found"));
    }

    #[test]
    fn build_prompt_separates_multiple_sessions() {
        let first = SessionExcerpt {
            source: "sessao1.jsonl".into(),
            text: "[user] first task".into(),
            ..fake_excerpt()
        };
        let second = SessionExcerpt {
            source: "sessao2.jsonl".into(),
            text: "[user] second task".into(),
            ..fake_excerpt()
        };
        let p = build_prompt(&fake_loop(), "main", "", "", &[first, second]);
        // both excerpts land under their own `# Session <source>` header, in order
        let h1 = p.find("# Session sessao1.jsonl").expect("first header");
        let h2 = p.find("# Session sessao2.jsonl").expect("second header");
        assert!(h1 < h2, "sessions kept in order");
        assert!(p.contains("[user] first task"));
        assert!(p.contains("[user] second task"));
        // no "none found" sentinel when excerpts exist
        assert!(!p.contains("none found"));
    }

    #[test]
    fn run_llm_passes_prompt_via_stdin() {
        // `cat` echoes stdin: validates the contract without a real LLM
        let out = run_llm("cat", "test prompt").unwrap();
        assert_eq!(out.trim(), "test prompt");
    }

    #[test]
    fn run_llm_contextual_error_when_command_fails() {
        let err = run_llm("false", "x").unwrap_err();
        assert!(
            matches!(err, DistillError::CommandFailed { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn with_sources_appends_git_and_sessions() {
        let doc = with_sources(
            "## Why\nlogin",
            &fake_loop(),
            &[fake_excerpt()],
            Confidence::High,
        );
        assert!(doc.contains("## Sources"));
        assert!(doc.contains("**Confidence:** high"));
        assert!(doc.contains("abcdef1")); // short sha
        assert!(doc.contains("sessao1.jsonl"));
    }

    #[test]
    fn format_dry_run_lists_evidence_without_llm_sections() {
        let doc = format_dry_run(
            &fake_loop(),
            "main",
            "abc feat: wip",
            "x.txt | 2 +",
            &[fake_excerpt()],
            Confidence::High,
        );
        assert!(doc.contains("**Confidence:** high"));
        assert!(doc.contains("abc feat: wip"));
        assert!(doc.contains("sessao1.jsonl"));
        assert!(doc.contains("in commit window, mentions branch"));
        assert!(doc.contains("Dry run — LLM not invoked"));
        assert!(!doc.contains("## Why"));
    }

    #[test]
    fn with_sources_short_sha_when_head_sha_under_7_chars() {
        let lp = OpenLoop {
            root_label: "app".into(),
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/x".into(),
            head_sha: "ab1".into(), // 3 chars < 7
            last_commit: Utc::now(),
            ahead: Some(0),
            behind: Some(0),
        };
        let doc = with_sources("## Why\nconteudo", &lp, &[], Confidence::Low);
        assert!(doc.contains("ab1"));
        assert!(!doc.contains("ab1\0")); // no extra bytes
    }
}
