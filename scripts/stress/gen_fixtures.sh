#!/usr/bin/env bash
# gen_fixtures.sh — deterministic fixture builders for the open-loops harness.
#
# Usage: gen_fixtures.sh <scenario> <outdir> [scale]
#
# Builds a self-contained, reproducible fixture tree under <outdir> and prints
# the generated ROOT path on the last line of stdout (the directory you would
# pass to `loops init`). All structure is derived only from <scale>; there is
# no randomness and no clock-derived content (see lib.sh determinism contract).
#
# Scenarios:
#   many-repos       <scale> independent repos, each with one unmerged branch.
#   many-branches    one repo with <scale> unmerged branches.
#   many-worktrees   one repo with <scale> worktrees (mixed verdicts).
#   big-session      bare+worktree container + an <scale>-MB session file at the
#                    ENCODED worktree path.
#   wide-tree        many non-repo dirs + a few repos near the scan_depth edge.
#   bare-worktree    canonical Fase B layout (.bare/.git/main + feature wt) plus
#                    a session under the encoded worktree path.
#   pathological     broken/no-commit/bare repos, detached + prunable worktrees,
#                    a slash branch and a unicode branch.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/stress/lib.sh
source "$HERE/lib.sh"

scenario="${1:?usage: gen_fixtures.sh <scenario> <outdir> [scale]}"
outdir="${2:?usage: gen_fixtures.sh <scenario> <outdir> [scale]}"
scale="${3:-10}"

mkdir -p "$outdir"
outdir="$(cd "$outdir" && pwd)" # absolute (loops init canonicalizes anyway)

# zero-padded index so lexical sort == numeric sort (stable repo ordering).
pad() { printf '%03d' "$1"; }

gen_many_repos() {
  local root="$outdir/many-repos"
  mkdir -p "$root"
  local i
  for ((i = 1; i <= scale; i++)); do
    local repo="$root/repo-$(pad "$i")"
    mk_repo "$repo" "$i"
    mk_branch "$repo" "feat/work-$(pad "$i")" "$((i + 1))"
  done
  echo "$root"
}

gen_many_branches() {
  local root="$outdir/many-branches"
  mkdir -p "$root"
  local repo="$root/big-repo"
  mk_repo "$repo" 0
  local i
  for ((i = 1; i <= scale; i++)); do
    mk_branch "$repo" "feat/branch-$(pad "$i")" "$i"
  done
  echo "$root"
}

gen_many_worktrees() {
  local root="$outdir/many-worktrees"
  mkdir -p "$root"
  local repo="$root/wt-repo"
  mk_repo "$repo" 0
  local i
  for ((i = 1; i <= scale; i++)); do
    local wt="$root/wt-$(pad "$i")"
    mk_worktree "$repo" "$wt" "feat/wt-$(pad "$i")"
    # give every third worktree an exclusive commit (cold instead of deletable)
    if (( i % 3 == 0 )); then
      printf 'c\n' >"$wt/c.txt"
      g "$wt" add -A
      g_commit "$wt" "wip wt $i" "$i"
    fi
  done
  echo "$root"
}

gen_big_session() {
  local root="$outdir/big-session"
  mkdir -p "$root"
  local container="$root/my-app"
  mk_bare_worktree_container "$container" 0
  # feature worktree checked out at <container>/feat-big
  local wt="$container/feat-big"
  g "$container" worktree add -q -b feat/big "$wt" >/dev/null 2>&1
  printf 'big work\n' >"$wt/big.txt"
  g "$wt" add -A
  g_commit "$wt" "wip feat/big" 1
  # an <scale>-MB session at the encoded WORKTREE path.
  local sdir="$root/.sessions"
  write_session "$sdir" "$wt" "huge.jsonl" "SENTINEL_BIG resume feat/big now" "$((scale * 1024))"
  # echo the sessions dir so the bench can wire it into config; ROOT stays last.
  echo "SESSIONS_DIR=$sdir" >&2
  echo "$root"
}

gen_wide_tree() {
  local root="$outdir/wide-tree"
  mkdir -p "$root"
  # A wide, shallow forest of non-repo directories (exercises the walker's
  # read_dir/recursion before it hits anything interesting). scan_depth is 4,
  # so put a repo at depth 4 (found) and one at depth 5 (pruned).
  local breadth=$(( scale ))
  local i j
  for ((i = 1; i <= breadth; i++)); do
    for ((j = 1; j <= breadth; j++)); do
      mkdir -p "$root/dir-$(pad "$i")/sub-$(pad "$j")/leaf"
    done
  done
  # repo exactly at depth 4 from root: a/b/c/repo-deep  (root=0,a=1,b=2,c=3,repo=4)
  local deep="$root/a/b/c/repo-deep"
  mk_repo "$deep" 0
  mk_branch "$deep" "feat/deep" 1
  # repo at depth 5: should be pruned by scan_depth=4
  local toodeep="$root/a/b/c/d/repo-toodeep"
  mk_repo "$toodeep" 0
  mk_branch "$toodeep" "feat/toodeep" 1
  # a shallow repo at depth 1 (always found)
  local shallow="$root/repo-shallow"
  mk_repo "$shallow" 0
  mk_branch "$shallow" "feat/shallow" 1
  echo "$root"
}

gen_bare_worktree() {
  local root="$outdir/bare-worktree"
  mkdir -p "$root"
  local container="$root/my-app"
  mk_bare_worktree_container "$container" 0
  # feature worktree at <container>/probe on branch feat/probe
  local wt="$container/probe"
  g "$container" worktree add -q -b feat/probe "$wt" >/dev/null 2>&1
  printf 'work\n' >"$wt/f.txt"
  g "$wt" add -A
  g_commit "$wt" "wip feat/probe" 1
  # Discriminating sessions for the Fase B regression:
  #   - one under the encoded WORKTREE path  -> MUST be surfaced
  #   - one under the encoded CONTAINER path -> MUST NOT be surfaced
  local sdir="$root/.sessions"
  write_session "$sdir" "$wt" "wt.jsonl" "SENTINEL_WORKTREE working on feat/probe" 0
  write_session "$sdir" "$container" "container.jsonl" "SENTINEL_CONTAINER also names feat/probe" 0
  echo "SESSIONS_DIR=$sdir" >&2
  echo "$root"
}

gen_pathological() {
  local root="$outdir/pathological"
  mkdir -p "$root"

  # 1) healthy repo so the scan has something valid to list alongside the junk.
  local ok="$root/ok-repo"
  mk_repo "$ok" 0
  mk_branch "$ok" "feat/ok" 1
  # slash branch (3 segments) and a unicode branch on the same repo.
  mk_branch "$ok" "feat/a/b/c" 2
  mk_branch "$ok" "feat/ção-café" 3

  # 2) no-commit repo: `git init` only -> default_branch() fails -> warning.
  local empty="$root/no-commit-repo"
  mkdir -p "$empty"
  git -C "$empty" init -q -b main

  # 3) pure bare repo with a seeded main + a feature branch via throwaway clone.
  local bare="$root/foo.git"
  git -C "$(dirname "$bare")" init -q --bare -b main "foo.git" 2>/dev/null || {
    mkdir -p "$bare"; git -C "$bare" init -q --bare -b main
  }
  local seed="$root/.seed"
  git clone -q "$bare" "$seed"
  printf 'init\n' >"$seed/README.md"
  g "$seed" add -A
  g_commit "$seed" "init" 0
  g "$seed" push -q origin main
  g "$seed" checkout -q -b feat/bare
  printf 'b\n' >"$seed/b.txt"
  g "$seed" add -A
  g_commit "$seed" "wip feat/bare" 1
  g "$seed" push -q origin feat/bare
  rm -rf "$seed"

  # 4) detached-HEAD worktree (clean) -> verdict active.
  local host="$root/host-repo"
  mk_repo "$host" 0
  mk_branch "$host" "feat/host" 1
  local det="$root/wt-detached"
  local head_sha
  head_sha="$(g "$host" rev-parse HEAD)"
  g "$host" worktree add -q --detach "$det" "$head_sha" >/dev/null 2>&1

  # 5) prunable worktree: create then delete its directory.
  local gone="$root/wt-gone"
  g "$host" worktree add -q -b feat/gone "$gone" >/dev/null 2>&1
  rm -rf "$gone"

  echo "$root"
}

case "$scenario" in
  many-repos)     gen_many_repos ;;
  many-branches)  gen_many_branches ;;
  many-worktrees) gen_many_worktrees ;;
  big-session)    gen_big_session ;;
  wide-tree)      gen_wide_tree ;;
  bare-worktree)  gen_bare_worktree ;;
  pathological)   gen_pathological ;;
  *)
    echo "error: unknown scenario '$scenario'" >&2
    echo "valid: many-repos many-branches many-worktrees big-session wide-tree bare-worktree pathological" >&2
    exit 2
    ;;
esac
