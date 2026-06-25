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
    git(
        repo,
        &["worktree", "add", path.to_str().unwrap(), "-b", branch],
    );
}

/// Creates a bare repository at `bare` (`git init --bare`).
pub fn init_bare_repo(bare: &Path) {
    std::fs::create_dir_all(bare).unwrap();
    git(bare, &["init", "--bare", "-b", "main"]);
}

/// Author layout: `container/.bare` + `container/.git` pointer + `main/` worktree with init commit.
pub fn init_bare_worktree_container(container: &Path) {
    std::fs::create_dir_all(container).unwrap();
    let bare = container.join(".bare");
    init_bare_repo(&bare);
    std::fs::write(container.join(".git"), "gitdir: ./.bare\n").unwrap();
    let main = container.join("main");
    git(
        container,
        &["worktree", "add", "-b", "main", main.to_str().unwrap()],
    );
    std::fs::write(main.join("README"), "init").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "init"]);
}

/// Adds `container/<name>/` worktree on a new branch (author layout).
pub fn add_named_worktree(container: &Path, name: &str, branch: &str) {
    let wt = container.join(name);
    git(
        container,
        &["worktree", "add", "-b", branch, wt.to_str().unwrap()],
    );
}

/// Creates `main` with one commit on a bare repo (no container pointer).
pub fn seed_bare_main(bare: &Path) {
    let tmp = bare.parent().unwrap().join("_seed");
    git(
        bare.parent().unwrap(),
        &["clone", bare.to_str().unwrap(), tmp.to_str().unwrap()],
    );
    std::fs::write(tmp.join("README"), "init").unwrap();
    git(&tmp, &["add", "."]);
    git(&tmp, &["commit", "-m", "init"]);
    git(&tmp, &["push", "origin", "main"]);
    std::fs::remove_dir_all(&tmp).ok();
}

/// Feature branch with exclusive commit, using a throwaway clone of `bare`.
pub fn add_branch_on_bare(bare: &Path, branch: &str, file: &str) {
    let tmp = bare.parent().unwrap().join("_wt");
    git(
        bare.parent().unwrap(),
        &["clone", bare.to_str().unwrap(), tmp.to_str().unwrap()],
    );
    add_branch_with_commit(&tmp, branch, file);
    git(&tmp, &["push", "origin", branch]);
    std::fs::remove_dir_all(&tmp).ok();
}
