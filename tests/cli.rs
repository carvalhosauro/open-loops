//! E2E: real binary, real git repos, LLM replaced by `cat`.
use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn git(repo: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

fn loops(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("loops").unwrap();
    cmd.env("OPEN_LOOPS_HOME", home);
    cmd
}

#[test]
fn full_flow_init_list_resume_cache_ignore() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("projetos/meu-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    git(&repo, &["checkout", "-b", "feat/login"]);
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat: login wip"]);

    // init registers the root
    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projetos"))
        .assert()
        .success()
        .stdout(predicate::str::contains("roots registered"));

    // list shows the open loop
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("meu-app/feat/login"));

    // swap the LLM for `cat`: resume prints the prompt (which contains the commits)
    let cfg_path = home.join("config.toml");
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(
        &cfg_path,
        cfg.replace("llm_command = \"claude -p\"", "llm_command = \"cat\""),
    )
    .unwrap();

    loops(&home)
        .args(["resume", "feat/login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("feat: login wip"))
        .stdout(predicate::str::contains("## Sources"))
        .stdout(predicate::str::contains("**Confidence:** low"));

    // dry-run shows evidence without calling the LLM
    loops(&home)
        .args(["resume", "feat/login", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run — LLM not invoked"))
        .stdout(predicate::str::contains("feat: login wip"))
        .stdout(predicate::str::contains("**Confidence:** low"));

    // second call comes from cache: works even with a broken LLM
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(
        &cfg_path,
        cfg.replace("llm_command = \"cat\"", "llm_command = \"false\""),
    )
    .unwrap();
    loops(&home)
        .args(["resume", "feat/login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Sources"));

    // ignore removes from the list
    loops(&home)
        .args(["ignore", "projetos/meu-app/feat/login"])
        .assert()
        .success();
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("feat/login").not());
}

#[test]
fn resume_no_match_guides_user() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    std::fs::create_dir_all(&root).unwrap();
    loops(&home).arg("init").arg(&root).assert().success();
    loops(&home)
        .args(["resume", "does-not-exist"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no loop matches"));
}

#[test]
fn list_and_resume_without_roots_guides_user() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // no init: Store::load returns Config::default with empty roots
    loops(&home)
        .assert()
        .failure()
        .stderr(predicate::str::contains("no roots configured"));
    loops(&home)
        .args(["resume", "anything"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no roots configured"));
}

#[test]
fn ignore_key_without_slash_rejects_with_helpful_message() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    loops(&home)
        .args(["ignore", "noslash"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("expected format: repo/branch"));
}

#[test]
fn resume_ambiguous_query_lists_candidates() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("projetos/app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);

    git(&repo, &["checkout", "-b", "feat/login"]);
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat: login"]);

    git(&repo, &["checkout", "main"]);
    git(&repo, &["checkout", "-b", "feat/signup"]);
    std::fs::write(repo.join("c.txt"), "c").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat: signup"]);

    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projetos"))
        .assert()
        .success();

    loops(&home)
        .args(["resume", "feat"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ambiguous query"))
        .stderr(predicate::str::contains("app/feat/login"))
        .stderr(predicate::str::contains("app/feat/signup"));
}

#[test]
fn list_prints_warnings_for_broken_repos() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");

    // repo with no commits: default_branch fails -> scan emits a warning
    let empty = root.join("vazio");
    std::fs::create_dir_all(&empty).unwrap();
    git(&empty, &["init", "-b", "main"]);

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .assert()
        .success()
        .stderr(predicate::str::contains("warning"));
}

#[test]
fn completions_generates_script_for_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    loops(&home)
        .arg("completions")
        .arg("bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("loops"));
}

/// Builds a git repo at `repo` (main + 1 commit) and returns its path ready for worktrees.
fn init_repo(repo: &std::path::Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "init"]);
}

#[test]
fn worktrees_aggregates_across_multiple_repos() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    for (i, name) in ["app-a", "app-b"].iter().enumerate() {
        let repo = root.join(name);
        init_repo(&repo);
        let wt = tmp.path().join(format!("wt-{i}"));
        git(
            &repo,
            &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"],
        );
    }
    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("app-a/wt-0"))
        .stdout(predicate::str::contains("app-b/wt-1"))
        // one cleanup command per deletable worktree
        .stdout(predicate::str::contains("2 worktree(s) to clean up"));
}

#[test]
fn worktrees_never_suggests_removing_unmerged_or_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("app");
    init_repo(&repo);

    // cold: unmerged branch with its own commit, clean
    let cold = tmp.path().join("wt-cold");
    git(
        &repo,
        &["worktree", "add", cold.to_str().unwrap(), "-b", "feat/cold"],
    );
    std::fs::write(cold.join("c.txt"), "c").unwrap();
    git(&cold, &["add", "."]);
    git(&cold, &["commit", "-m", "wip"]);

    // dirty: branch off main with an uncommitted file
    let dirty = tmp.path().join("wt-dirty");
    git(
        &repo,
        &[
            "worktree",
            "add",
            dirty.to_str().unwrap(),
            "-b",
            "feat/dirty",
        ],
    );
    std::fs::write(dirty.join("d.txt"), "d").unwrap();

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("cold"))
        .stdout(predicate::str::contains("active"))
        // safety: no destructive command suggested for live/unmerged work
        .stdout(predicate::str::contains("nothing to clean up"))
        .stdout(predicate::str::contains("worktree remove").not());
}

#[test]
fn worktrees_output_is_ascii() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("app");
    init_repo(&repo);
    let wt = tmp.path().join("wt");
    git(
        &repo,
        &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"],
    );
    loops(&home).arg("init").arg(&root).assert().success();

    let out = loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(out.is_ascii(), "worktrees output must be ASCII-only");
}

#[test]
fn worktrees_clean_environment_has_no_false_positive() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("app");
    init_repo(&repo); // only the main worktree
    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("home"))
        .stdout(predicate::str::contains("nothing to clean up"))
        .stdout(predicate::str::contains("worktree remove").not());
}

#[test]
fn completions_for_zsh_and_fish_are_nonempty() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    for shell in ["zsh", "fish"] {
        loops(&home)
            .arg("completions")
            .arg(shell)
            .assert()
            .success()
            .stdout(predicate::str::contains("loops"));
    }
}

#[test]
fn worktrees_lists_and_suggests_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projetos");
    let repo = root.join("meu-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    // merged worktree (new branch off main) and clean => deletable
    let wt = tmp.path().join("wt-done");
    git(
        &repo,
        &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"],
    );

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("deletable"))
        .stdout(predicate::str::contains("worktree remove"));

    // the wt alias works
    loops(&home).arg("wt").assert().success();
}

#[test]
fn list_filters_by_query_term() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    for name in ["api", "web"] {
        let repo = root.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "init"]);
        git(&repo, &["checkout", "-b", "feat/x"]);
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "wip"]);
    }
    loops(&home).arg("init").arg(&root).assert().success();

    // bare `loops` shows both; `loops api` shows only api, with 3-segment key
    loops(&home)
        .arg("api")
        .assert()
        .success()
        .stdout(predicate::str::contains("projects/api/feat/x"))
        .stdout(predicate::str::contains("web/feat/x").not());
}
