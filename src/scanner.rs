//! Descoberta de repositórios e branches não mergeadas via shell-out ao git.
//! Decisão de design: shell-out (não git2/gix) — simples e debugável;
//! o gargalo de performance do produto é o LLM, não o git.
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Executa um subcomando git em `repo` e devolve stdout aparado.
///
/// # Errors
///
/// Retorna `Err` se o git não estiver no PATH ou se o comando falhar.
pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("git não encontrado no PATH — instale o git")?;
    if !out.status.success() {
        bail!(
            "git {:?} falhou em {}: {}",
            args,
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Branch default: origin/HEAD se existir; senão main; senão master.
///
/// # Errors
///
/// Retorna `Err` se nenhuma branch default for encontrada.
pub fn default_branch(repo: &Path) -> Result<String> {
    if let Ok(sym) = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        if let Some(branch) = sym.strip_prefix("origin/") {
            return Ok(branch.to_string());
        }
    }
    for candidate in ["main", "master"] {
        if git(
            repo,
            &["rev-parse", "--verify", &format!("refs/heads/{candidate}")],
        )
        .is_ok()
        {
            return Ok(candidate.to_string());
        }
    }
    bail!(
        "não achei a branch default em {} (esperava origin/HEAD, main ou master)",
        repo.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn default_branch_detecta_main() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        assert_eq!(default_branch(&repo).unwrap(), "main");
    }

    #[test]
    fn git_falha_com_mensagem_contextual() {
        let tmp = tempfile::tempdir().unwrap();
        // diretório não é repo git
        let err = git(tmp.path(), &["status"]).unwrap_err();
        assert!(err.to_string().contains(&tmp.path().display().to_string()));
    }
}
