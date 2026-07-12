# Onboarding — open-loops

Welcome. This gets a new contributor from clone to a green test run to a first
change in a few minutes. For the full conventions see
[`CONTRIBUTING.md`](CONTRIBUTING.md); this file is the fast path.

## 1. Prerequisites

- **Rust 1.89** (the pinned MSRV). `rust-toolchain.toml` selects it automatically;
  you only need `rustc`/`cargo` and `git` on `PATH`.
- Optional: [`just`](https://github.com/casey/just) and
  [`lefthook`](https://github.com/evilmartians/lefthook) for the wrapped tasks and
  git hooks. Without them, run the `cargo` commands directly.

## 2. Clone, build, test

```bash
git clone https://github.com/carvalhosauro/open-loops
cd open-loops

just setup     # install git hooks (or skip if you don't have lefthook)
just test      # cargo test — creates real throwaway git repos in a tempdir (~30s)
just lint      # cargo clippy --all-targets -- -D warnings
just fmt       # cargo fmt
```

No `just`? The equivalents:

```bash
cargo +1.89 test
cargo +1.89 clippy --all-targets -- -D warnings
cargo +1.89 fmt --check
```

> **Format with the pinned toolchain.** CI's `fmt` job uses rustfmt 1.89. A newer
> local rustfmt can format a `match` arm differently and pass locally while CI
> fails — always verify with `cargo +1.89 fmt --check`.

## 3. Run it against your own repos

```bash
cargo run -- init ~/code        # register a root
cargo run --                    # list your open loops (no LLM needed)
cargo run -- resume <branch> --dry-run   # inspect resume evidence without an LLM
```

State lives in `~/.open-loops/` (override with `OPEN_LOOPS_HOME=/tmp/ol-test` to
experiment without touching your real config). Nothing is written inside your git
repos.

## 4. The map

Start at the architecture overview and follow the domain you're touching:

- [`docs/architecture/00-overview.md`](docs/architecture/00-overview.md) — the
  end-to-end flow and shared vocabulary. Read this first.
- The eight domain docs (discovery, sessions, query engine, inventory, distill,
  cache/index, config/state, CLI) each map to their source files.
- [`docs/features.md`](docs/features.md) and
  [`docs/configuration.md`](docs/configuration.md) are the source of truth for
  commands and config.

Code layout is summarised in [`CLAUDE.md`](CLAUDE.md) / [`AGENTS.md`](AGENTS.md).

## 5. Make a change

1. Branch off `main`.
2. Write the change **and** its test. Integration tests build real git repos in a
   tempdir (`src/testutil.rs`); prefer that over mocking git. Error tests assert
   on typed variants with `matches!`, not on git's stderr text.
3. Update the docs in the same PR — docs are part of Done.
4. `just test && just lint && just fmt` (or the `cargo +1.89` equivalents).
5. Open a PR with a [Conventional Commits](https://www.conventionalcommits.org/)
   title (e.g. `feat(query): …`). A hook validates the format.

## 6. Where to start

Look for [`good first issue`](https://github.com/carvalhosauro/open-loops/labels/good%20first%20issue)
issues. Good starter areas, in rough order of ramp-up:

- **A new query filter or output tweak** — the query engine (`src/query.rs`) is
  pure and well-tested; a small feature there is a gentle first PR.
- **A new session harness adapter** — implement `SessionSource`
  (`src/sessions/mod.rs`) for Codex CLI / OpenCode / another AI tool. High-impact:
  it widens who the tool works for. The adapter is isolated behind the trait.
- **Docs and examples** — always welcome and a low-risk way to learn the codebase.

Questions? Open an issue before a large PR so we can agree on the approach.
