//! Test helpers: temporary git repositories.
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
        .expect("git installed")
        .status;
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

/// Creates a repo with a main branch and 1 commit.
pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

/// Creates a branch from the current state with 1 exclusive commit.
pub fn add_branch_with_commit(dir: &Path, branch: &str, file: &str) {
    git(dir, &["checkout", "-b", branch]);
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", &format!("wip {branch}")]);
    git(dir, &["checkout", "main"]);
}

/// Creates a worktree at `path` on a new branch off the current HEAD (counts as merged).
pub fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    git(repo, &["worktree", "add", path.to_str().unwrap(), "-b", branch]);
}
