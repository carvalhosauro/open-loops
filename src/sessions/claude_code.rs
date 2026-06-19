//! Adapter para sessões do Claude Code (~/.claude/projects/<path-encoded>/*.jsonl).
//! ATENÇÃO: formato interno do Claude Code, não é API pública — pode mudar.
//! Por isso o parsing é tolerante: linha ruim é pulada, nunca aborta (risco 1 da spec).
use super::{SessionExcerpt, SessionSource};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

pub struct ClaudeCode {
    pub projects_dir: PathBuf,
}

/// Claude Code codifica o caminho do projeto substituindo '/' e '.' por '-'.
/// Ex.: /home/g/repo/x -> -home-g-repo-x
pub fn encode_project_path(p: &Path) -> String {
    p.to_string_lossy().replace(['/', '.'], "-")
}

/// Extrai o texto de uma linha jsonl de sessão. None para linhas
/// não-mensagem, corrompidas ou vazias (parsing tolerante).
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

/// Lê os últimos `max_bytes` do arquivo e extrai o texto das mensagens.
/// O fim da conversa concentra o "onde parei" (decisão da spec).
fn read_tail_text(path: &Path, max_bytes: u64) -> Result<String> {
    let raw = std::fs::read(path)?;
    let start = raw.len().saturating_sub(max_bytes as usize);
    let tail = String::from_utf8_lossy(&raw[start..]);
    let mut lines = tail.lines();
    if start > 0 {
        lines.next(); // primeira linha pode estar cortada no meio
    }
    Ok(lines
        .filter_map(extract_text)
        .collect::<Vec<_>>()
        .join("\n"))
}

impl SessionSource for ClaudeCode {
    /// Trechos das sessões mais relevantes para a branch.
    ///
    /// # Errors
    ///
    /// Retorna erro se não for possível ler o diretório do projeto.
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
        let mut candidates: Vec<(DateTime<Utc>, PathBuf)> = Vec::new();
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
            // heurística da spec: janela temporal OU menção à branch
            let relevant = in_window
                || std::fs::read_to_string(&path)
                    .map(|c| c.contains(branch))
                    .unwrap_or(false);
            if relevant {
                candidates.push((modified, path));
            }
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0)); // mais recente primeiro
        candidates.truncate(max_sessions);
        let mut out = Vec::new();
        for (modified, path) in candidates {
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
    fn encode_project_path_igual_ao_claude_code() {
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
    fn extract_text_pega_user_assistant_e_ignora_resto() {
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
        assert!(extract_text("linha corrompida não-json").is_none());
    }

    #[test]
    fn excerpts_seleciona_por_janela_tolera_lixo_e_limita_quantidade() {
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
        // arquivo de outro formato é ignorado
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
    fn excerpts_vazio_quando_dir_do_projeto_nao_existe() {
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
    fn excerpts_inclui_sessao_fora_janela_se_menciona_branch() {
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

        let src = ClaudeCode { projects_dir: projects };
        let now = Utc::now();
        // janela dois anos atrás — mtime do arquivo é agora (fora da janela)
        let passado = now - Duration::days(730);
        let window = (passado - Duration::days(1), passado);
        let ex = src.excerpts(repo, "feat/login", window, 3, 50).unwrap();
        assert_eq!(ex.len(), 1, "heurística de menção deve incluir a sessão");
        assert!(ex[0].text.contains("feat/login"));
    }

    #[test]
    fn excerpts_trunca_arquivo_grande_e_pula_linha_cortada() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();

        // padding de linhas summary (não extraídas) para forçar arquivo > 1 KB
        let pad_line = format!(
            "{{\"type\":\"summary\",\"x\":\"{}\"}}\n",
            "A".repeat(80)
        );
        let mut content = pad_line.repeat(15); // ~1500 bytes
        content.push_str(
            r#"{"type":"user","message":{"content":"contexto final"}}"#,
        );
        content.push('\n');
        assert!(content.len() > 1024);

        std::fs::write(dir.join("grande.jsonl"), &content).unwrap();

        let src = ClaudeCode { projects_dir: projects };
        let now = Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));
        // max_kb=1 força truncamento: start > 0 → primeira linha do tail é pulada
        let ex = src.excerpts(repo, "feat/x", window, 3, 1).unwrap();
        assert_eq!(ex.len(), 1);
        assert!(ex[0].text.contains("contexto final"));
    }

    #[test]
    fn excerpts_pula_sessao_com_apenas_mensagens_sem_texto() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();
        // apenas linhas summary e tool_result — extract_text retorna None para todas
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

        let src = ClaudeCode { projects_dir: projects };
        let now = Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));
        let ex = src.excerpts(repo, "feat/x", window, 3, 50).unwrap();
        assert!(ex.is_empty(), "sessão sem texto extraível deve ser pulada");
    }
}
