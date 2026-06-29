# Architecture

Living, per-domain documentation of how `open-loops` works today — the flow,
interfaces, invariants, and the decisions that shaped each domain. This layer
consolidates what used to live across `docs/decisions/` (ADRs),
`docs/superpowers/specs/`, and `docs/superpowers/plans/`; the originals are
preserved in git history.

## How to navigate

Read [`00-overview`](00-overview.md) first — it defines the shared vocabulary
(*open loop*, *evidence snapshot*, *resume context*, *pull-only*) and the
end-to-end flow that the runtime domains (01–08) slot into. Then read whichever
domain you need; each doc is self-contained and cross-links its neighbors.

## Documents

| Doc | Domain |
|---|---|
| [00-overview](00-overview.md) | The open-loop concept, the end-to-end flow across all domains, the pull-only philosophy, and the roadmap. |
| [01-discovery](01-discovery.md) | Discovery of repositories, branches, and worktrees by shelling out to git. |
| [02-sessions-attribution](02-sessions-attribution.md) | Session sources (`SessionSource` trait + the `claude_code` adapter) and attributing session excerpts to a worktree/branch. |
| [03-query-engine](03-query-engine.md) | Parsing a query into a `ScanPlan` and evaluating it in memory; saved contexts. |
| [04-inventory-evidence](04-inventory-evidence.md) | The `loops` listing (ahead/behind, idle-for) and the resume evidence snapshot. |
| [05-resume-distill](05-resume-distill.md) | Building the prompt and invoking the configurable LLM command (stdin→stdout); the `--dry-run` short-circuit. |
| [06-cache-index](06-cache-index.md) | The resume-output cache (keyed `branch@head-sha`) and the SQLite index. |
| [07-config-state](07-config-state.md) | `config.toml`, runtime `state.toml`, and `ignores.toml`. |
| [08-cli-output](08-cli-output.md) | CLI orchestration (clap subcommands) and the shared render layer. |
| [09-build-ci-release](09-build-ci-release.md) | The CI matrix and MSRV gate, and the release-plz + cargo-dist release pipeline. |

## Conventions

Each domain doc follows the same nine sections: **Purpose · Domain map ·
Concepts & vocabulary · Main flow · Interfaces & contracts · Invariants & edge
cases · Decisions · Extension & limitations · References**. The *Main flow*
section carries a Mermaid diagram; the *Decisions* section absorbs the relevant
ADRs (cited as `ex-ADR-00NN` for traceability against git history); code is
cited by `path/file.rs:line`.

User-facing references — feature reference, configuration keys, distribution —
live in [`docs/features.md`](../features.md),
[`docs/configuration.md`](../configuration.md), and
[`docs/distribution.md`](../distribution.md); these docs link to them rather
than restating them.
