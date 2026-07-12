# Contributing to open-loops

Thanks for your interest in improving `loops`. This project is a small, focused
Rust CLI; contributions that keep it small and focused are the most welcome.

## Before you start

- **Open an issue first for anything non-trivial.** Bug reports and small fixes
  can go straight to a PR, but for a new feature, a behaviour change, or a
  refactor, please open an issue so we can agree on the approach before you spend
  time on it.
- **Read the architecture map.** [`AGENTS.md`](AGENTS.md) points at
  [`docs/architecture/`](docs/architecture/) (start with `00-overview.md`). The
  source of truth for commands and configuration is
  [`docs/features.md`](docs/features.md) and
  [`docs/configuration.md`](docs/configuration.md).

## Development

The [`justfile`](justfile) wraps the common tasks:

```bash
just setup   # install git hooks (lefthook)
just test    # cargo test
just lint    # cargo clippy --all-targets -- -D warnings
just fmt     # cargo fmt
just cov     # coverage; gate 70% overall (core modules target 85%)
```

If you don't have `just`, run the underlying `cargo` commands directly (see the
`justfile` and [`CLAUDE.md`](CLAUDE.md)). The minimum supported Rust version
(MSRV) is **1.89**, enforced by a dedicated CI job; the toolchain is pinned in
`rust-toolchain.toml`.

Before opening a PR, make sure `just test`, `just lint`, and `just fmt` all pass.
CI runs on Linux, macOS, and Windows.

## Conventions

- **Conventional Commits.** Commit messages follow
  [Conventional Commits](https://www.conventionalcommits.org/) (e.g.
  `feat(cli): …`, `fix(scanner): …`, `docs: …`). A git hook validates the format.
- **English only.** All code, comments, and user-facing strings — including error
  messages — are in English. Error messages should be actionable (say what to do
  next).
- **Library API returns typed errors.** Public library functions return
  domain-specific `thiserror` enums (`QueryError`, `GitError`, …) aggregated as
  `OpenLoopsError` at the CLI boundary — no `anyhow` in the library.
- **Docs are part of Done.** When you change behaviour, update the relevant docs
  (`docs/features.md`, `docs/configuration.md`, and the architecture layer) in the
  same PR. The PR checklist treats docs as part of the Definition of Done.
- **Tests create real git repos.** Integration tests build throwaway git
  repositories in a tempdir (`src/testutil.rs`); prefer that over mocking git.

## Reporting a security issue

Please do **not** open a public issue for security problems. See
[`SECURITY.md`](SECURITY.md) for how to report privately.

## Code of Conduct

This project adheres to the [Contributor Covenant](CODE_OF_CONDUCT.md). By
participating, you are expected to uphold it.
