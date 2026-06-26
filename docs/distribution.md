# Distribution — crates.io and Homebrew

`open-loops` ships as the `loops` binary through several channels. This document
covers the **minimum setup** for `cargo install` (crates.io) and Homebrew (tap).

| Channel | User command | CI workflow | Secret |
|---|---|---|---|
| crates.io | `cargo install open-loops` | `release-plz.yml` (on merge to `main`) | `CARGO_REGISTRY_TOKEN`, `RELEASE_PLZ_TOKEN` |
| Homebrew | `brew install carvalhosauro/tap/open-loops` | `release.yml` (cargo-dist, on version tag) | `HOMEBREW_TAP_TOKEN` |

Prerelease versions (suffix after patch, e.g. `1.1.0-rc.1`) are outside the
default release-plz flow; handle manually if needed.

---

## One-time checklist

Do these once before the first automated release:

1. **crates.io** — log in at <https://crates.io>, create an API token at
   <https://crates.io/settings/tokens>, add it to this repo as
   **`CARGO_REGISTRY_TOKEN`**. The crate name `open-loops` must be owned by your
   account (first publish can be local: `cargo publish` with the token in env).
2. **release-plz PAT** — create a fine-grained personal access token with
   **Contents: Read and write** and **Pull requests: Read and write** on this
   repo. Add it as **`RELEASE_PLZ_TOKEN`**. release-plz uses this PAT (not the
   workflow `GITHUB_TOKEN`) to push the version tag after publish; otherwise
   `release.yml` would not trigger (GitHub anti-loop rule). See ADR
   [0007](decisions/0007-release-plz-cargo-dist-split.md).
3. **Homebrew tap** — create a public GitHub repo **`carvalhosauro/homebrew-tap`**
   (empty is fine; cargo-dist fills `Formula/` on first release). Create a PAT
   with `repo` scope (or fine-grained `contents: read/write` on the tap) and add
   it here as **`HOMEBREW_TAP_TOKEN`**.

---

## Release flow

1. Conventional Commits land on `main`.
2. **release-plz** (`.github/workflows/release-plz.yml`) opens a Release PR:
   version bump in `Cargo.toml`, `CHANGELOG.md` update (from `cliff.toml`).
3. Merge the Release PR → release-plz publishes to crates.io and pushes tag
   `vX.Y.Z` using `RELEASE_PLZ_TOKEN`.
4. The tag triggers **`release.yml`** → cargo-dist builds binaries, updates the
   Homebrew formula, and creates the GitHub Release.

Local preview: `just changelog` (git-cliff) — does **not** replace the
release-plz changelog step.

---

## Homebrew

cargo-dist builds a formula (`open-loops.rb`) and, with `tap` +
`publish-jobs = ["homebrew"]` in `dist-workspace.toml`, commits it to the tap on
each release.

`release.yml` contains a small customization for the first release: if the tap
repo exists but has no `main` branch yet, the workflow initializes it before
publishing the formula. Keep `allow-dirty = ["ci"]` in `dist-workspace.toml`
while that customization is present so `dist plan` accepts the checked-in
workflow.

**Verify** (after a tagged release):

```bash
brew tap carvalhosauro/homebrew-tap
brew install carvalhosauro/tap/open-loops
loops --version
```

---

## crates.io

release-plz publishes the crate on merge of the Release PR (`cargo publish
--locked` via `CARGO_REGISTRY_TOKEN`). The former `publish-crate.yml` workflow
was removed in favor of release-plz.

**Verify**:

```bash
curl -fsSL https://index.crates.io/op/en/open-loops | tail -1
cargo install open-loops
loops --version
```
