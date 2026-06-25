# Setup

## Install

```bash
cargo install open-loops
# or
curl -fsSL https://github.com/carvalhosauro/open-loops/releases/latest/download/open-loops-installer.sh | sh
# or (macOS/Linux, after first tap release)
brew install carvalhosauro/tap/open-loops
```

One-time maintainer setup (secrets, tap repo): [distribution.md](distribution.md).

## Configure

```bash
loops init ~/repo ~/work
```

This creates `~/.open-loops/config.toml`:

```toml
# directories scanned for git repositories (up to 3 levels deep)
roots = ["/home/you/repo", "/home/you/work"]
# command that receives the prompt on stdin and returns the response on stdout
llm_command = "claude -p"
# where Claude Code sessions are stored
sessions_dir = "/home/you/.claude/projects"
# maximum number of sessions used per distillation
max_sessions = 3
# KB read from the end of each session
max_session_kb = 50
```

## Verify

```bash
loops   # should list your unmerged branches
```
