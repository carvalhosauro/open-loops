# ADR 0005: Repository discovery via git interrogation

Date: 2026-06-25 · Status: accepted

## Context

`find_repos` originally treated only `dir/.git` directories as repositories.
Bare stores and worktrees use `.git` files or no `.git` at all, so discovery
returned zero repos in bare+worktree layouts.

## Decision

1. Mark FS candidates when `.git` exists (file or directory) or a cheap bare
   probe matches (`HEAD` + `objects/` + `refs/`).
2. Resolve each candidate with `git rev-parse --path-format=absolute --git-common-dir`.
3. Deduplicate by that absolute common-dir. N worktrees → one logical repo.
4. Derive `repo_name` from the common-dir basename (`.git`/`.bare` → parent name;
   `foo.git` → `foo`).
5. Replace fixed walk depth with configurable `scan_depth` (default 4).

## Rationale

Layout-specific path heuristics (`.bare`, worktree folder names) would encode
one author's tree. Git already exposes the canonical store identity. Shell-out
matches ADR 0002. The 3-segment key (`root_label/repo/branch` per ADR 0003)
is unchanged — only the source of `repo_name` changes.

## Consequences

- `find_repos` returns `(repos, warnings)`; failed candidates warn, never abort.
- Future inventory cache (ADR 0003 phase 3) should hash the absolute common-dir.
- Isolated `.bare` dirs hidden under a dot-prefixed parent are not discovered
  unless a root points at them or the container `.git` pointer exists (documented).
- Spec Fase B (implemented) attributes AI sessions per worktree: `open_loops`
  resolves `OpenLoop.repo_path` to the branch's worktree via `git worktree list`
  (fallback: container/common-dir). The common-dir stays the dedup/identity
  anchor; `repo_path` never enters the canonical key.
