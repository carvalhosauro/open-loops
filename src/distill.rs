//! Destilação: monta o prompt com as evidências (git + sessões) e chama o
//! LLM via comando configurável (default "claude -p"). Comando injetável =
//! testes usam `cat` e usuários podem trocar de LLM sem mudar código.
use crate::scanner::OpenLoop;
use crate::sessions::SessionExcerpt;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Monta o prompt de reconstituição de contexto para um loop aberto.
///
/// Inclui branch, commits, diffstat e trechos de sessões de IA.
/// Quando não há sessões, declara explicitamente "nenhuma encontrada".
pub fn build_prompt(
    lp: &OpenLoop,
    default_branch: &str,
    commits: &str,
    diffstat: &str,
    excerpts: &[SessionExcerpt],
) -> String {
    let mut p = format!(
        "Você reconstrói o contexto de uma branch de trabalho pausada.\n\
         Responda em markdown, em português, com exatamente estas seções:\n\n\
         ## Por quê\n## Feito\n## Falta\n## Próximo passo\n\n\
         Seja concreto e direto. Baseie-se APENAS nas evidências abaixo.\n\
         Se a evidência for insuficiente para uma seção, escreva \"evidência insuficiente\".\n\n\
         # Branch\n{key} (base: {default_branch})\n\n\
         # Commits (base..branch)\n{commits}\n\n\
         # Diffstat\n{diffstat}\n",
        key = lp.key(),
    );
    if excerpts.is_empty() {
        p.push_str("\n# Sessões de IA\nnenhuma encontrada\n");
    } else {
        for e in excerpts {
            p.push_str(&format!(
                "\n# Sessão {} (modificada {})\n{}\n",
                e.source,
                e.modified.format("%Y-%m-%d"),
                e.text
            ));
        }
    }
    p
}

/// Executa o comando LLM com o prompt em stdin e devolve o stdout.
///
/// O comando é interpretado via `sh -c`, portanto pode conter pipes e
/// redirecionamentos (ex.: `"claude -p | tee /tmp/saida.md"`).
///
/// # Errors
///
/// Retorna `Err` se o processo não puder ser iniciado ou se encerrar com
/// status diferente de zero (ex.: LLM não instalado, credencial ausente).
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
                "falha ao executar o comando LLM `{llm_command}` — \
                 está instalado? Ajuste llm_command no config.toml"
            )
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("stdin não disponível para o processo LLM"))?
        .write_all(prompt.as_bytes())
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(e).context("falha ao escrever o prompt no stdin do LLM")
            }
        })?;
    let out = child
        .wait_with_output()
        .context("falha ao aguardar o processo LLM")?;
    if !out.status.success() {
        bail!(
            "comando LLM falhou (`{llm_command}`): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Anexa a seção `## Fontes` ao documento gerado pelo LLM.
///
/// Permite ao usuário auditar as evidências usadas na reconstituição
/// (mitigação do risco de alucinação — ver spec §Riscos).
pub fn with_sources(answer: &str, lp: &OpenLoop, excerpts: &[SessionExcerpt]) -> String {
    let sha_curto = &lp.head_sha[..7.min(lp.head_sha.len())];
    let mut doc = format!(
        "# {}\n\n{}\n\n## Fontes\n\n- git: branch {} (HEAD {})\n",
        lp.key(),
        answer.trim(),
        lp.branch,
        sha_curto
    );
    for e in excerpts {
        doc.push_str(&format!(
            "- sessão: {} (modificada {})\n",
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
    fn build_prompt_inclui_evidencias_e_secoes() {
        let p = build_prompt(
            &fake_loop(),
            "main",
            "abc feat: wip",
            "x.txt | 2 +",
            &[fake_excerpt()],
        );
        assert!(p.contains("## Por quê"));
        assert!(p.contains("## Próximo passo"));
        assert!(p.contains("app/feat/login"));
        assert!(p.contains("abc feat: wip"));
        assert!(p.contains("[user] implementa login"));
    }

    #[test]
    fn build_prompt_sem_sessoes_declara_ausencia() {
        let p = build_prompt(&fake_loop(), "main", "", "", &[]);
        assert!(p.contains("nenhuma encontrada"));
    }

    #[test]
    fn run_llm_passa_prompt_via_stdin() {
        // `cat` ecoa o stdin: valida o contrato sem LLM real
        let out = run_llm("cat", "prompt de teste").unwrap();
        assert_eq!(out.trim(), "prompt de teste");
    }

    #[test]
    fn run_llm_erro_contextual_quando_comando_falha() {
        let err = run_llm("false", "x").unwrap_err();
        assert!(err.to_string().contains("comando LLM falhou"));
    }

    #[test]
    fn with_sources_anexa_git_e_sessoes() {
        let doc = with_sources("## Por quê\nlogin", &fake_loop(), &[fake_excerpt()]);
        assert!(doc.contains("## Fontes"));
        assert!(doc.contains("abcdef1")); // sha curto
        assert!(doc.contains("sessao1.jsonl"));
    }
}
