//! Helpers de teste: repositórios git temporários.
use std::path::Path;
use std::process::Command;

pub fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git instalado")
        .status;
    assert!(
        status.success(),
        "git {args:?} falhou em {}",
        repo.display()
    );
}

/// Cria repo com branch main e 1 commit.
pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

/// Cria branch a partir do estado atual com 1 commit próprio.
pub fn add_branch_with_commit(dir: &Path, branch: &str, file: &str) {
    git(dir, &["checkout", "-b", branch]);
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", &format!("wip {branch}")]);
    git(dir, &["checkout", "main"]);
}
