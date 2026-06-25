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

### Bare + worktree layout (`.bare/` store)

Typical layout:

```
~/repo/acme/my-app/
├── .bare/          # bare store (no .git inside)
├── .git            # file: gitdir: ./.bare
├── main/           # worktree
└── feat-*/         # more worktrees
```

**You do not register `.bare/` as a root.** Point a root at a parent directory
that contains the **container** (`my-app/`, the folder with the `.git` pointer
file). The scanner detects the container, resolves the common-dir to `.bare/`,
and names the repo after the container (`my-app`).

```toml
roots = ["/home/you/repo/acme"]
scan_depth = 4   # default; increase if repos sit deeper in the tree
```

From `~/repo/acme`, `my-app/` is depth 1 — well within the default. If your
tree is `org/category/project/container`, count levels from each root and set
`scan_depth` accordingly (e.g. depth 4 needs `scan_depth = 4` or higher).

**Alternative:** register the container directly:

```toml
roots = ["/home/you/repo/acme/my-app"]
scan_depth = 1
```

**Alternative:** register the bare store directly (also works):

```toml
roots = ["/home/you/repo/acme/my-app/.bare"]
scan_depth = 1
```

Repo name is still `my-app` (parent of `.bare`), not `.bare`.

**Edge case:** a `.bare/` directory with **no** container `.git` pointer, hidden
under a dot-prefixed parent during descent, is not discovered. Fix: add a root
that points at the container or at `.bare/` itself (see alternatives above).

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
