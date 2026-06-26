# shellcheck shell=bash
# lib.sh — shared helpers for the open-loops deterministic stress/regression harness.
#
# SOURCE this file; do not execute it. Every consumer runs under strict mode and
# pins a fixed git identity + fixed commit timestamps so that idle/window/sort
# behaviour is reproducible run-to-run.
#
# Determinism contract:
#   - Fixed author/committer identity and fixed GIT_*_DATE (see PIN_DATE).
#   - No $RANDOM, no clock-derived content. Counts come only from the scale arg.
#   - Idle is computed by `loops` against the *current* wall clock, so absolute
#     IDLE numbers grow over time. Assert on STRUCTURE (ordering, columns,
#     presence), never on absolute idle days.
#   - Always isolate state in a fresh mktemp dir; never touch ~/.open-loops or
#     ~/.claude.

set -euo pipefail

# --- paths -------------------------------------------------------------------

# Directory containing this lib (scripts/stress).
STRESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Repo root (two levels up from scripts/stress).
ROOT="$(cd "$STRESS_DIR/../.." && pwd)"

# LOOPS_BIN: path to the `loops` binary under test. Override via env.
LOOPS_BIN="${LOOPS_BIN:-$ROOT/target/release/loops}"
if [[ ! -x "$LOOPS_BIN" ]]; then
  echo "error: loops binary not found or not executable at: $LOOPS_BIN" >&2
  echo "       build it with \`cargo build --release\` or set LOOPS_BIN." >&2
  exit 1
fi

# --- pinned git identity + clock --------------------------------------------

# Constant ISO date used as the baseline commit timestamp. Per-commit ordering
# is achieved by offsetting from this constant by an integer index (see g_commit).
PIN_DATE="${PIN_DATE:-2026-01-01T00:00:00Z}"
# Epoch seconds for PIN_DATE (date -d is GNU coreutils; present on Linux/WSL).
PIN_EPOCH="$(date -u -d "$PIN_DATE" +%s)"

export GIT_AUTHOR_NAME="stress-bot"
export GIT_AUTHOR_EMAIL="stress@open-loops.test"
export GIT_COMMITTER_NAME="stress-bot"
export GIT_COMMITTER_EMAIL="stress@open-loops.test"
# Default both dates to the pin; g_commit overrides per-commit for ordering.
export GIT_AUTHOR_DATE="$PIN_DATE"
export GIT_COMMITTER_DATE="$PIN_DATE"
# Quiet, hint-free, deterministic git regardless of the host's global config.
export GIT_CONFIG_NOSYSTEM=1
export GIT_TERMINAL_PROMPT=0

# g(): pinned git wrapper. `g <dir> <args...>` runs `git -C <dir> <args...>`
# with the fixed identity already exported. Named `g` (not an alias) so it
# survives `set -e` and works the same under bash -c.
g() {
  git -C "$1" "${@:2}"
}

# g_commit(): create a commit in <dir> with a timestamp offset by <index>
# seconds from PIN_DATE, so that commit order is stable AND distinct.
# Usage: g_commit <dir> <message> <index>
g_commit() {
  local dir="$1" msg="$2" idx="${3:-0}"
  local ts
  ts="$(date -u -d "@$((PIN_EPOCH + idx))" +%Y-%m-%dT%H:%M:%S%z)"
  GIT_AUTHOR_DATE="$ts" GIT_COMMITTER_DATE="$ts" \
    git -C "$dir" commit -q -m "$msg"
}

# --- session-path encoding ---------------------------------------------------

# encode_project_path(): reproduce Claude Code's project-dir encoding — replace
# every '/' AND '.' in the absolute path with '-'. Must match the Rust
# implementation in src/sessions/claude_code.rs.
#   /home/g/my.app -> -home-g-my-app
encode_project_path() {
  printf '%s' "$1" | sed 's/[/.]/-/g'
}

# --- repo / branch / worktree builders --------------------------------------

# mk_repo(): a normal repo with a `main` branch and one commit.
#   mk_repo <dir> [index]
mk_repo() {
  local dir="$1" idx="${2:-0}"
  mkdir -p "$dir"
  git -C "$dir" init -q -b main
  printf 'init\n' >"$dir/README.md"
  g "$dir" add -A
  g_commit "$dir" "init" "$idx"
}

# mk_branch(): create a branch off main with one exclusive commit, then return
# to main. Deterministic file name derived from the branch.
#   mk_branch <repo> <branch> [index]
mk_branch() {
  local repo="$1" branch="$2" idx="${3:-0}"
  local safe
  safe="$(printf '%s' "$branch" | tr '/ ' '__')"
  g "$repo" checkout -q -b "$branch"
  printf 'work on %s\n' "$branch" >"$repo/$safe.txt"
  g "$repo" add -A
  g_commit "$repo" "wip $branch" "$idx"
  g "$repo" checkout -q main
}

# mk_worktree(): add a worktree on a NEW branch off the current HEAD (counts as
# merged into main => verdict deletable unless given its own commit).
#   mk_worktree <repo> <wt_path> <branch>
mk_worktree() {
  local repo="$1" wt="$2" branch="$3"
  g "$repo" worktree add -q -b "$branch" "$wt" >/dev/null 2>&1
}

# mk_bare_worktree_container(): the canonical Fase B layout —
#   <container>/.bare        bare object store
#   <container>/.git         "gitdir: ./.bare" pointer file
#   <container>/main         main worktree with an init commit
#   mk_bare_worktree_container <container> [index]
mk_bare_worktree_container() {
  local container="$1" idx="${2:-0}"
  mkdir -p "$container/.bare"
  git -C "$container/.bare" init -q --bare -b main
  printf 'gitdir: ./.bare\n' >"$container/.git"
  git -C "$container" worktree add -q -b main "$container/main" >/dev/null 2>&1
  printf 'init\n' >"$container/main/README.md"
  g "$container/main" add -A
  g_commit "$container/main" "init" "$idx"
}

# write_session(): write a Claude Code session file at the ENCODED path for
# <project_abs_path> under <sessions_dir>. The body is a deterministic user
# message containing <text>. Optionally pads the file to <kb> kilobytes with
# non-extractable `summary` lines so the meaningful message stays in the tail.
#   write_session <sessions_dir> <project_abs_path> <file_name> <text> [kb]
write_session() {
  local sdir="$1" project="$2" name="$3" text="$4" kb="${5:-0}"
  local enc dir
  enc="$(encode_project_path "$project")"
  dir="$sdir/$enc"
  mkdir -p "$dir"
  local f="$dir/$name"
  : >"$f"
  if [[ "$kb" -gt 0 ]]; then
    # Pad with a SINGLE giant `summary` line (ignored by extract_text) sized to
    # ~<kb> KB, then a newline, then the real message LAST. This:
    #   - keeps the meaningful message in the tail window read_tail_text reads,
    #   - and (when the file exceeds max_session_kb) exercises the truncation
    #     path where the cut first line is skipped — exactly the Rust contract.
    # Generated without a `yes | head` pipe, which SIGPIPEs under `pipefail`.
    local target
    target=$(( kb * 1024 ))
    {
      printf '{"type":"summary","x":"'
      # head -c closing the pipe makes tr SIGPIPE; isolate so it cannot abort
      # the caller under `set -o pipefail`.
      ( set +o pipefail; tr '\0' 'A' </dev/zero | head -c "$target" ) || true
      printf '"}\n'
    } >>"$f"
  fi
  printf '%s\n' "{\"type\":\"user\",\"message\":{\"content\":\"$text\"}}" >>"$f"
}

# --- config setters ----------------------------------------------------------
# These edit TOP-LEVEL scalars with sed -i. NEVER append: appending after the
# [aliases] table would capture the key inside that table and break the config.

# set_cat_llm(): point llm_command at `cat` so the resume pipeline echoes the
# prompt instead of calling a real LLM.
set_cat_llm() {
  local home="$1"
  sed -i 's/^llm_command = .*/llm_command = "cat"/' "$home/config.toml"
}

# set_llm(): set an arbitrary llm_command (e.g. "false" to prove cache hits).
set_llm() {
  local home="$1" cmd="$2"
  sed -i "s/^llm_command = .*/llm_command = \"$cmd\"/" "$home/config.toml"
}

# set_sessions_dir(): point sessions_dir at <path>. Uses | as the sed delimiter
# so absolute paths with slashes are safe.
set_sessions_dir() {
  local home="$1" path="$2"
  sed -i "s|^sessions_dir = .*|sessions_dir = \"$path\"|" "$home/config.toml"
}

# --- timing ------------------------------------------------------------------

# Detect /usr/bin/time once. If present we capture wall + max RSS; otherwise we
# fall back to bash SECONDS and report RSS as n/a.
TIME_BIN=""
if [[ -x /usr/bin/time ]]; then
  TIME_BIN="/usr/bin/time"
fi

# timed(): run a command, capture wall seconds and max RSS (MB).
# Sets globals TIMED_WALL_S and TIMED_RSS_MB. stdout/stderr of the command are
# discarded (benchmarks care about cost, not output).
#   timed <cmd> [args...]
timed() {
  TIMED_WALL_S=""
  TIMED_RSS_MB=""
  local tf
  tf="$(mktemp)"
  if [[ -n "$TIME_BIN" ]]; then
    # -v gives "Elapsed (wall clock) time" and "Maximum resident set size (kbytes)".
    "$TIME_BIN" -v "$@" >/dev/null 2>"$tf" || true
    local wall_raw rss_kb
    # Elapsed line is h:mm:ss or m:ss.ss — normalise to seconds.
    wall_raw="$(grep -E 'Elapsed \(wall clock\)' "$tf" | sed 's/.*: //')"
    rss_kb="$(grep -E 'Maximum resident set size' "$tf" | sed 's/.*: //')"
    TIMED_WALL_S="$(elapsed_to_seconds "$wall_raw")"
    if [[ -n "$rss_kb" ]]; then
      TIMED_RSS_MB="$(awk -v k="$rss_kb" 'BEGIN{printf "%.0f", k/1024}')"
    else
      TIMED_RSS_MB="n/a"
    fi
  else
    local start end
    start="$SECONDS"
    "$@" >/dev/null 2>&1 || true
    end="$SECONDS"
    TIMED_WALL_S="$(awk -v s="$((end - start))" 'BEGIN{printf "%.2f", s}')"
    TIMED_RSS_MB="n/a"
  fi
  rm -f "$tf"
}

# elapsed_to_seconds(): convert /usr/bin/time's [h:]m:s[.frac] into seconds.
elapsed_to_seconds() {
  local raw="$1"
  awk -v t="$raw" 'BEGIN{
    n = split(t, p, ":")
    s = 0
    for (i = 1; i <= n; i++) s = s * 60 + p[i]
    printf "%.2f", s
  }'
}

# --- assertions + counters ---------------------------------------------------

PASS_COUNT=0
FAIL_COUNT=0
# Lines for the final summary table: "PASS|<name>" or "FAIL|<name>".
SUMMARY_ROWS=()

pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  SUMMARY_ROWS+=("PASS|$1")
  echo "  PASS: $1"
}

fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  SUMMARY_ROWS+=("FAIL|$1")
  echo "  FAIL: $1" >&2
}

# assert_contains <haystack> <needle> <name>
assert_contains() {
  if [[ "$1" == *"$2"* ]]; then
    pass "$3"
  else
    fail "$3 (expected to contain: $2)"
  fi
}

# assert_not_contains <haystack> <needle> <name>
assert_not_contains() {
  if [[ "$1" != *"$2"* ]]; then
    pass "$3"
  else
    fail "$3 (expected NOT to contain: $2)"
  fi
}

# assert_exit <expected_code> <actual_code> <name>
assert_exit() {
  if [[ "$1" == "$2" ]]; then
    pass "$3"
  else
    fail "$3 (expected exit $1, got $2)"
  fi
}

# print_summary(): render the PASS/FAIL table and totals. Returns 1 if any FAIL.
print_summary() {
  echo
  echo "================ SUMMARY ================"
  local row status name
  for row in "${SUMMARY_ROWS[@]}"; do
    status="${row%%|*}"
    name="${row#*|}"
    printf '  [%s] %s\n' "$status" "$name"
  done
  echo "----------------------------------------"
  printf '  PASS=%d  FAIL=%d  TOTAL=%d\n' "$PASS_COUNT" "$FAIL_COUNT" "$((PASS_COUNT + FAIL_COUNT))"
  echo "========================================"
  [[ "$FAIL_COUNT" -eq 0 ]]
}

# fresh_home(): create an isolated OPEN_LOOPS_HOME and echo its path.
fresh_home() {
  mktemp -d
}
