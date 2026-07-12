//! E2E: real binary, real git repos, LLM replaced by `cat`.
use assert_cmd::Command;
use open_loops::config::{ContextDef, Store};
use open_loops::sessions::claude_code::encode_project_path;
use open_loops::state::State;
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

/// TOML basic strings treat `\` as escape; use forward slashes (valid on all OSes).
fn toml_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
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

/// Regression: `main.rs` must print the full `source()` chain (anyhow `{:#}`
/// parity). With bare `Display` on thiserror types, the TOML cause vanished.
#[test]
fn invalid_config_toml_reports_root_cause() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "not valid toml [[[").unwrap();
    loops(&home)
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid config.toml"))
        .stderr(predicate::str::contains("TOML parse error"));
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

    // The warning now travels through `tracing` (level WARN), so assert on the
    // stable message text rather than a "warning:" prefix.
    loops(&home)
        .assert()
        .success()
        .stderr(predicate::str::contains("default branch"));
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

#[test]
fn list_path_flag_shows_worktree_path_column() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("projetos/app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    git(&repo, &["checkout", "-b", "feat/x"]);
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "wip"]);

    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projetos"))
        .assert()
        .success();

    // Without --path: no PATH column.
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("PATH").not());
    // With --path: PATH column present and the repo dir shown.
    loops(&home)
        .arg("--path")
        .assert()
        .success()
        .stdout(predicate::str::contains("PATH"))
        .stdout(predicate::str::contains("projetos/app"));
}

#[test]
fn list_finds_branches_in_bare_worktree_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    let container = root.join("my-app");
    std::fs::create_dir_all(&container).unwrap();

    // inline git setup (tests/cli.rs has its own git helper)
    let bare = container.join(".bare");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-b", "main"]);
    std::fs::write(container.join(".git"), "gitdir: ./.bare\n").unwrap();
    let main = container.join("main");
    git(
        &container,
        &["worktree", "add", "-b", "main", main.to_str().unwrap()],
    );
    std::fs::write(main.join("a.txt"), "a").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "init"]);
    git(&main, &["checkout", "-b", "feat/login"]);
    std::fs::write(main.join("b.txt"), "b").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "feat"]);
    git(&main, &["checkout", "main"]);

    loops(&home).arg("init").arg(&root).assert().success();

    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("my-app/feat/login"));
}

#[test]
fn resume_includes_session_excerpt_for_branch_in_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    let container = root.join("my-app");

    // bare + worktree container (inline git: tests/cli.rs has its own git helper)
    let bare = container.join(".bare");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-b", "main"]);
    std::fs::write(container.join(".git"), "gitdir: ./.bare\n").unwrap();
    let main = container.join("main");
    git(
        &container,
        &["worktree", "add", "-b", "main", main.to_str().unwrap()],
    );
    std::fs::write(main.join("a.txt"), "a").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-m", "init"]);
    // feature branch checked out in its OWN worktree directory
    let feat = container.join("feat-login");
    git(
        &container,
        &[
            "worktree",
            "add",
            "-b",
            "feat/login",
            feat.to_str().unwrap(),
        ],
    );
    std::fs::write(feat.join("b.txt"), "b").unwrap();
    git(&feat, &["add", "."]);
    git(&feat, &["commit", "-m", "feat: login wip"]);

    // fake Claude Code session under the ENCODED WORKTREE path (not the container)
    let sessions = tmp.path().join("ai-sessions");
    let feat_path = std::fs::canonicalize(&feat).unwrap_or(feat);
    let proj = sessions.join(encode_project_path(&feat_path));
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("s.jsonl"),
        concat!(
            r#"{"type":"user","message":{"content":"resume the login feature please"}}"#,
            "\n",
        ),
    )
    .unwrap();

    loops(&home).arg("init").arg(&root).assert().success();

    // point llm_command at `cat` and sessions_dir at our fake projects dir (in place)
    let cfg_path = home.join("config.toml");
    let raw = std::fs::read_to_string(&cfg_path).unwrap();
    let rewritten: String = raw
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("sessions_dir") {
                format!("sessions_dir = \"{}\"", toml_path(&sessions))
            } else if l.trim_start().starts_with("llm_command") {
                "llm_command = \"cat\"".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&cfg_path, rewritten + "\n").unwrap();

    // resume distills (cat echoes the prompt) AND carries the worktree session excerpt
    loops(&home)
        .args(["resume", "feat/login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("resume the login feature please"));
}

#[test]
fn inventory_write_through_on_list() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("projects/my-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    git(&repo, &["checkout", "-b", "feat/cache-me"]);
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat: cache me"]);

    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projects"))
        .assert()
        .success();

    // First `loops`: inventory file must be created under home/inventory/.
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("my-app/feat/cache-me"));

    let inv_dir = home.join("inventory");
    assert!(
        inv_dir.exists(),
        "inventory dir should be created after first scan"
    );
    let entries: Vec<_> = std::fs::read_dir(&inv_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(entries.len(), 1, "exactly one inventory JSON expected");

    // Verify the JSON contains the expected fields.
    let json_raw = std::fs::read_to_string(entries[0].path()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&json_raw).unwrap();
    assert!(json["repo_path"].is_string(), "repo_path must be present");
    assert!(json["indexed_at"].is_string(), "indexed_at must be present");
    let loops_arr = json["loops"].as_array().unwrap();
    assert_eq!(loops_arr.len(), 1);
    let memo = &loops_arr[0];
    assert_eq!(memo["branch"].as_str().unwrap(), "feat/cache-me");
    assert!(memo["head_sha"].is_string());
    assert!(memo["ab_base_sha"].is_string());
    assert_eq!(memo["ahead"].as_u64().unwrap(), 1);
    assert_eq!(memo["behind"].as_u64().unwrap(), 0);

    // `loops --fresh` must still work (bypasses memo but rewrites the file).
    loops(&home)
        .arg("--fresh")
        .assert()
        .success()
        .stdout(predicate::str::contains("feat/cache-me"));

    // `loops refresh` must print "refreshed N repos" on stderr.
    loops(&home)
        .arg("refresh")
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));
}

/// The SQLite index is created live on the first scan and a corrupt/deleted db
/// self-heals on the next run (git is the source of truth; the index is
/// disposable). Deleting `index.db` between two runs must be transparent.
#[test]
fn index_db_created_and_self_heals_on_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    let repo = projects.join("my-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    git(&repo, &["checkout", "-b", "feat/heal"]);
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat"]);

    loops(&home).arg("init").arg(&projects).assert().success();

    // First scan: the index db must be created live.
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("my-app/feat/heal"));
    let db_path = home.join("index.db");
    assert!(
        db_path.exists(),
        "index.db must be created on the first scan"
    );

    // Corrupt then delete the db — the next run must rebuild it transparently and
    // still list the loop (no error, exit 0).
    std::fs::write(&db_path, b"not a sqlite database").unwrap();
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("my-app/feat/heal"));
    assert!(db_path.exists(), "corrupt index.db must be rebuilt");

    std::fs::remove_file(&db_path).unwrap();
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("my-app/feat/heal"));
    assert!(db_path.exists(), "deleted index.db must self-heal");
}

/// `loops refresh` reclaims index rows for a repo gone from disk, mirroring the
/// inventory orphan prune. A live repo's rows must survive.
#[test]
fn refresh_prunes_index_rows_for_disk_gone_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    repo_with_feature(&projects, "keep-me");
    repo_with_feature(&projects, "delete-me");

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    // Both repos must be indexed.
    let count = |home: &Path| -> i64 {
        let conn =
            rusqlite::Connection::open(home.join("index.db")).expect("open index for assertion");
        conn.query_row("SELECT COUNT(*) FROM repos", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count(&home), 2, "both repos indexed after first scan");

    // delete-me disappears from disk; refresh must reclaim its index row.
    std::fs::remove_dir_all(projects.join("delete-me")).unwrap();
    loops(&home)
        .arg("refresh")
        .assert()
        .success()
        .stderr(predicate::str::contains("removed orphan index entry"));
    assert_eq!(count(&home), 1, "disk-gone repo's index row must be pruned");
}

#[test]
fn scoped_refresh_does_not_prune_other_inventory() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let api = tmp.path().join("projects/api-service");
    let web = tmp.path().join("projects/web-app");
    for repo in [&api, &web] {
        std::fs::create_dir_all(repo).unwrap();
        git(repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "init"]);
        git(repo, &["checkout", "-b", "feat/x"]);
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "feat"]);
    }

    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projects"))
        .assert()
        .success();

    loops(&home).assert().success();

    let inv_dir = home.join("inventory");
    assert_eq!(
        count_inventory_json(&inv_dir),
        2,
        "expected one inventory file per repo"
    );

    loops(&home)
        .args(["refresh", "repo:api"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));

    assert_eq!(
        count_inventory_json(&inv_dir),
        2,
        "scoped refresh must not delete inventory for repos outside the query"
    );
}

#[test]
fn resume_dry_run_skips_ahead_behind_without_attr_filter() {
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

    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projetos"))
        .assert()
        .success();

    loops(&home)
        .args(["resume", "feat/login", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ahead: -, behind: -"));
}

/// Creates `<root>/<name>` as a git repo on `main` plus one unmerged `feat/x`
/// branch carrying a single extra commit (ahead 1, behind 0). File contents are
/// keyed by `name` so each repo has distinct commit SHAs (identical trees +
/// same-second commits would otherwise collide, which never happens for real
/// repos but would for fixtures built in a tight loop).
fn repo_with_feature(root: &Path, name: &str) {
    let repo = root.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), format!("base-{name}")).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    git(&repo, &["checkout", "-b", "feat/x"]);
    std::fs::write(repo.join("b.txt"), format!("feat-{name}")).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat"]);
    // Leave the repo on its default branch so callers that add worktrees or set
    // origin/HEAD start from a clean main checkout.
    git(&repo, &["checkout", "main"]);
}

/// Counts inventory `*.json` files in `dir` (0 if the dir is absent).
fn count_inventory_json(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

/// `loops refresh <bare-word>` must scope the reindex to repos the same query
/// would list — not reindex every repo. Regression for the bug where bare terms
/// (which only filter in memory) were ignored by `run_refresh`, so the push-down
/// filter was `None` and all repos were rewritten.
#[test]
fn refresh_bare_term_scopes_to_matching_repos() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    for name in ["alpha", "beta", "gamma"] {
        repo_with_feature(&projects, name);
    }

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    // Bare term "beta" matches only the beta repo → reindex exactly one.
    loops(&home)
        .args(["refresh", "beta"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));

    // The explicit repo: filter must agree.
    loops(&home)
        .args(["refresh", "repo:beta"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));

    // Empty query still reindexes everything.
    loops(&home)
        .arg("refresh")
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 3 repos"));
}

/// A disk-gone repo's inventory is reclaimed on ANY refresh, regardless of the
/// query scope — `prune_orphans` is a deliberate global GC (commit 948446c).
/// This exercises the disk-gone-out-of-scope path that
/// `scoped_refresh_does_not_prune_other_inventory` (on-disk only) cannot.
#[test]
fn refresh_prunes_disk_gone_repo_even_when_out_of_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    for name in ["api-1", "api-2", "web-1"] {
        repo_with_feature(&projects, name);
    }

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    let inv_dir = home.join("inventory");
    assert_eq!(
        count_inventory_json(&inv_dir),
        3,
        "one inventory file per repo"
    );

    // web-1 disappears from disk; it is now an orphan.
    std::fs::remove_dir_all(projects.join("web-1")).unwrap();

    // Scoped refresh that never scans web-1 still reclaims its orphan memo.
    loops(&home)
        .args(["refresh", "repo:api"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 2 repos"))
        .stderr(predicate::str::contains("removed orphan inventory"));

    assert_eq!(
        count_inventory_json(&inv_dir),
        2,
        "disk-gone web-1 inventory must be pruned"
    );
}

/// Proves the memo is actually read on a warm scan and that `--fresh` bypasses
/// it: poison the cached `ahead` to 99, then use the query engine as an oracle —
/// `loops ahead:99` matches only if the (wrong) memo was served, and
/// `loops --fresh ahead:99` recomputes the real value (1) so nothing matches.
#[test]
fn cache_hit_serves_memo_and_fresh_recomputes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    repo_with_feature(&projects, "app");

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    // The SQLite index gate is now the authoritative warm cache: on a second scan
    // with unchanged refs it serves the cached loops and the heavy git phase
    // (incl. the JSON inventory memo) never runs. So poison the cached `ahead` in
    // BOTH the SQLite index and the JSON inventory, preserving each store's
    // validation keys so the warm cache still validates and gets served.
    let inv_dir = home.join("inventory");
    let inv_file = std::fs::read_dir(&inv_dir)
        .unwrap()
        .flatten()
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .unwrap()
        .path();
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&inv_file).unwrap()).unwrap();
    json["loops"][0]["ahead"] = serde_json::json!(99);
    std::fs::write(&inv_file, serde_json::to_string(&json).unwrap()).unwrap();

    // Poison the SQLite gate's cached ahead, leaving refs_fingerprint/default_sha
    // intact so the gate still hits on the warm scan.
    {
        let conn = rusqlite::Connection::open(home.join("index.db")).expect("open index to poison");
        let n = conn
            .execute("UPDATE loops SET ahead = 99", [])
            .expect("poison sqlite ahead");
        assert_eq!(n, 1, "exactly one cached loop row to poison");
    }

    // Warm scan serves the poisoned cache → ahead:99 matches.
    loops(&home)
        .arg("ahead:99")
        .assert()
        .success()
        .stdout(predicate::str::contains("feat/x"));

    // --fresh ignores the caches and recomputes ahead=1 → ahead:99 matches nothing.
    loops(&home)
        .args(["--fresh", "ahead:99"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No loops match"));
}

/// `loops refresh branch:<x>` is an in-memory filter, so run_refresh must scope
/// the reindex by it too (not just `repo:`/`root:`).
#[test]
fn refresh_branch_filter_scopes_to_matching_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    // Two repos with DISTINCT branch names so branch: selects exactly one.
    for (name, branch) in [("api", "feat/login"), ("web", "feat/cart")] {
        let repo = projects.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), format!("base-{name}")).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "init"]);
        git(&repo, &["checkout", "-b", branch]);
        std::fs::write(repo.join("b.txt"), format!("feat-{name}")).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "feat"]);
    }

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    loops(&home)
        .args(["refresh", "branch:login"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));
}

/// A query that matches nothing reindexes nothing (and does not error/panic).
#[test]
fn refresh_no_match_reindexes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    for name in ["alpha", "beta"] {
        repo_with_feature(&projects, name);
    }

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    loops(&home)
        .args(["refresh", "zzz-no-such-repo-or-branch"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 0 repos"));
}

/// Two worktrees of one repo share a single common-dir, so they must map to
/// exactly ONE inventory file (the worktree-safety invariant run_refresh's
/// HEAD-sha scoping relies on). Both unmerged branches must still be listed.
#[test]
fn worktrees_of_one_repo_share_a_single_inventory_file() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    // Primary checkout: main + one unmerged feat/x, left on main.
    repo_with_feature(&root, "app");
    let repo = root.join("app");
    // Second unmerged branch in a linked worktree (shares the common-dir).
    let wt = tmp.path().join("wt-y");
    git(
        &repo,
        &["worktree", "add", wt.to_str().unwrap(), "-b", "feat/y"],
    );
    std::fs::write(wt.join("c.txt"), "c").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "feat y"]);

    loops(&home).arg("init").arg(&root).assert().success();
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("feat/x"))
        .stdout(predicate::str::contains("feat/y"));

    assert_eq!(
        count_inventory_json(&home.join("inventory")),
        1,
        "two worktrees of one repo must share a single inventory file"
    );
}

/// BUG-2 regression: concurrent `loops --fresh` processes writing the same
/// inventory must not race on the tmp file. Asserts every process exits 0, no
/// `.tmp` is left behind, and every inventory JSON parses.
#[test]
fn inventory_writes_survive_concurrent_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    for name in ["api", "web", "cli"] {
        repo_with_feature(&projects, name);
    }
    loops(&home).arg("init").arg(&projects).assert().success();

    let bin = env!("CARGO_BIN_EXE_loops");
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let home = home.clone();
            let bin = bin.to_string();
            std::thread::spawn(move || {
                std::process::Command::new(bin)
                    .env("OPEN_LOOPS_HOME", &home)
                    .arg("--fresh")
                    .output()
                    .unwrap()
            })
        })
        .collect();
    for h in handles {
        let out = h.join().unwrap();
        assert!(out.status.success(), "concurrent scan exited non-zero");
    }

    let inv_dir = home.join("inventory");
    let mut tmp_left = 0;
    for entry in std::fs::read_dir(&inv_dir).unwrap().flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("tmp") => tmp_left += 1,
            Some("json") => {
                let raw = std::fs::read_to_string(&path).unwrap();
                serde_json::from_str::<serde_json::Value>(&raw)
                    .unwrap_or_else(|e| panic!("corrupt inventory {}: {e}", path.display()));
            }
            _ => {}
        }
    }
    assert_eq!(tmp_left, 0, "no .tmp file should survive concurrent writes");
    assert_eq!(
        count_inventory_json(&inv_dir),
        3,
        "all three repos' inventory must be present and valid"
    );
}

/// A stale / single-branch `origin/HEAD` that points at a branch with no local
/// ref must NOT hide the repo: default-branch detection falls back to main.
#[test]
fn stale_origin_head_falls_back_to_main() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = tmp.path().join("projects");
    repo_with_feature(&root, "app");
    let repo = root.join("app");
    // origin/HEAD points at a branch with no local ref (stale pointer).
    git(
        &repo,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/ghost",
        ],
    );

    loops(&home).arg("init").arg(&root).assert().success();

    // The repo must still appear (fell back to main), not vanish.
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("app/feat/x"));
}

/// A corrupt inventory file for a repo that IS in the refresh scope is rewritten
/// valid by write-through BEFORE prune runs, so it survives (not reclaimed). The
/// prune-as-unreadable path only fires for files outside the scanned set.
#[test]
fn refresh_rewrites_corrupt_inventory_for_in_scope_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let projects = tmp.path().join("projects");
    repo_with_feature(&projects, "app");

    loops(&home).arg("init").arg(&projects).assert().success();
    loops(&home).assert().success();

    // Corrupt the cached inventory for the (live, in-scope) repo.
    let inv_dir = home.join("inventory");
    let inv_file = std::fs::read_dir(&inv_dir)
        .unwrap()
        .flatten()
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .unwrap()
        .path();
    std::fs::write(&inv_file, b"{ corrupt").unwrap();

    // refresh scans the repo → write-through rewrites the file valid before prune.
    loops(&home).arg("refresh").assert().success();

    assert_eq!(
        count_inventory_json(&inv_dir),
        1,
        "in-scope file must survive"
    );
    let raw = std::fs::read_to_string(&inv_file).unwrap();
    serde_json::from_str::<serde_json::Value>(&raw)
        .expect("inventory must be valid JSON again after refresh");
}

/// Merges `[contexts.*]` into config and optional `current_context` into state.toml.
fn setup_contexts_config(
    home: &Path,
    work_root: &Path,
    personal_root: &Path,
    current: Option<&str>,
    extra: &[(&str, &str)],
) {
    let store = Store::new(home.to_path_buf());
    let mut cfg = store.load().unwrap();
    cfg.contexts.insert(
        "work".into(),
        ContextDef {
            filter: format!("root:{}", toml_path(work_root)),
        },
    );
    cfg.contexts.insert(
        "personal".into(),
        ContextDef {
            filter: format!("root:{}", toml_path(personal_root)),
        },
    );
    for (name, filter) in extra {
        cfg.contexts.insert(
            (*name).into(),
            ContextDef {
                filter: (*filter).to_string(),
            },
        );
    }
    store.save(&cfg).unwrap();

    if let Some(name) = current {
        let mut state = State::load(home).unwrap();
        state.set_current_context(Some(name.to_string())).unwrap();
    }
}

/// Creates `work/` and `personal/` roots each holding one open-loop repo; returns keys.
fn init_two_root_fixture(tmp: &Path, home: &Path) -> (String, String) {
    let work_root = tmp.join("work");
    let personal_root = tmp.join("personal");
    repo_with_feature(&work_root, "work-app");
    repo_with_feature(&personal_root, "personal-app");

    loops(home)
        .args([
            "init",
            work_root.to_str().unwrap(),
            personal_root.to_str().unwrap(),
        ])
        .assert()
        .success();

    (
        "work/work-app/feat/x".to_string(),
        "personal/personal-app/feat/x".to_string(),
    )
}

fn git_commit_with_date(repo: &Path, date: &str, message: &str) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["commit", "-m", message])
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "dated git commit failed");
}

#[test]
fn context_default_scopes_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work_root = tmp.path().join("work");
    let personal_root = tmp.path().join("personal");
    let (work_key, personal_key) = init_two_root_fixture(tmp.path(), &home);
    setup_contexts_config(&home, &work_root, &personal_root, Some("work"), &[]);

    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains(&work_key))
        .stdout(predicate::str::contains(&personal_key).not());

    loops(&home)
        .arg("@personal")
        .assert()
        .success()
        .stdout(predicate::str::contains(&personal_key))
        .stdout(predicate::str::contains(&work_key).not());

    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains(&personal_key))
        .stdout(predicate::str::contains(&work_key).not());

    loops(&home)
        .arg("@none")
        .assert()
        .success()
        .stdout(predicate::str::contains(&work_key))
        .stdout(predicate::str::contains(&personal_key));

    let state = State::load(&home).unwrap();
    assert_eq!(state.current_context(), None);
}

#[test]
fn context_explicit_overrides_default() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work_root = tmp.path().join("work");
    let personal_root = tmp.path().join("personal");
    let (work_key, personal_key) = init_two_root_fixture(tmp.path(), &home);
    setup_contexts_config(&home, &work_root, &personal_root, Some("work"), &[]);

    loops(&home)
        .arg("@personal")
        .assert()
        .success()
        .stdout(predicate::str::contains(&personal_key))
        .stdout(predicate::str::contains(&work_key).not());

    let state = State::load(&home).unwrap();
    assert_eq!(state.current_context(), Some("personal"));

    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains(&personal_key))
        .stdout(predicate::str::contains(&work_key).not());
}

#[test]
fn context_with_idle_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work_root = tmp.path().join("work");
    let personal_root = tmp.path().join("personal");
    let repo = work_root.join("work-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);

    git(&repo, &["checkout", "-b", "feat/recent"]);
    std::fs::write(repo.join("recent.txt"), "recent").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "recent wip"]);
    git(&repo, &["checkout", "main"]);

    git(&repo, &["checkout", "-b", "feat/stale"]);
    std::fs::write(repo.join("stale.txt"), "stale").unwrap();
    git(&repo, &["add", "."]);
    git_commit_with_date(&repo, "2020-01-01 00:00:00 +0000", "stale wip");
    git(&repo, &["checkout", "main"]);

    std::fs::create_dir_all(&personal_root).unwrap();
    loops(&home)
        .args([
            "init",
            work_root.to_str().unwrap(),
            personal_root.to_str().unwrap(),
        ])
        .assert()
        .success();

    let recent_filter = format!("root:{} idle:<=30d", toml_path(&work_root));
    setup_contexts_config(
        &home,
        &work_root,
        &personal_root,
        Some("work"),
        &[("recent", &recent_filter)],
    );

    loops(&home)
        .arg("@recent")
        .assert()
        .success()
        .stdout(predicate::str::contains("work/work-app/feat/recent"))
        .stdout(predicate::str::contains("work/work-app/feat/stale").not());
}

#[test]
fn context_unknown_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work_root = tmp.path().join("work");
    let personal_root = tmp.path().join("personal");
    init_two_root_fixture(tmp.path(), &home);
    setup_contexts_config(&home, &work_root, &personal_root, Some("work"), &[]);

    loops(&home)
        .arg("@nope")
        .assert()
        .failure()
        .stderr(predicate::str::contains("[contexts.nope]"));
}

#[test]
fn refresh_honours_context() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let work_root = tmp.path().join("work");
    let personal_root = tmp.path().join("personal");
    init_two_root_fixture(tmp.path(), &home);
    setup_contexts_config(&home, &work_root, &personal_root, Some("work"), &[]);

    loops(&home).assert().success();

    let inv_dir = home.join("inventory");
    assert_eq!(
        count_inventory_json(&inv_dir),
        1,
        "default context should index only the work root"
    );

    loops(&home)
        .arg("refresh")
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));

    loops(&home)
        .args(["refresh", "@personal"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));

    assert_eq!(
        count_inventory_json(&inv_dir),
        2,
        "refresh @personal must index the personal root without touching work scope on default refresh"
    );

    loops(&home)
        .arg("refresh")
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));
}
