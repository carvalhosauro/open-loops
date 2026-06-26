#!/usr/bin/env bash
# regress.sh — black-box BEHAVIOR regression for the open-loops CLI.
#
# Usage: regress.sh [--scale N]
#
# Builds deterministic fixtures and asserts the observable contract of every
# subcommand. Exits NONZERO if any assertion fails (CI-able). Prints a PASS/FAIL
# summary table + count at the end.
#
# Coverage:
#   - list excludes default + merged branches
#   - each query filter (bare term, repo:, branch:, key:, root:, idle:, ahead:,
#     behind:, +ignored)
#   - resume --dry-run shows commits + diffstat
#   - resume (cat) shows `## Sources` + `**Confidence:**`; 2nd identical call is a
#     cache hit (proven by flipping llm_command to `false`)
#   - ignore removes a loop from the list
#   - worktrees / wt verdicts (home/cold/deletable/active/prunable)
#   - completions for bash/zsh/fish
#   - Fase B: session under encoded WORKTREE path IS surfaced by resume <branch>;
#     the SAME session under the CONTAINER path is NOT surfaced (discriminator)
#   - graceful degradation: broken/no-commit repo -> stderr warning, exit 0,
#     other repos still listed

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/stress/lib.sh
source "$HERE/lib.sh"

SCALE=8
while [[ $# -gt 0 ]]; do
  case "$1" in
    --scale) SCALE="${2:?--scale needs N}"; shift 2 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "error: unknown arg '$1'" >&2; exit 2 ;;
  esac
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# loops_in <home> <args...>: run loops with an isolated home, stdout only.
loops_in() {
  local home="$1"; shift
  OPEN_LOOPS_HOME="$home" "$LOOPS_BIN" "$@" 2>/dev/null
}

# gen(): build a fixture, exposing GEN_ROOT and GEN_SDIR (may be empty).
# stdout (root) and stderr (SESSIONS_DIR=, git noise) go to separate files —
# no pipe to tail, so large fixtures never SIGPIPE under `set -o pipefail`.
gen() {
  local scenario="$1" scale="${2:-$SCALE}" out err
  out="$(mktemp)"; err="$(mktemp)"
  bash "$HERE/gen_fixtures.sh" "$scenario" "$WORK/$scenario" "$scale" >"$out" 2>"$err"
  GEN_ROOT="$(tail -n1 "$out")"
  GEN_SDIR="$(sed -n 's/^SESSIONS_DIR=//p' "$err" | tail -n1)"
  rm -f "$out" "$err"
}

# ===========================================================================
echo "### GROUP: list — excludes default + merged ###"
# ===========================================================================
section_list() {
  local home root out
  home="$(mktemp -d)"
  root="$WORK/list-basic"
  local repo="$root/app"
  mk_repo "$repo" 0
  mk_branch "$repo" "feat/keep" 1          # unmerged -> should appear
  g "$repo" branch merged-branch           # points at main -> merged -> hidden
  loops_in "$home" init "$root" >/dev/null
  out="$(loops_in "$home")"
  assert_contains "$out" "app/feat/keep" "list shows unmerged branch"
  assert_not_contains "$out" "merged-branch" "list hides merged branch"
  assert_not_contains "$out" "app/main" "list hides default branch"
  assert_contains "$out" "LOOP" "list prints LOOP/IDLE/AHEAD/BEHIND header"
  assert_contains "$out" "AHEAD" "list header has AHEAD column"
  rm -rf "$home"
}
section_list

# ===========================================================================
echo "### GROUP: query filters ###"
# ===========================================================================
section_filters() {
  local home root out
  home="$(mktemp -d)"
  root="$WORK/filters"
  mkdir -p "$root"
  local api="$root/billing-api"
  local web="$root/web-ui"
  mk_repo "$api" 0
  mk_repo "$web" 10
  mk_branch "$api" "feat/login" 1          # idx1 -> oldest exclusive commit
  mk_branch "$web" "fix/layout" 20         # idx20 -> newer
  loops_in "$home" init "$root" >/dev/null

  out="$(loops_in "$home" billing)"
  assert_contains "$out" "billing-api/feat/login" "bare term matches repo substring"
  assert_not_contains "$out" "web-ui/fix/layout" "bare term excludes non-match"

  out="$(loops_in "$home" repo:web)"
  assert_contains "$out" "web-ui/fix/layout" "repo: filter matches"
  assert_not_contains "$out" "billing-api" "repo: filter excludes others"

  out="$(loops_in "$home" branch:login)"
  assert_contains "$out" "feat/login" "branch: filter matches"
  assert_not_contains "$out" "fix/layout" "branch: filter excludes others"

  out="$(loops_in "$home" key:web-ui/fix)"
  assert_contains "$out" "web-ui/fix/layout" "key: filter matches full key substring"

  out="$(loops_in "$home" root:filters)"
  assert_contains "$out" "feat/login" "root: filter keeps matching root"

  # idle: every commit is pinned in the past, so idle:>1d is always true; a
  # 9999-week threshold is always false. Structural, not absolute.
  out="$(loops_in "$home" "idle:>1d")"
  assert_contains "$out" "feat/login" "idle:>1d surfaces aged loops"
  out="$(loops_in "$home" "idle:>9999w")"
  assert_not_contains "$out" "feat/login" "idle:>9999w excludes everything"

  # ahead/behind: each feature branch is exactly 1 ahead, 0 behind of main.
  out="$(loops_in "$home" "ahead:>=1")"
  assert_contains "$out" "feat/login" "ahead:>=1 surfaces ahead branches"
  out="$(loops_in "$home" "ahead:>5")"
  assert_not_contains "$out" "feat/login" "ahead:>5 excludes 1-ahead branches"
  out="$(loops_in "$home" "behind:0")"
  assert_contains "$out" "feat/login" "behind:0 matches non-behind branches"
  out="$(loops_in "$home" "behind:>0")"
  assert_not_contains "$out" "feat/login" "behind:>0 excludes non-behind branches"
  rm -rf "$home"
}
section_filters

# ===========================================================================
echo "### GROUP: ignore + +ignored tag ###"
# ===========================================================================
section_ignore() {
  local home root out key
  home="$(mktemp -d)"
  root="$WORK/ignore"
  local repo="$root/app"
  mk_repo "$repo" 0
  mk_branch "$repo" "feat/drop" 1
  loops_in "$home" init "$root" >/dev/null
  key="$(loops_in "$home" | awk '/feat\/drop/{print $1; exit}')"
  loops_in "$home" ignore "$key" >/dev/null
  out="$(loops_in "$home")"
  assert_not_contains "$out" "feat/drop" "ignore removes loop from default list"
  out="$(loops_in "$home" +ignored)"
  assert_contains "$out" "feat/drop" "+ignored brings the ignored loop back"
  rm -rf "$home"
}
section_ignore

# ===========================================================================
echo "### GROUP: resume --dry-run ###"
# ===========================================================================
section_dry_run() {
  local home root out
  home="$(mktemp -d)"
  root="$WORK/dryrun"
  local repo="$root/app"
  mk_repo "$repo" 0
  mk_branch "$repo" "feat/dry" 1
  loops_in "$home" init "$root" >/dev/null
  out="$(loops_in "$home" resume feat/dry --dry-run)"
  assert_contains "$out" "Commits (base..branch)" "dry-run shows commits section"
  assert_contains "$out" "wip feat/dry" "dry-run shows the branch commit"
  assert_contains "$out" "Diffstat" "dry-run shows diffstat section"
  assert_contains "$out" "feat_dry.txt" "dry-run diffstat names changed file"
  assert_contains "$out" "Dry run — LLM not invoked" "dry-run declares no LLM call"
  rm -rf "$home"
}
section_dry_run

# ===========================================================================
echo "### GROUP: resume (cat) + cache hit ###"
# ===========================================================================
section_resume_cache() {
  local home root out1 out2 ec
  home="$(mktemp -d)"
  root="$WORK/resume"
  local repo="$root/app"
  mk_repo "$repo" 0
  mk_branch "$repo" "feat/resume" 1
  loops_in "$home" init "$root" >/dev/null
  set_cat_llm "$home"
  out1="$(loops_in "$home" resume feat/resume)"
  assert_contains "$out1" "## Sources" "resume output has ## Sources section"
  assert_contains "$out1" "**Confidence:**" "resume output has Confidence line"

  # Flip llm_command to `false`: a 2nd identical call must succeed purely from
  # cache (no LLM invoked), proving the cache hit.
  set_llm "$home" "false"
  set +e
  out2="$(OPEN_LOOPS_HOME="$home" "$LOOPS_BIN" resume feat/resume 2>/dev/null)"
  ec=$?
  set -e
  assert_exit 0 "$ec" "2nd resume exits 0 with llm_command=false (cache hit)"
  assert_contains "$out2" "## Sources" "cached resume still has ## Sources"
  rm -rf "$home"
}
section_resume_cache

# ===========================================================================
echo "### GROUP: worktrees / wt verdicts ###"
# ===========================================================================
section_worktrees() {
  local home root out outwt
  home="$(mktemp -d)"
  gen pathological "$SCALE"
  root="$GEN_ROOT"
  loops_in "$home" init "$root" >/dev/null
  out="$(loops_in "$home" worktrees)"
  assert_contains "$out" "WORKTREE" "worktrees prints table header"
  assert_contains "$out" "VERDICT" "worktrees header has VERDICT column"
  assert_contains "$out" "home" "worktrees classifies a home (main) worktree"
  assert_contains "$out" "active" "worktrees classifies a detached/active worktree"
  assert_contains "$out" "prunable" "worktrees classifies a prunable worktree"
  # wt alias must produce the same header.
  outwt="$(loops_in "$home" wt)"
  assert_contains "$outwt" "WORKTREE" "wt alias works like worktrees"
  rm -rf "$home"

  # cold + deletable: build a dedicated repo so verdicts are unambiguous.
  home="$(mktemp -d)"
  root="$WORK/wt-verdicts"
  local repo="$root/app"
  mk_repo "$repo" 0
  mk_worktree "$repo" "$root/wt-done" "feat/done"     # merged + clean -> deletable
  mk_worktree "$repo" "$root/wt-cold" "feat/cold"     # gets own commit -> cold
  printf 'c\n' >"$root/wt-cold/c.txt"; g "$root/wt-cold" add -A; g_commit "$root/wt-cold" "wip cold" 5
  loops_in "$home" init "$root" >/dev/null
  out="$(loops_in "$home" worktrees)"
  assert_contains "$out" "deletable" "worktrees classifies a deletable worktree"
  assert_contains "$out" "cold" "worktrees classifies a cold worktree"
  rm -rf "$home"
}
section_worktrees

# ===========================================================================
echo "### GROUP: completions (bash/zsh/fish) ###"
# ===========================================================================
section_completions() {
  local home sh ec out
  home="$(mktemp -d)"
  for sh in bash zsh fish; do
    set +e
    out="$(OPEN_LOOPS_HOME="$home" "$LOOPS_BIN" completions "$sh" 2>/dev/null)"
    ec=$?
    set -e
    assert_exit 0 "$ec" "completions $sh exits 0"
    assert_contains "$out" "loops" "completions $sh script mentions loops"
  done
  rm -rf "$home"
}
section_completions

# ===========================================================================
echo "### GROUP: Fase B — worktree session surfaced, container session NOT ###"
# ===========================================================================
section_fase_b() {
  local home out_wt out_ct
  home="$(mktemp -d)"
  gen bare-worktree "$SCALE"
  loops_in "$home" init "$GEN_ROOT" >/dev/null
  set_cat_llm "$home"
  set_sessions_dir "$home" "$GEN_SDIR"

  # The session under the encoded WORKTREE path must be surfaced.
  out_wt="$(loops_in "$home" resume feat/probe)"
  assert_contains "$out_wt" "SENTINEL_WORKTREE" "Fase B: worktree-path session IS surfaced"
  # The session under the CONTAINER path must NOT be surfaced (the discriminator
  # that catches a revert of the repo_path=worktree fix).
  assert_not_contains "$out_wt" "SENTINEL_CONTAINER" "Fase B: container-path session is NOT surfaced"

  # Cross-check via dry-run: exactly one matched session line, no container leak.
  out_ct="$(loops_in "$home" resume feat/probe --dry-run)"
  assert_contains "$out_ct" "wt.jsonl" "Fase B dry-run lists the worktree session file"
  assert_not_contains "$out_ct" "container.jsonl" "Fase B dry-run omits the container session file"
  rm -rf "$home"
}
section_fase_b

# ===========================================================================
echo "### GROUP: graceful degradation (broken repo) ###"
# ===========================================================================
section_graceful() {
  local home root stderr stdout ec
  home="$(mktemp -d)"
  root="$WORK/graceful"
  mkdir -p "$root"
  local good="$root/good"
  mk_repo "$good" 0
  mk_branch "$good" "feat/survive" 1
  # broken: init with NO commits -> default_branch() fails -> warning, not abort.
  local broken="$root/broken"
  mkdir -p "$broken"
  git -C "$broken" init -q -b main
  loops_in "$home" init "$root" >/dev/null

  set +e
  stderr="$(OPEN_LOOPS_HOME="$home" "$LOOPS_BIN" 2>&1 >/dev/null)"
  stdout="$(OPEN_LOOPS_HOME="$home" "$LOOPS_BIN" 2>/dev/null)"
  ec=$?
  set -e
  assert_exit 0 "$ec" "broken repo does not abort the scan (exit 0)"
  assert_contains "$stderr" "warning" "broken repo emits a warning on stderr"
  assert_contains "$stdout" "good/feat/survive" "healthy repo still listed despite broken neighbour"
  rm -rf "$home"
}
section_graceful

# ===========================================================================
print_summary
