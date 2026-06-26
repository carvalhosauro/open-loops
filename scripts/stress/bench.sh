#!/usr/bin/env bash
# bench.sh — deterministic stress BENCHMARK for the open-loops CLI.
#
# Usage: bench.sh [--heavy] [--out FILE] [--keep]
#
#   --heavy     use the large scales that originally exposed the bottlenecks
#               (~500 repos, ~2000 branches, ~150 worktrees, ~300MB session).
#               Default scales are modest and finish in ~1-2 min total.
#   --out FILE  also write the results table to FILE.
#   --keep      keep the generated fixtures (default: clean up on exit).
#
# Each PERF scenario is generated, the relevant `loops` command(s) are run under
# the timing harness, and a table is printed:
#   scenario | command | wall_s | max_rss_mb
#
# The run header records loops/git versions, nproc and the repo HEAD so a table
# is self-describing and comparable to the baseline in README.md.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/stress/lib.sh
source "$HERE/lib.sh"

HEAVY=0
OUT=""
KEEP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --heavy) HEAVY=1; shift ;;
    --out) OUT="${2:?--out needs a file}"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "error: unknown arg '$1'" >&2; exit 2 ;;
  esac
done

# Scales: modest defaults vs --heavy (the bottleneck-exposing sizes).
if [[ "$HEAVY" -eq 1 ]]; then
  S_REPOS=500; S_BRANCHES=2000; S_WORKTREES=150; S_SESSION_MB=300; S_WIDE=12
else
  S_REPOS=60; S_BRANCHES=120; S_WORKTREES=30; S_SESSION_MB=20; S_WIDE=6
fi

WORK="$(mktemp -d)"
cleanup() { [[ "$KEEP" -eq 1 ]] || rm -rf "$WORK"; }
trap cleanup EXIT

# Collected result rows: "scenario|command|wall_s|rss_mb".
ROWS=()

# run_case(): generate a fixture, set up an isolated home, time a command.
#   run_case <scenario> <scale> <loops-args...>
# The loops args ARE the command (e.g. `list` is the no-arg invocation, so for
# the inventory we pass no args at all). Scenarios that emit SESSIONS_DIR= on
# stderr get it wired into config with llm_command=cat for resume runs.
run_case() {
  local scenario="$1" scale="$2"; shift 2
  local home root genout generr sessions_dir
  home="$(mktemp -d)"
  genout="$(mktemp)"
  generr="$(mktemp)"
  echo ">> generating $scenario (scale=$scale)..." >&2
  # Capture stdout (root path) and stderr (SESSIONS_DIR=, git noise) to SEPARATE
  # files — no pipe to tail, so a large generator never SIGPIPEs under pipefail,
  # and the two streams never interleave.
  bash "$HERE/gen_fixtures.sh" "$scenario" "$WORK/$scenario" "$scale" >"$genout" 2>"$generr"
  root="$(tail -n1 "$genout")"
  sessions_dir="$(sed -n 's/^SESSIONS_DIR=//p' "$generr" | tail -n1)"
  rm -f "$genout" "$generr"

  OPEN_LOOPS_HOME="$home" "$LOOPS_BIN" init "$root" >/dev/null
  if [[ -n "$sessions_dir" ]]; then
    set_cat_llm "$home"
    set_sessions_dir "$home" "$sessions_dir"
  fi

  local cmd_label="loops $*"
  [[ $# -eq 0 ]] && cmd_label="loops (list)"
  echo ">> timing: $cmd_label ($scenario)" >&2
  OPEN_LOOPS_HOME="$home" timed "$LOOPS_BIN" "$@"
  ROWS+=("$scenario|$cmd_label|$TIMED_WALL_S|$TIMED_RSS_MB")
  rm -rf "$home"
}

print_header() {
  echo "==================== open-loops stress bench ===================="
  printf '  loops:   %s\n' "$("$LOOPS_BIN" --version 2>/dev/null || echo '?')"
  printf '  git:     %s\n' "$(git --version)"
  printf '  nproc:   %s\n' "$(nproc)"
  printf '  HEAD:    %s\n' "$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo '?')"
  printf '  mode:    %s\n' "$([[ "$HEAVY" -eq 1 ]] && echo heavy || echo default)"
  printf '  timer:   %s\n' "$([[ -n "$TIME_BIN" ]] && echo "$TIME_BIN -v" || echo 'bash SECONDS (no RSS)')"
  echo "================================================================="
}

print_table() {
  local fmt='  %-16s %-26s %8s %12s\n'
  printf "$fmt" "SCENARIO" "COMMAND" "WALL_S" "MAX_RSS_MB"
  printf '  %s\n' "----------------------------------------------------------------------"
  local r
  for r in "${ROWS[@]}"; do
    IFS='|' read -r sc cmd wall rss <<<"$r"
    printf "$fmt" "$sc" "$cmd" "$wall" "$rss"
  done
}

main() {
  print_header

  # PERF scenarios + the command that exercises their hot path. The inventory
  # ("list") is the no-arg invocation, so those cases pass no loops args.
  run_case many-repos     "$S_REPOS"                     # scan + dedup + per-repo for-each-ref
  run_case many-branches  "$S_BRANCHES"                  # rev-list --left-right per branch
  run_case many-worktrees "$S_WORKTREES"  worktrees      # worktree list + per-wt status/log
  run_case big-session    "$S_SESSION_MB" resume feat/big # tail read of a big session
  run_case wide-tree      "$S_WIDE"                       # filesystem walk to scan_depth

  echo
  print_table

  if [[ -n "$OUT" ]]; then
    { print_header; echo; print_table; } >"$OUT"
    echo ">> wrote table to $OUT" >&2
  fi
}

main
