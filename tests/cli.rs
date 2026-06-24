//! E2E: binário real, repos git reais, LLM substituído por `cat`.
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
        .stdout(predicate::str::contains("## Fontes"))
        .stderr(predicate::str::contains("low confidence")); // no AI sessions in the fixture

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
        .stdout(predicate::str::contains("## Fontes"));

    // ignore removes from the list
    loops(&home)
        .args(["ignore", "meu-app/feat/login"])
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
fn resume_query_ambigua_lista_candidatos() {
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
fn list_imprime_warnings_de_repos_quebrados() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let raiz = tmp.path().join("projetos");

    // repo sem commits: default_branch falha → scan gera warning
    let vazio = raiz.join("vazio");
    std::fs::create_dir_all(&vazio).unwrap();
    git(&vazio, &["init", "-b", "main"]);

    loops(&home).arg("init").arg(&raiz).assert().success();

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
    // worktree mergeada (branch nova off main) e limpa => deletable
    let wt = tmp.path().join("wt-done");
    git(&repo, &["worktree", "add", wt.to_str().unwrap(), "-b", "fix/done"]);

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .arg("worktrees")
        .assert()
        .success()
        .stdout(predicate::str::contains("deletable"))
        .stdout(predicate::str::contains("worktree remove"));

    // alias wt funciona
    loops(&home).arg("wt").assert().success();
}
