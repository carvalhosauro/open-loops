//! E2E: real binary, real git repos, LLM replaced by `cat`.
use assert_cmd::Command;
use open_loops::sessions::claude_code::encode_project_path;
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
    let count_json = || {
        std::fs::read_dir(&inv_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count()
    };
    assert_eq!(count_json(), 2, "expected one inventory file per repo");

    loops(&home)
        .args(["refresh", "repo:api"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 1 repo"));

    assert_eq!(
        count_json(),
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
    let count_json = || {
        std::fs::read_dir(&inv_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count()
    };
    assert_eq!(count_json(), 3, "one inventory file per repo");

    // web-1 disappears from disk; it is now an orphan.
    std::fs::remove_dir_all(projects.join("web-1")).unwrap();

    // Scoped refresh that never scans web-1 still reclaims its orphan memo.
    loops(&home)
        .args(["refresh", "repo:api"])
        .assert()
        .success()
        .stderr(predicate::str::contains("refreshed 2 repos"))
        .stderr(predicate::str::contains("removed orphan inventory"));

    assert_eq!(count_json(), 2, "disk-gone web-1 inventory must be pruned");
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

    // Poison the cached ahead count, preserving the SHA keys so the memo still
    // validates and gets served.
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

    // Warm scan serves the poisoned memo → ahead:99 matches.
    loops(&home)
        .arg("ahead:99")
        .assert()
        .success()
        .stdout(predicate::str::contains("feat/x"));

    // --fresh ignores the memo and recomputes ahead=1 → ahead:99 matches nothing.
    loops(&home)
        .args(["--fresh", "ahead:99"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No loops match"));
}
