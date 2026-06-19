//! Descoberta de repositórios e branches não mergeadas via shell-out ao git.
//! Decisão de design: shell-out (não git2/gix) — simples e debugável;
//! o gargalo de performance do produto é o LLM, não o git.
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
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

/// Um loop aberto: branch não mergeada com commits próprios.
#[derive(Debug, Clone)]
pub struct OpenLoop {
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub head_sha: String,
    pub last_commit: DateTime<Utc>,
    pub ahead: u32,
    pub behind: u32,
}

impl OpenLoop {
    /// Chave canônica usada em resume/ignore: "repo/branch".
    pub fn key(&self) -> String {
        format!("{}/{}", self.repo_name, self.branch)
    }
}

const MAX_DEPTH: usize = 3;
const SKIP_DIRS: [&str; 2] = ["node_modules", "target"];

/// Varre as raízes até MAX_DEPTH procurando diretórios com .git.
///
/// Diretórios ocultos (nome começa com `.`) e os listados em `SKIP_DIRS`
/// são ignorados.
pub fn find_repos(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for root in roots {
        walk(root, 0, &mut repos);
    }
    repos.sort();
    repos
}

fn walk(dir: &Path, depth: usize, repos: &mut Vec<PathBuf>) {
    if dir.join(".git").is_dir() {
        repos.push(dir.to_path_buf());
        return;
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !path.is_dir() || name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        walk(&path, depth + 1, repos);
    }
}

/// Retorna todas as branches não mergeadas (exceto a default) de um repo.
///
/// # Errors
///
/// Retorna `Err` se o git falhar ou se a branch default não for encontrada.
pub fn open_loops(repo: &Path) -> Result<Vec<OpenLoop>> {
    let default = default_branch(repo)?;
    let merged: std::collections::HashSet<String> = git(
        repo,
        &["branch", "--merged", &default, "--format=%(refname:short)"],
    )?
    .lines()
    .map(|s| s.trim().to_string())
    .collect();
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string());
    let raw = git(
        repo,
        &[
            "for-each-ref",
            "refs/heads",
            "--format=%(refname:short)%09%(objectname)%09%(committerdate:iso8601-strict)",
        ],
    )?;
    let mut result = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split('\t');
        let (Some(branch), Some(sha), Some(date)) = (parts.next(), parts.next(), parts.next())
        else {
            eprintln!("aviso: linha inesperada do git for-each-ref ignorada: {line:?}");
            continue;
        };
        if branch == default || merged.contains(branch) {
            continue;
        }
        let counts = git(
            repo,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{default}...{branch}"),
            ],
        )?;
        let mut c = counts.split_whitespace();
        let behind: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
        let ahead: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
        let last_commit = DateTime::parse_from_rfc3339(date)
            .with_context(|| format!("data inválida vinda do git: {date}"))?
            .with_timezone(&Utc);
        result.push(OpenLoop {
            repo_name: repo_name.clone(),
            repo_path: repo.to_path_buf(),
            branch: branch.to_string(),
            head_sha: sha.to_string(),
            last_commit,
            ahead,
            behind,
        });
    }
    Ok(result)
}

/// Varre todos os repos encontrados nas raízes em paralelo.
///
/// Falhas em repos individuais viram warnings, nunca abortam a varredura.
pub fn scan(roots: &[PathBuf]) -> (Vec<OpenLoop>, Vec<String>) {
    let repos = find_repos(roots);
    let results: Vec<Result<Vec<OpenLoop>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| s.spawn(move || open_loops(repo)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("panic ao escanear o repositório")))
            })
            .collect()
    });
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut loops) => all.append(&mut loops),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
        }
    }
    (all, warnings)
}

/// Commits exclusivos da branch em relação à default (para o prompt de destilação).
///
/// # Errors
///
/// Retorna `Err` se o git falhar.
pub fn git_log(repo: &Path, default: &str, branch: &str) -> Result<String> {
    git(repo, &["log", "--oneline", &format!("{default}..{branch}")])
}

/// Diffstat da branch contra a base (para o prompt de destilação).
///
/// # Errors
///
/// Retorna `Err` se o git falhar.
pub fn diffstat(repo: &Path, default: &str, branch: &str) -> Result<String> {
    git(repo, &["diff", "--stat", &format!("{default}...{branch}")])
}

/// Janela temporal dos commits exclusivos da branch.
///
/// Útil para filtrar sessões de IA muito antigas.
///
/// # Errors
///
/// Retorna `Err` se o git falhar ou se não houver commits na branch.
pub fn commit_window(
    repo: &Path,
    default: &str,
    branch: &str,
) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let raw = git(
        repo,
        &["log", "--format=%cI", &format!("{default}..{branch}")],
    )?;
    let mut dates: Vec<DateTime<Utc>> = raw
        .lines()
        .filter_map(|l| DateTime::parse_from_rfc3339(l.trim()).ok())
        .map(|d| d.with_timezone(&Utc))
        .collect();
    if dates.is_empty() {
        // branch sem commit próprio: usa o último commit dela
        let head = git(repo, &["log", "-1", "--format=%cI", branch])?;
        dates.push(DateTime::parse_from_rfc3339(head.trim())?.with_timezone(&Utc));
    }
    let min = dates
        .iter()
        .min()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("sem datas de commit para {branch}"))?;
    let max = dates
        .iter()
        .max()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("sem datas de commit para {branch}"))?;
    Ok((min, max))
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

    #[test]
    fn find_repos_acha_repos_ate_profundidade_3_e_pula_ocultos() {
        let tmp = tempfile::tempdir().unwrap();
        testutil::init_repo(&tmp.path().join("a/b/repo1"));
        testutil::init_repo(&tmp.path().join("repo2"));
        testutil::init_repo(&tmp.path().join(".oculto/repo3"));
        let repos = find_repos(&[tmp.path().to_path_buf()]);
        let names: Vec<_> = repos
            .iter()
            .map(|r| r.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"repo1".to_string()));
        assert!(names.contains(&"repo2".to_string()));
        assert!(!names.contains(&"repo3".to_string()));
    }

    #[test]
    fn open_loops_acha_nao_mergeada_ignora_mergeada_e_default() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        testutil::git(&repo, &["branch", "mergeada"]); // aponta para main => mergeada

        let loops = open_loops(&repo).unwrap();
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.branch, "feat/x");
        assert_eq!(l.repo_name, "app");
        assert_eq!(l.key(), "app/feat/x");
        assert_eq!(l.ahead, 1);
        assert_eq!(l.behind, 0);
        assert_eq!(l.head_sha.len(), 40);
    }

    #[test]
    fn scan_agrega_repos_e_reporta_warning_sem_abortar() {
        let tmp = tempfile::tempdir().unwrap();
        let bom = tmp.path().join("bom");
        testutil::init_repo(&bom);
        testutil::add_branch_with_commit(&bom, "feat/ok", "ok.txt");
        // repo quebrado de verdade: repo sem nenhum commit (default_branch falha)
        let vazio = tmp.path().join("vazio");
        std::fs::create_dir_all(&vazio).unwrap();
        testutil::git(&vazio, &["init", "-b", "main"]);

        let (loops, warnings) = scan(&[tmp.path().to_path_buf()]);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].key(), "bom/feat/ok");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("vazio"));
    }

    #[test]
    fn helpers_de_contexto_retornam_commits_e_janela() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let log = git_log(&repo, "main", "feat/x").unwrap();
        assert!(log.contains("wip feat/x"));
        let stat = diffstat(&repo, "main", "feat/x").unwrap();
        assert!(stat.contains("x.txt"));
        let (ini, fim) = commit_window(&repo, "main", "feat/x").unwrap();
        assert!(ini <= fim);
    }

    #[test]
    fn default_branch_detecta_master_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        testutil::git(repo, &["init", "-b", "master"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        testutil::git(repo, &["add", "."]);
        testutil::git(repo, &["commit", "-m", "init"]);
        assert_eq!(default_branch(repo).unwrap(), "master");
    }

    #[test]
    fn default_branch_erro_sem_main_nem_master() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        testutil::git(repo, &["init", "-b", "trunk"]);
        // sem commits: refs/heads/main e refs/heads/master não existem
        let err = default_branch(repo).unwrap_err();
        assert!(err.to_string().contains("não achei a branch default"));
    }
}
