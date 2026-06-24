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
    assert!(ok, "git {args:?} falhou");
}

fn loops(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("loops").unwrap();
    cmd.env("OPEN_LOOPS_HOME", home);
    cmd
}

#[test]
fn fluxo_completo_init_list_resume_cache_ignore() {
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

    // init registra a raiz
    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projetos"))
        .assert()
        .success()
        .stdout(predicate::str::contains("raízes registradas"));

    // list mostra o loop aberto
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("meu-app/feat/login"));

    // troca o LLM por `cat`: resume imprime o prompt (que contém os commits)
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
        .stderr(predicate::str::contains("confiança baixa")); // sem sessões de IA no fixture

    // segunda chamada vem do cache: funciona mesmo com LLM quebrado
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

    // ignore remove da lista
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
fn resume_sem_match_orienta_usuario() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let raiz = tmp.path().join("projetos");
    std::fs::create_dir_all(&raiz).unwrap();
    loops(&home).arg("init").arg(&raiz).assert().success();
    loops(&home)
        .args(["resume", "nao-existe"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nenhum loop bate"));
}

#[test]
fn list_e_resume_sem_raizes_orienta_usuario() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    // sem init: Store::load retorna Config::default com roots vazio
    loops(&home)
        .assert()
        .failure()
        .stderr(predicate::str::contains("nenhuma raiz configurada"));
    loops(&home)
        .args(["resume", "qualquer"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nenhuma raiz configurada"));
}

#[test]
fn ignore_chave_sem_barra_rejeita_com_mensagem_util() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    loops(&home)
        .args(["ignore", "semslash"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("formato esperado: repo/branch"));
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
        .stderr(predicate::str::contains("query ambígua"))
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
        .stderr(predicate::str::contains("aviso"));
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
