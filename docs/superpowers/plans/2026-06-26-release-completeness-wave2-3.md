# WAVE 2/3 — Release Completeness + Automation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete release artifacts (completions, man page, docs in tarball) and automate version/changelog/crates.io publish via release-plz, while cargo-dist keeps binaries/Homebrew.

**Architecture:** Extract `Cli`/`Command` to `src/cli_command.rs` for shared use in `build.rs` and runtime. `build.rs` generates 4 shell completions + `loops.1` into `OUT_DIR`, then copies to gitignored `dist-artifacts/` for cargo-dist `include`. Add `release-plz.toml` + workflow with PAT handoff; delete `publish-crate.yml`.

**Tech Stack:** Rust 1.89, clap 4, clap_complete, clap_mangen, cargo-dist 0.32.0, release-plz action v0.3.159

**Spec:** `docs/superpowers/specs/2026-06-26-release-completeness-automation-design.md`  
**ROADMAP:** `ROADMAP.md` §WAVE 2/3

---

### Task 1: Extract shared CLI command definition

**Files:**
- Create: `src/cli_command.rs`
- Modify: `src/cli.rs` (move `Cli`/`Command`, re-export)

- [ ] **Step 1: Create `src/cli_command.rs`**

```rust
//! Clap command surface shared by runtime and `build.rs` (via `include!`).
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "loops", version, about = "Recover the context of paused work")]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Filter the inventory (e.g. `loops api idle:>7d`). See ADR 0003 grammar.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub query: Vec<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Register repository roots (e.g. loops init ~/repo)
    Init { paths: Vec<PathBuf> },
    /// Distill a loop's context: why, done, remaining, next step
    Resume {
        query: String,
        /// Show matched git commits and AI sessions without calling the LLM
        #[arg(long)]
        dry_run: bool,
    },
    /// Drop a dead loop from the list (repo/branch format)
    Ignore { key: String },
    /// List git worktrees with a cleanup verdict (alias: wt)
    #[command(visible_alias = "wt")]
    Worktrees,
    /// Generate a shell completion script (bash, zsh, fish, ...)
    Completions { shell: clap_complete::Shell },
}
```

- [ ] **Step 2: Update `src/cli.rs`**

Replace `Cli`/`Command` definitions with:

```rust
mod cli_command;
pub use cli_command::{Cli, Command};
```

- [ ] **Step 3: Verify**

Run: `cargo test`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli_command.rs src/cli.rs
git commit -m "refactor(cli): extract shared clap command for build.rs"
```

---

### Task 2: Add `build.rs` — completions + man page

**Files:**
- Create: `build.rs`
- Modify: `Cargo.toml` (`[build-dependencies]`)
- Modify: `.gitignore` (add `dist-artifacts/`)

- [ ] **Step 1: Add build-dependencies to `Cargo.toml`**

```toml
[build-dependencies]
clap = { version = "4", features = ["derive"] }
clap_complete = "4"
clap_mangen = "0.2"
```

- [ ] **Step 2: Create `build.rs`**

```rust
use clap::CommandFactory;
use clap_complete::{generate_to, Shell};
use std::{env, fs, path::PathBuf};

include!("src/cli_command.rs");

fn main() {
    let outdir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let mut cmd = Cli::command();

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
        generate_to(shell, &mut cmd, "loops", &outdir).expect("generate completions");
    }

    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf).expect("render man page");
    fs::write(outdir.join("loops.1"), buf).expect("write man page");

    // Stable path for cargo-dist `include` (gitignored).
    let artifacts = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("dist-artifacts");
    fs::create_dir_all(&artifacts).expect("create dist-artifacts");
    for entry in fs::read_dir(&outdir).expect("read OUT_DIR") {
        let entry = entry.expect("dirent");
        let path = entry.path();
        if path.is_file() {
            let dest = artifacts.join(entry.file_name());
            fs::copy(&path, &dest).expect("copy artifact");
        }
    }

    println!("cargo:rerun-if-changed=src/cli_command.rs");
}
```

- [ ] **Step 3: Add to `.gitignore`**

```
dist-artifacts/
```

- [ ] **Step 4: Build and verify artifacts**

Run:
```bash
cargo build
ls dist-artifacts/
```
Expected: `_loops`, `_loops.ps1`, `loops.bash`, `loops.fish`, `loops.1`, `loops.zsh` (clap_complete naming)

Run: `man -l dist-artifacts/loops.1` (or `groff -man -Tascii dist-artifacts/loops.1 | head`)
Expected: man page renders

- [ ] **Step 5: Commit**

```bash
git add build.rs Cargo.toml Cargo.lock .gitignore
git commit -m "feat(build): generate shell completions and man page at compile time"
```

---

### Task 3: Bundle artifacts in cargo-dist archive

**Files:**
- Modify: `dist-workspace.toml`

- [ ] **Step 1: Add `include` and explicit `auto-includes`**

```toml
[dist]
# ...existing keys...
auto-includes = true
include = [
  "dist-artifacts/",
]
```

- [ ] **Step 2: Verify locally (if cargo-dist installed)**

Run: `cargo dist build --artifacts=local` (optional; CI validates)
Or: confirm `dist-artifacts/` populated after `cargo build` and config paths are correct.

- [ ] **Step 3: Commit**

```bash
git add dist-workspace.toml
git commit -m "feat(dist): bundle completions and man page in release archives"
```

---

### Task 4: Complete `Cargo.toml` publish metadata + release profile

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add package metadata and profile**

```toml
[package]
# ...existing...
rust-version = "1.89"
authors = ["Gustavo Carvalho <gustavoooliveiradecarvalho@gmail.com>"]
readme = "README.md"
categories = ["command-line-utilities", "development-tools"]
keywords = ["git", "worktree", "context", "ai", "agents"]

[profile.release]
lto = "thin"
strip = true
```

- [ ] **Step 2: Verify publish metadata**

Run: `cargo publish --dry-run --locked`
Expected: PASS (may warn crate already exists — OK)

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore(cargo): add crates.io metadata and release profile"
```

---

### Task 5: release-plz automation

**Files:**
- Create: `release-plz.toml`
- Create: `.github/workflows/release-plz.yml`
- Delete: `.github/workflows/publish-crate.yml`

- [ ] **Step 1: Create `release-plz.toml`**

```toml
[workspace]
publish = true
git_release_enable = false
git_tag_name = "v{{ version }}"
changelog_update = true
```

- [ ] **Step 2: Create `.github/workflows/release-plz.yml`**

Use SHA-pinned actions (match WAVE 1):

```yaml
name: release-plz
on:
  push:
    branches: [main]
permissions:
  contents: write
  pull-requests: write
concurrency:
  group: release-plz-${{ github.ref }}
  cancel-in-progress: false
jobs:
  release-plz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@e97e2d8cc328f1b50210efc529dca0028893a2d9 # stable
      - name: Run release-plz
        uses: release-plz/action@e8792575c7f2366cf6ff3ccc33ead9ace5b691c7 # v0.3.159
        env:
          GITHUB_TOKEN: ${{ secrets.RELEASE_PLZ_TOKEN }}
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

- [ ] **Step 3: Delete `publish-crate.yml`**

- [ ] **Step 4: Commit**

```bash
git add release-plz.toml .github/workflows/release-plz.yml
git rm .github/workflows/publish-crate.yml
git commit -m "feat(release): automate version bump and crates.io publish via release-plz"
```

---

### Task 6: Documentation + ADR

**Files:**
- Create: `docs/decisions/0007-release-plz-cargo-dist-split.md`
- Modify: `CLAUDE.md`, `AGENTS.md`, `docs/distribution.md`, `ROADMAP.md`
- Modify: `docs/superpowers/specs/2026-06-25-ci-hardening-design.md` (obsolete manual changelog DoD)
- Modify: `justfile` (changelog comment)

- [ ] **Step 1: ADR 0007** — document split: release-plz owns version/changelog/crates.io/tag; cargo-dist owns binaries/Homebrew; PAT `RELEASE_PLZ_TOKEN` for tag push handoff.

- [ ] **Step 2: Update release docs** — new flow: merge Release PR → release-plz publishes + tags → `release.yml` builds binaries. `just changelog` is local preview only. Document `RELEASE_PLZ_TOKEN` secret.

- [ ] **Step 3: Mark ROADMAP WAVE 2/3 items complete**

- [ ] **Step 4: Commit**

```bash
git add docs/decisions/0007-release-plz-cargo-dist-split.md CLAUDE.md AGENTS.md docs/distribution.md ROADMAP.md docs/superpowers/specs/2026-06-25-ci-hardening-design.md justfile
git commit -m "docs: ADR 0007 and release-plz workflow documentation"
```

---

### Task 7: CI verification

- [ ] **Step 1: Run full local gate**

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo publish --dry-run --locked
```

- [ ] **Step 2: Push branch and open PR**

- [ ] **Step 3: Fix any CI failures until green**

---

## Acceptance criteria mapping

| Criterion | Task |
|-----------|------|
| `build.rs` completions + man | Task 2 |
| `clap_mangen` + shared clap | Tasks 1–2 |
| `dist-workspace.toml` bundles artifacts | Task 3 |
| LICENSE/README/CHANGELOG in tarball | Task 3 (`auto-includes = true`) |
| `loops completions` preserved | Task 1 (unchanged handler) |
| `Cargo.toml` metadata + profile | Task 4 |
| release-plz config + workflow | Task 5 |
| `RELEASE_PLZ_TOKEN` documented | Task 6 |
| `publish-crate.yml` deleted | Task 5 |
| ADR 0007 | Task 6 |
| End-to-end patch release | Post-merge (requires repo secrets); document in PR |
