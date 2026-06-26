# ADR 0007: release-plz and cargo-dist split

Date: 2026-06-26 · Status: accepted

Spec: [Release completeness + automation](../superpowers/specs/2026-06-26-release-completeness-automation-design.md)

## Decision

1. **release-plz** owns version bump, `CHANGELOG.md` updates (via existing
   `cliff.toml`), crates.io publish, and the `v{{ version }}` git tag.
2. **cargo-dist** (`release.yml`) remains the sole owner of GitHub Release
   assets, cross-platform binaries, and the Homebrew formula (`git_release_enable
   = false` in `release-plz.toml` avoids duplicate GitHub Releases).
3. **Tag handoff** uses a fine-grained PAT stored as `RELEASE_PLZ_TOKEN`, not
   the workflow `GITHUB_TOKEN`. Tags pushed with the default token do not trigger
   other workflows (GitHub anti-loop protection), so `release.yml` would never
   run without the PAT.
4. **`publish-crate.yml` is removed** — crates.io publish is handled by
   release-plz on merge of the Release PR to `main`.

## Rationale

Manual release (version edit, `just changelog`, tag push, separate publish
workflow) is error-prone. release-plz is designed for solo crates with
Conventional Commits and composes with cargo-dist rather than replacing it.
cargo-dist does not publish to crates.io; release-plz does not build binaries or
update the Homebrew tap.

## Flow

```
merge Release PR → main
  → release-plz: bump version, update CHANGELOG, publish crates.io, push tag vX.Y.Z (PAT)
  → tag triggers release.yml → cargo-dist: binaries, Homebrew formula, GitHub Release
```

## Consequences

- Repo secrets: `RELEASE_PLZ_TOKEN` (PAT with Contents + Pull requests write),
  `CARGO_REGISTRY_TOKEN`, `HOMEBREW_TAP_TOKEN` (unchanged).
- `just changelog` is a local preview only; release changelog is updated by
  release-plz when the Release PR merges.
- First end-to-end patch release after merge validates the PAT → tag → cargo-dist
  handoff.
- **Archive vs installer:** WAVE 2 puts completions and `loops.1` in the release
  tarball (`dist-artifacts/`). cargo-dist 0.32 does not yet wire them into the
  Homebrew formula or shell installer paths (upstream limitation). Users who
  install from the archive can copy files manually; `loops completions` remains
  the local generation path.
