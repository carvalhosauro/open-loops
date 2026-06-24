//! Distillation: builds the prompt with evidence (git + sessions) and calls the
//! LLM via a configurable command (default "claude -p"). Injectable command means
//! tests use `cat` and users can swap LLMs without changing code.
use crate::scanner::OpenLoop;
use crate::sessions::SessionExcerpt;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

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
        .with_context(|| {
            format!(
                "failed to run the LLM command `{llm_command}` — \
                 is it installed? Adjust llm_command in config.toml"
            )
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("stdin not available for the LLM process"))?
        .write_all(prompt.as_bytes())
        .or_else(|e| {
            // broken pipe means the LLM exited before reading all of stdin — that's fine
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(e).context("failed to write the prompt to the LLM stdin")
            }
        })?;
    let out = child
        .wait_with_output()
        .context("failed to wait for the LLM process")?;
    if !out.status.success() {
        bail!(
            "LLM command failed (`{llm_command}`): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Appends the `## Sources` section to the LLM-generated document.
///
/// Lets the user audit the evidence used in the reconstruction
/// (mitigates hallucination risk — see spec §Risks).
pub fn with_sources(answer: &str, lp: &OpenLoop, excerpts: &[SessionExcerpt]) -> String {
    let short_sha = &lp.head_sha[..7.min(lp.head_sha.len())];
    let mut doc = format!(
        "# {}\n\n{}\n\n## Sources\n\n- git: branch {} (HEAD {})\n",
        lp.key(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use crate::sessions::SessionExcerpt;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fake_loop() -> OpenLoop {
        OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: "abcdef1234567890".into(),
            last_commit: Utc::now(),
            ahead: 2,
            behind: 1,
        }
    }

    fn fake_excerpt() -> SessionExcerpt {
        SessionExcerpt {
            source: "sessao1.jsonl".into(),
            modified: Utc::now(),
            text: "[user] implementa login".into(),
        }
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
    fn run_llm_passes_prompt_via_stdin() {
        // `cat` echoes stdin: validates the contract without a real LLM
        let out = run_llm("cat", "test prompt").unwrap();
        assert_eq!(out.trim(), "test prompt");
    }

    #[test]
    fn run_llm_contextual_error_when_command_fails() {
        let err = run_llm("false", "x").unwrap_err();
        assert!(err.to_string().contains("LLM command failed"));
    }

    #[test]
    fn with_sources_appends_git_and_sessions() {
        let doc = with_sources("## Why\nlogin", &fake_loop(), &[fake_excerpt()]);
        assert!(doc.contains("## Sources"));
        assert!(doc.contains("abcdef1")); // short sha
        assert!(doc.contains("sessao1.jsonl"));
    }

    #[test]
    fn with_sources_short_sha_when_head_sha_under_7_chars() {
        let lp = OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/x".into(),
            head_sha: "ab1".into(), // 3 chars < 7
            last_commit: Utc::now(),
            ahead: 0,
            behind: 0,
        };
        let doc = with_sources("## Why\nconteudo", &lp, &[]);
        assert!(doc.contains("ab1"));
        assert!(!doc.contains("ab1\0")); // no extra bytes
    }
}
