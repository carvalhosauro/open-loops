# ADR 0006: CI, MSRV, and cross-OS testing

Date: 2026-06-26 · Status: accepted

Spec: [CI hardening design](../superpowers/specs/2026-06-25-ci-hardening-design.md)

## Decision

1. **MSRV 1.89** is a verified contract: dedicated `msrv` job runs
   `cargo check --locked --all-targets` on Rust 1.89 (matches `rust-toolchain.toml`).
2. **CI test matrix:** `ubuntu-latest`, `macos-latest` (arm64), `windows-latest`
   with `fail-fast: false`.
3. **mac-intel** (`x86_64-apple-darwin`) is shipped via cargo-dist but **not**
   tested in CI — accepted trade-off (same codebase as mac-arm; arch rarely
   diverges for git/path shell-out).
4. **`--locked`** on clippy, test, msrv, and coverage jobs.
5. **Supply-chain:** `cargo-deny` via `deny.toml`; advisories non-blocking,
   licenses/bans/sources blocking.
6. **Workflow hygiene:** SHA-pinned GitHub Actions; `concurrency` with
   `cancel-in-progress` per workflow+ref.

## Rationale

`loops` shells out to git and walks the filesystem — behavior that diverges
across OSes. Pinning MSRV without a job that enforces it is a false promise.
`--locked` aligns CI with what publish and users receive. cargo-deny covers
advisories and license policy for a crates.io + Homebrew artifact.

## Consequences

- Contributors need green CI on all three OSes before merge.
- Intel mac users rely on release binaries without CI coverage on that target.
- Dependabot maintains SHA pins for GitHub Actions and Cargo deps.
