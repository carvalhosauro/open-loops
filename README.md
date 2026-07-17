# open-loops

[![CI](https://github.com/carvalhosauro/open-loops/actions/workflows/ci.yml/badge.svg)](https://github.com/carvalhosauro/open-loops/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/open-loops.svg)](https://crates.io/crates/open-loops)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-blue)](rust-toolchain.toml)
[![license](https://img.shields.io/crates/l/open-loops.svg)](LICENSE)

> **What did I start and not finish? Where did I leave off? What's the next step?**

`loops` is a git companion that finds your paused work — every unmerged branch
across all your repos, listed in seconds — and reconstructs *why you started it
and what's left* by distilling your git history and the AI-coding sessions that
touched it. No notes, no tickets, no ceremony.

<!-- Demo GIF: generate with `agg docs/demo.cast docs/demo.gif`
     (install agg: `cargo install --git https://github.com/asciinema/agg`),
     commit docs/demo.gif, then uncomment:
<p align="center"><img src="docs/demo.gif" alt="loops demo" width="720"></p>
-->

```console
$ loops
scanning git repositories…
LOOP                    IDLE FOR  AHEAD  BEHIND
my-app/feat/login            12d      3       1
api/fix/timeout               2d      1       0

$ loops resume feat/login
# my-app/feat/login
**Confidence:** high

## Why
Adding OAuth login to unblock the onboarding flow.
## Done
- Token validation middleware
- Login form wired to /auth/login
## Remaining
- Refresh token rotation
## Next step
Implement refresh token rotation in auth/refresh.rs and add an expiry test.
```

**Pull-only and local-first.** It captures nothing ahead of time and never writes
inside your repos: it reads git and your AI-session logs *on demand*, so it works
retroactively on branches you already have. The only network call is the LLM
command *you* configure (default `claude -p`; any stdin→stdout command works).
Every reconstruction ships a **confidence score** and an auditable `## Sources`
section — see [SECURITY.md](SECURITY.md).

## Install

```bash
# via cargo
cargo install open-loops

# via script (Linux/macOS)
curl -fsSL https://github.com/carvalhosauro/open-loops/releases/latest/download/open-loops-installer.sh | sh

# via Homebrew (requires carvalhosauro/homebrew-tap — see docs/distribution.md)
brew install carvalhosauro/tap/open-loops
```

## Quickstart

```bash
# 1. register where your repos live
loops init ~/repo

# 2. inventory — open loops, most idle first (<5s, no LLM)
loops
# scanning git repositories…
# LOOP                    IDLE FOR  AHEAD  BEHIND
# my-app/feat/login            12d      3       1
# api/fix/timeout                 2d      1       0

# 3. audit evidence before distilling (no LLM call)
loops resume feat/login --dry-run
# scanning git…
# matching AI sessions…
#
# # my-app/feat/login
#
# **Confidence:** medium — AI sessions found but alignment uncertain — audit Sources before trusting
#
# ## Git
# ...
# ## AI sessions
# - abc123.jsonl (modified 2026-06-18) [in commit window, mentions branch]
#
# ---
# Dry run — LLM not invoked. Run without `--dry-run` to distill.

# 4. resume: why, done, remaining, next step + auditable sources
loops resume feat/login
# scanning git…
# matching AI sessions…
# distilling…
#
# # my-app/feat/login
#
# **Confidence:** medium — AI sessions found but alignment uncertain — audit Sources before trusting
#
# ## Why
# Adding OAuth login to unblock the onboarding flow.
#
# ## Done
# - Token validation middleware
# - Login form wired to `/auth/login`
#
# ## Remaining
# - Refresh token rotation
# - Error states on the form
#
# ## Next step
# Implement refresh token rotation in `auth/refresh.rs` and add a test for expiry.
#
# ## Sources
# - git: branch feat/login (HEAD a1b2c3d)
# - AI session: abc123.jsonl (modified 2026-06-18)
```

Progress lines go to **stderr** so you can pipe or redirect stdout without
losing the distilled document. Long steps (`distilling…`) can take ~30–60s on
a cold run; repeat calls are instant from cache.

State lives in `~/.open-loops/` — nothing is written inside your repos.

## Audit before you trust

Every resume ships a **confidence score** and a **`## Sources`** section — not
metadata for debugging, but the audit trail you use to decide whether to trust
the distillation.

| Score | Meaning | What to do |
|---|---|---|
| `high` | AI sessions overlap branch commits and mention the branch name | Usually safe to continue |
| `medium` | Sessions matched heuristically | Read **Sources**; confirm sessions match this branch |
| `low` | No AI sessions matched — context from git only | Treat as a draft; verify **Sources** and diff yourself |

Recommended flow when confidence is not `high`:

1. `loops resume <branch> --dry-run` — inspect matched commits and sessions (no LLM).
2. Check **`## Sources`** in the full output — do those commits and sessions belong to this work?
3. Run `loops resume <branch>` only when the evidence looks right.

## Why not just…?

| Alternative | What breaks |
|---|---|
| `git branch -a` + re-reading diffs | Shows the *code*, not the context — you still reconstruct "why" and "what's next" in your head, per branch. |
| Notes / journals / TODO files | Require discipline *before* you pause. Nobody writes the note on the branch they abandon in a hurry. |
| Issue trackers | Ceremony up front, drift afterwards — the ticket rarely matches where the branch actually stopped. |
| Grepping old AI session logs | The context is in there, but digging it out by hand is exactly the archaeology `loops` automates. |

The difference is **retroactive**: every alternative needs you to have done
something *at pause time*. `loops` works on branches you already walked away
from, because git history and session logs were captured anyway.

## Demo

Record or replay locally:

```bash
cargo build --release
./scripts/demo.sh          # runs the quickstart flow in a temp dir
asciinema play docs/demo.cast   # replay the bundled recording
```

## Docs

Full reference in [`docs/`](docs/): [setup](docs/setup.md) ·
[features](docs/features.md) · [configuration](docs/configuration.md).

## Contributing

Contributions are welcome. New here? [`ONBOARDING.md`](ONBOARDING.md) gets you from
clone to a first change in minutes. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the
full dev workflow and conventions, [`SECURITY.md`](SECURITY.md) to report a
vulnerability, and the [Code of Conduct](CODE_OF_CONDUCT.md). Good first issues are
labelled [`good first issue`](https://github.com/carvalhosauro/open-loops/labels/good%20first%20issue).

## License

MIT OR Apache-2.0.

---

If `loops` rescued a branch you'd given up on, [star the repo](https://github.com/carvalhosauro/open-loops)
and tell the friend whose `git branch` output looks like a graveyard — that's how
they'll find it.
