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
| `inventory_ttl_secs` | integer | `0` | Seconds before a cached ahead/behind entry expires; `0` = SHA-only validation, no time-based expiry |
| `default_context` | string | — | Named context applied when the query has no `@` token |
| `[contexts.X]` | table | — | Saved scope; `filter` is a query string |

## Contexts

Define named scopes so you do not repeat `root:` on every invocation:

```toml
default_context = "work"

[contexts.work]
filter = "root:~/work"

[contexts.personal]
filter = "root:~/personal"

[contexts.recent-work]
filter = "root:~/work idle:<=30d"
```

Each `filter` is a full query fragment (`root:`, `repo:`, `idle:>7d`, bare terms,
`+ignored`, etc.). Context filters cannot contain `@` or `:` (reports are phase 5).

When the query has no `@` token, `default_context` applies automatically. Override
per shell or session with `LOOPS_CONTEXT` — it wins over `default_context` in
config. An empty `LOOPS_CONTEXT` is treated as unset.

```bash
LOOPS_CONTEXT=personal loops    # same as default_context = "personal"
loops @none                     # ignore default for one run
```

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
├── config.toml        # this configuration
├── ignores.toml       # loops dismissed via `loops ignore`
├── cache/             # distillations per repo/branch@sha (safe to delete)
└── inventory/         # ahead/behind memo per repo (safe to delete)
```

### Inventory cache

`loops` memoises the expensive `rev-list` (ahead/behind) computation for each
unmerged branch in `inventory/<fnv64hex>.json`, keyed by `(branch, head_sha,
default_sha)`. The light git phase (branch list, last-commit date) always runs
fresh; only ahead/behind is cached.

Use `--inventory_ttl_secs` to add a time-based expiry on top of SHA
validation (default 0 = SHA-only):

```toml
inventory_ttl_secs = 3600   # re-run rev-list after 1 hour even if SHAs match
```

Use `loops --fresh` to bypass the memo for a single invocation, or
`loops refresh` to force a full reindex.
