# Features

## `loops` — inventory

```bash
loops
# LOOP                    IDLE FOR  AHEAD  BEHIND
# my-app/feat/login            12d      3       1
```

Unmerged branches across all repos under the configured roots, sorted from
most idle to most recent. Progress on stderr: `scanning git repositories…`.
No LLM — always fast.

Discovery is layout-agnostic: normal repos, worktrees, and bare stores under your
configured roots are found automatically. Repo names come from git's common-dir,
not from worktree folder names.

Scan results are cached in a disposable SQLite index at `~/.open-loops/index.db`.
When a repo's refs are unchanged since the last run, the cached loops are served
and the heavy git phase (`for-each-ref`, `branch --merged`, per-branch
`rev-list`) is skipped — so the second call is noticeably faster on large
many-branch repos. Ahead/behind counts are also memoised in
`~/.open-loops/inventory/` by `(branch, head_sha, default_sha)`.

**Git is always the source of truth; the index is a throwaway cache.** It is
safe to delete `~/.open-loops/index.db` at any time — the next run rebuilds it.
A corrupt index self-heals (it is rebuilt from git, with a one-line warning) and
never aborts a command.

```bash
loops --fresh             # bypass the cache and recompute all ahead/behind
```

### Filtering

```bash
loops api                 # repos/branches matching "api"
loops api idle:>7d        # plus idle more than 7 days
loops repo:api branch:fix/ behind:>0
loops +ignored            # include dismissed loops
```

Bare terms substring-match the repo, branch, or key (AND across terms).
Attributes: `repo:`, `branch:`, `key:`, `root:` (substring), `idle:` (needs a
comparator, e.g. `idle:>7d`; units m/h/d/w), `ahead:`/`behind:` (`>`,`<`,`>=`,
`<=`, or bare equality). Tags: `-ignored` (default), `+ignored`.

### Contexts

Named scopes via `@` tokens. Definitions live in `config.toml` (`[contexts.X]`);
the active context is persisted in `state.toml` when you pass `@name` on the CLI.
A context is your daily universe — work vs personal — not the same as reports
(`:` — coming in phase 5).

```bash
loops @work              # switch to work (saved in state.toml)
loops                     # uses current context from state.toml
loops @none               # clear state and show full universe
loops @work api idle:>7d  # compose context + ad-hoc filters
```

`@none` and `@all` clear `state.toml`. Only one `@` token per query. See
`docs/configuration.md` for `[contexts.X]` setup.

## `loops refresh [query]` — reindex inventory

```bash
loops refresh             # reindex all repos
loops refresh api         # reindex repos matching "api"
```

Forces a full recomputation of ahead/behind for all (or filtered) repos and
rebuilds the SQLite index rows for them. It also prunes both stores' orphans —
the inventory files (`~/.open-loops/inventory/<hash>.json`) and the index rows
(`~/.open-loops/index.db`) for repos that have been deleted or moved off disk.
Prints `refreshed N repos` on stderr when complete.

## `loops resume <query>` — context resumption

```bash
loops resume feat/login
loops resume feat/login --dry-run   # audit evidence without calling the LLM
loops resume feat/login --fresh     # bypass inventory memo
```

The query matches by substring against `repo/branch`. Progress on stderr:
`scanning git…` → `matching AI sessions…` → `distilling…` (skipped for
`--dry-run` and cache hits). Output on stdout: `## Why`, `## Done`,
`## Remaining`, `## Next step` + `## Sources` (commits and sessions used —
**audit this section if confidence is not high**). First call invokes the LLM
(~30-60s); repeating is instant (cache per commit).

Each resume includes a **confidence score** at the top:

| Score | Meaning |
|---|---|
| `high` | AI sessions overlap branch commits and mention the branch name |
| `medium` | Sessions matched heuristically — verify Sources before trusting |
| `low` | No AI sessions — context from git only |

`--dry-run` prints the git log, diffstat, and matched sessions that would feed
the LLM, without invoking it.

Sessions are matched against the **worktree where the branch is checked out**.
In bare+worktree layouts each branch lives in its own directory, so `loops resume`
looks up the AI sessions recorded for that directory — not the container. A branch
with no worktree falls back to the repo path: commits and diffstat still distill,
but session excerpts are typically empty (the AI never ran there).

## `loops ignore <repo/branch>` — dismiss

```bash
loops ignore my-app/feat/old-experiment
```

Removes the loop from the list (the branch is not touched). To undo, edit
`~/.open-loops/ignores.toml`.

## `loops init <dir>...` — register roots

```bash
loops init ~/repo
```

## `loops worktrees` (alias `wt`) — worktree inventory

```bash
loops worktrees
# WORKTREE          BRANCH       IDLE  MERGED  STATE  VERDICT
# my-app/fix-bug    fix/bug       8d   yes     clean  deletable
# api/spike-redis   spike/redis   40d  no      clean  cold
```

Lists every git worktree across the configured roots with a cleanup verdict:

- `deletable` — merged into the default branch and clean; safe to remove.
- `cold` — not merged, clean; review candidate.
- `active` — has uncommitted changes; live work, left alone.
- `prunable` — directory gone / orphaned; `git worktree prune` clears it.
- `home` — the main worktree; never removed.

For `deletable`/`prunable` worktrees it prints the exact cleanup command to copy.
It never deletes anything itself.

## `loops completions <shell>` — shell autocomplete

```bash
loops completions zsh > ~/.zfunc/_loops   # zsh
loops completions bash > /etc/bash_completion.d/loops
loops completions fish > ~/.config/fish/completions/loops.fish
```

Prints a completion script for the given shell (`bash`, `zsh`, `fish`, ...).
