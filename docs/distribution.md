# Distribution — crates.io and Homebrew

`open-loops` ships as the `loops` binary through several channels. This document
covers the **minimum setup** for `cargo install` (crates.io) and Homebrew (tap).

Both channels trigger on a version tag (`vX.Y.Z` or `X.Y.Z`). Prerelease tags
(with a `-` suffix, e.g. `v1.1.0-rc.1`) skip the publish steps.

| Channel | User command | CI workflow | Secret |
|---|---|---|---|
| crates.io | `cargo install open-loops` | `publish-crate.yml` | `CARGO_REGISTRY_TOKEN` |
| Homebrew | `brew install carvalhosauro/tap/open-loops` | `release.yml` (cargo-dist) | `HOMEBREW_TAP_TOKEN` |

---

## One-time checklist

Do these once before the first tagged release that should go live on both
channels:

1. **crates.io** — log in at <https://crates.io>, create an API token at
   <https://crates.io/settings/tokens>, add it to this repo as
   **`CARGO_REGISTRY_TOKEN`**. The crate name `open-loops` must be owned by your
   account (first publish can be local: `cargo publish` with the token in env).
2. **Homebrew tap** — create a public GitHub repo **`carvalhosauro/homebrew-tap`**
   (empty is fine; cargo-dist fills `Formula/` on first release). Create a PAT
   with `repo` scope (or fine-grained `contents: read/write` on the tap) and add
   it here as **`HOMEBREW_TAP_TOKEN`**.
3. **Tag a release** — bump `version` in `Cargo.toml`, update `CHANGELOG.md`,
   commit, then:

   ```bash
   git tag vX.Y.Z && git push origin vX.Y.Z
   ```

   CI runs `release.yml` (binaries + Homebrew formula) and `publish-crate.yml`
   (`cargo publish`).

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

cargo-dist does not publish Rust library/binary crates to crates.io; the
dedicated workflow verifies tag ↔ `Cargo.toml` version, skips if already
published, then runs `cargo publish --locked`.

**Verify**:

```bash
curl -fsSL https://index.crates.io/op/en/open-loops | tail -1
cargo install open-loops
loops --version
```
