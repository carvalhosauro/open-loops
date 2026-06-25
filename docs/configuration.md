# Configuration

File: `~/.open-loops/config.toml` (created by `loops init`).
Override the base directory: `OPEN_LOOPS_HOME` environment variable.

| Key | Type | Default | Description |
|---|---|---|---|
| `roots` | list of paths | `[]` | Directories searched for git repositories (see `scan_depth`) |
| `scan_depth` | integer | `4` | Max directory depth from each root to search for repositories |
| `llm_command` | string | `"claude -p"` | LLM command: prompt via stdin, response via stdout |
| `sessions_dir` | path | `~/.claude/projects` | Claude Code sessions directory |
| `max_sessions` | integer | `3` | Sessions used per distillation |
| `max_session_kb` | integer | `50` | KB read from the end of each session |
| `aliases` | table | `{}` | Per-root label override, keyed by canonical root path (resolves key collisions) |

## Repository discovery

`loops` discovers repositories by asking git, not by folder naming. Supported
layouts include normal checkouts (`.git` directory), worktrees (`.git` file), and
bare stores. Multiple worktrees of the same repo are scanned once (deduplicated
by git common-dir).

**Edge case:** a bare directory named `.bare` hidden inside a dot-prefixed parent
is skipped during descent and will not be found unless you register a root that
points directly at the repository container or bare path.

## Changing the LLM

Any command that reads from stdin and writes to stdout works:

```toml
llm_command = "ollama run llama3"
```

## Root labels

Keys are `root-label/repo/branch`. The label is the root's directory name, or an
alias when two roots share a name:

```toml
roots = ["/home/you/work", "/home/you/personal"]

[aliases]
"/home/you/work" = "w"
```

If two roots resolve to the same label and neither has an alias, `loops` exits
with an actionable error.

### Upgrading from 0.1.x

Keys gained a `root-label/` prefix (they were `repo/branch`). Existing
`ignores.toml` entries no longer match — re-run `loops ignore <key>` with the
new 3-segment key shown by `loops`. The distillation cache regenerates on its
own (safe to delete `~/.open-loops/cache/`).

## State files

```
~/.open-loops/
├── config.toml    # this configuration
├── ignores.toml   # loops dismissed via `loops ignore`
└── cache/         # distillations per repo/branch@sha (safe to delete)
```
