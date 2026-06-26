# WAVE 1 — CI Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden CI per `docs/superpowers/specs/2026-06-25-ci-hardening-design.md` — cross-OS matrix, MSRV gate, supply-chain audit, caching, concurrency, SHA-pinned actions, and docs.

**Architecture:** Replace the single `check` job with focused jobs (`fmt`, `test` matrix, `msrv`, `coverage`, `audit` matrix). Add root `deny.toml` and `.github/dependabot.yml`. Fix any Windows path/CRLF failures exposed by the matrix. Document decisions in ADR 0006.

**Tech Stack:** GitHub Actions, cargo-deny 0.16+, Swatinem/rust-cache, dtolnay/rust-toolchain, Rust 1.89 MSRV

**Spec:** `docs/superpowers/specs/2026-06-25-ci-hardening-design.md`  
**ROADMAP:** `ROADMAP.md` §WAVE 1

---

### Task 1: Rewrite `.github/workflows/ci.yml`

**Files:**
- Modify: `.github/workflows/ci.yml` (full rewrite)

- [ ] **Step 1: Add workflow-level `concurrency` and `env`**

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_INCREMENTAL: 0
  CARGO_NET_RETRY: 10
  RUSTUP_MAX_RETRIES: 10
  RUST_BACKTRACE: short
  RUSTFLAGS: "-D warnings"
```

- [ ] **Step 2: Write complete `ci.yml` with SHA-pinned actions**

Use these verified SHAs (tag in comment):

| Action | SHA | Tag |
|--------|-----|-----|
| `actions/checkout` | `9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0` | v7 |
| `dtolnay/rust-toolchain` | `e97e2d8cc328f1b50210efc529dca0028893a2d9` | stable |
| `Swatinem/rust-cache` | `c19371144df3bb44fab255c43d04cbc2ab54d1c4` | v2.9.1 |
| `taiki-e/install-action` | `846871f174ba44370fe1539a9bdbe11d1e309e50` | cargo-llvm-cov |
| `EmbarkStudios/cargo-deny-action` | `bb137d7af7e4fb67e5f82a49c4fce4fad40782fe` | v2.0.20 |

Full workflow:

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_INCREMENTAL: 0
  CARGO_NET_RETRY: 10
  RUSTUP_MAX_RETRIES: 10
  RUST_BACKTRACE: short
  RUSTFLAGS: "-D warnings"

jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
      - uses: dtolnay/rust-toolchain@e97e2d8cc328f1b50210efc529dca0028893a2d9 # stable
        with:
          components: rustfmt
      - run: cargo fmt --check

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
      - uses: dtolnay/rust-toolchain@e97e2d8cc328f1b50210efc529dca0028893a2d9 # stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1
      - run: cargo clippy --all-targets --locked -- -D warnings
      - run: cargo test --locked

  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
      - uses: dtolnay/rust-toolchain@e97e2d8cc328f1b50210efc529dca0028893a2d9 # 1.89
        with:
          toolchain: 1.89
      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1
      - run: cargo check --locked --all-targets

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
      - uses: dtolnay/rust-toolchain@e97e2d8cc328f1b50210efc529dca0028893a2d9 # stable
      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1
      - uses: taiki-e/install-action@846871f174ba44370fe1539a9bdbe11d1e309e50 # cargo-llvm-cov
      - run: cargo llvm-cov --locked --fail-under-lines 70

  audit:
    strategy:
      fail-fast: false
      matrix:
        checks: [advisories, "bans licenses sources"]
    runs-on: ubuntu-latest
    continue-on-error: ${{ matrix.checks == 'advisories' }}
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
      - uses: EmbarkStudios/cargo-deny-action@bb137d7af7e4fb67e5f82a49c4fce4fad40782fe # v2.0.20
        with:
          rust-version: stable
          command: check ${{ matrix.checks }}
          arguments: --all-features --locked
```

- [ ] **Step 3: Verify no movable tags in `uses:`**

Run: `rg 'uses:.*@v[0-9]' .github/workflows/ci.yml`  
Expected: no matches (only full SHAs)

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: harden workflow with cross-OS matrix, MSRV, cache, audit"
```

---

### Task 2: Create `deny.toml`

**Files:**
- Create: `deny.toml`

- [ ] **Step 1: Write `deny.toml` (cargo-deny 0.16+ schema)**

```toml
[graph]
targets = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
]
all-features = true

[advisories]
unmaintained = "workspace"
yanked = "deny"
ignore = []

[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "Unicode-3.0",
    "ISC",
    "BSD-3-Clause",
    "Zlib",
]
confidence-threshold = 0.93
exceptions = [
    { allow = ["MPL-2.0"], crate = "option-ext" },
]

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

- [ ] **Step 2: Validate locally**

Run: `cargo deny check bans licenses sources --all-features --locked`  
Expected: PASS (add licenses to `allow` only if a new dep requires it after review)

Run: `cargo deny check advisories --all-features --locked`  
Expected: PASS or report-only (non-blocking in CI)

- [ ] **Step 3: Commit**

```bash
git add deny.toml
git commit -m "ci: add cargo-deny policy (deny.toml)"
```

---

### Task 3: Create `.github/dependabot.yml`

**Files:**
- Create: `.github/dependabot.yml`

- [ ] **Step 1: Write dependabot config**

```yaml
version: 2
updates:
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: weekly
    groups:
      cargo-minor-patch:
        patterns:
          - "*"
        update-types:
          - minor
          - patch

  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
    groups:
      actions-minor-patch:
        patterns:
          - "*"
        update-types:
          - minor
          - patch
```

- [ ] **Step 2: Commit**

```bash
git add .github/dependabot.yml
git commit -m "ci: add dependabot for cargo and github-actions"
```

---

### Task 4: Documentation (ADR, README badges, CLAUDE.md, CHANGELOG, ROADMAP)

**Files:**
- Create: `docs/decisions/0006-ci-msrv-cross-os.md`
- Modify: `README.md` (badges at top)
- Modify: `CLAUDE.md` (CI/MSRV note)
- Modify: `CHANGELOG.md` (unreleased section)
- Modify: `ROADMAP.md` (mark WAVE 1 items done)

- [ ] **Step 1: Create ADR 0006 (English, matching ADR style)**

Content must cover:
- MSRV 1.89 verified by dedicated `msrv` job
- CI matrix: ubuntu + macos-arm + windows; mac-intel delivered but not tested
- `--locked`, cargo-deny, SHA-pinned actions, concurrency
- Link to spec `docs/superpowers/specs/2026-06-25-ci-hardening-design.md`

- [ ] **Step 2: Add README badges after title**

```markdown
[![CI](https://github.com/carvalhosauro/open-loops/actions/workflows/ci.yml/badge.svg)](https://github.com/carvalhosauro/open-loops/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/open-loops.svg)](https://crates.io/crates/open-loops)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-blue)](rust-toolchain.toml)
[![license](https://img.shields.io/crates/l/open-loops.svg)](LICENSE)
```

- [ ] **Step 3: Add CI note to CLAUDE.md / AGENTS.md dev section**

Short note: CI runs on ubuntu/macos/windows; MSRV 1.89 enforced by `msrv` job.

- [ ] **Step 4: Update CHANGELOG unreleased**

Add under `### Internals` or new `### CI`:
- Cross-OS CI matrix (ubuntu, macos, windows)
- MSRV 1.89 gate, cargo-deny, dependabot, SHA-pinned actions

- [ ] **Step 5: Mark WAVE 1 checkboxes in ROADMAP.md**

- [ ] **Step 6: Commit**

```bash
git add docs/decisions/0006-ci-msrv-cross-os.md README.md CLAUDE.md CHANGELOG.md ROADMAP.md
git commit -m "docs: ADR 0006, CI badges, changelog for WAVE 1"
```

---

### Task 5: Fix Windows test failures (if exposed)

**Files:**
- Modify: as needed (`src/sessions/claude_code.rs`, `tests/cli.rs`, etc.)

- [ ] **Step 1: Run local checks mirroring CI**

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo check --locked --all-targets  # with toolchain 1.89 if available
```

- [ ] **Step 2: Fix Windows-specific issues**

Common fixes per spec:
- `encode_project_path`: handle Windows path separators (`\`) and drive letters
- Tests using hardcoded `/tmp/...` paths → use `tempfile` or `std::env::temp_dir()`
- CRLF in session/jsonl parsing → normalize line endings in tolerant parsers

- [ ] **Step 3: Re-run full test suite**

Run: `cargo test --locked`  
Expected: all 16 integration tests pass

- [ ] **Step 4: Commit fixes**

```bash
git add -A
git commit -m "fix: cross-platform paths for Windows CI matrix"
```

---

### Task 6: Final verification and PR

- [ ] **Step 1: Run full local CI mirror**

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny check bans licenses sources --all-features --locked
cargo deny check advisories --all-features --locked
```

- [ ] **Step 2: Push branch and open PR**

```bash
git push -u origin cursor/ci-hardening-wave1-098e
```

- [ ] **Step 3: Monitor CI until all jobs green**

Jobs: `fmt`, `test (ubuntu-latest)`, `test (macos-latest)`, `test (windows-latest)`, `msrv`, `coverage`, `audit (advisories)`, `audit (bans licenses sources)`

- [ ] **Step 4: Fix any CI failures and push until green**
