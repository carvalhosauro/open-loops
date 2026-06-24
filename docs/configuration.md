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

## Changing the LLM

Any command that reads from stdin and writes to stdout works:

```toml
llm_command = "ollama run llama3"
```

## State files

```
~/.open-loops/
├── config.toml    # this configuration
├── ignores.toml   # loops dismissed via `loops ignore`
└── cache/         # distillations per repo/branch@sha (safe to delete)
```
