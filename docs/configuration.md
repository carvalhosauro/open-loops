# Configuration

File: `~/.open-loops/config.toml` (created by `loops init`).
Override the base directory: `OPEN_LOOPS_HOME` environment variable.

| Key | Type | Default | Description |
|---|---|---|---|
| `roots` | list of paths | `[]` | Directories scanned (3 levels deep) for git repos |
| `llm_command` | string | `"claude -p"` | LLM command: prompt via stdin, response via stdout |
| `sessions_dir` | path | `~/.claude/projects` | Claude Code sessions directory |
| `max_sessions` | integer | `3` | Sessions used per distillation |
| `max_session_kb` | integer | `50` | KB read from the end of each session |
| `aliases` | table | `{}` | Per-root label override, keyed by canonical root path (resolves key collisions) |

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

## State files

```
~/.open-loops/
├── config.toml    # this configuration
├── ignores.toml   # loops dismissed via `loops ignore`
└── cache/         # distillations per repo/branch@sha (safe to delete)
```
