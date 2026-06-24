# Features

## `loops` — inventory

```bash
loops
# LOOP                    IDLE FOR  AHEAD  BEHIND
# my-app/feat/login            12d      3       1
```

Unmerged branches across all repos under the configured roots, sorted from
most idle to most recent. No LLM — always fast.

## `loops resume <query>` — context resumption

```bash
loops resume feat/login
```

The query matches by substring against `repo/branch`. Output: `## Why`,
`## Done`, `## Missing`, `## Next step` + `## Sources` (commits and sessions
used — audit if you are unsure). First call invokes the LLM (~30-60s);
repeating is instant (cache per commit). Without AI sessions, context comes
from git only and a "low confidence" warning appears.

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
