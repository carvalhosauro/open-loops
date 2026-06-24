#!/usr/bin/env bash
# Self-contained quickstart demo for asciinema recording or manual runs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOOPS="${LOOPS_BIN:-$ROOT/target/release/loops}"
DEMO="$(mktemp -d)"
HOME="$DEMO/home"
REPO="$DEMO/projects/my-app"

export OPEN_LOOPS_HOME="$HOME"
export GIT_AUTHOR_NAME=demo
export GIT_AUTHOR_EMAIL=demo@example.com
export GIT_COMMITTER_NAME=demo
export GIT_COMMITTER_EMAIL=demo@example.com

mkdir -p "$REPO"
git -C "$REPO" init -b main >/dev/null
printf 'base\n' >"$REPO/README.md"
git -C "$REPO" add .
git -C "$REPO" commit -m "init" >/dev/null

git -C "$REPO" checkout -b feat/login >/dev/null
printf 'login wip\n' >"$REPO/auth.rs"
git -C "$REPO" add .
git -C "$REPO" commit -m "feat: login wip" >/dev/null

echo '$ loops init ~/projects'
"$LOOPS" init "$DEMO/projects"
echo
echo '$ loops'
"$LOOPS"
echo
echo '$ loops resume feat/login --dry-run'
"$LOOPS" resume feat/login --dry-run
echo
echo '$ loops resume feat/login   # LLM replaced by cat in demo config'
CFG="$HOME/config.toml"
sed -i 's/llm_command = "claude -p"/llm_command = "cat"/' "$CFG"
"$LOOPS" resume feat/login

rm -rf "$DEMO"
