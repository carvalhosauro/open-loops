# open-loops

> What did I start and not finish? Where did I leave off? What's the next step?

`loops` lists your paused work (unmerged branches across all your repos) and
reconstructs resume context from your AI sessions and git — without you
documenting anything.

## Install

```bash
# via cargo
cargo install open-loops

# via script (Linux/macOS)
curl -fsSL https://github.com/carvalhosauro/open-loops/releases/latest/download/open-loops-installer.sh | sh

# via Homebrew (after v0.1.0 release)
brew install carvalhosauro/tap/open-loops
```

## Quickstart

```bash
# 1. register where your repos live
loops init ~/repo

# 2. inventory — open loops, most idle first (<5s, no LLM)
loops
# LOOP                    IDLE FOR  AHEAD  BEHIND
# my-app/feat/login            12d      3       1
# api/fix/timeout                 2d      1       0

# 3. audit evidence before distilling (no LLM call)
loops resume feat/login --dry-run
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

State lives in `~/.open-loops/` — nothing is written inside your repos.

Every resume output includes a **confidence score** (`high` / `medium` / `low`)
and a **Sources** section so you can audit the evidence before trusting the
distillation.

Full docs in [`docs/`](docs/): [setup](docs/setup.md) ·
[features](docs/features.md) · [configuration](docs/configuration.md).

## Demo

Record or replay locally:

```bash
cargo build --release
./scripts/demo.sh          # runs the quickstart flow in a temp dir
asciinema play docs/demo.cast   # replay the bundled recording
```

## License

MIT OR Apache-2.0.
