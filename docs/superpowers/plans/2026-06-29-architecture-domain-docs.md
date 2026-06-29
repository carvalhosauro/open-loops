# Architecture Domain Docs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fragmented spec/plan/ADR docs with a single living layer in `docs/architecture/` — one flow-focused, English document per domain.

**Architecture:** Eleven Markdown files under `docs/architecture/` (`README.md`, `00-overview.md`, `01`–`09`). Each domain doc follows a fixed 9-section template, embeds a Mermaid flow diagram, and absorbs the relevant ADRs into a *Decisions* section. After the new layer exists, the consolidated sources (`docs/decisions/`, `docs/audit/`, `docs/superpowers/plans/`, and `docs/superpowers/specs/` except the unimplemented `library-maturity-oss-health`) are deleted — git preserves their history.

**Tech Stack:** Markdown + Mermaid. Source of truth is the Rust crate in `src/`. No code changes.

**Reference spec:** `docs/superpowers/specs/2026-06-29-architecture-domain-docs-design.md`

## Global Constraints

- **English-first.** Every file in `docs/architecture/` is written in English. Content absorbed from PT-BR ADRs/specs/plans is *re-written* in English, not machine-translated.
- **Template (9 sections), in order:** Purpose · Domain map · Concepts & vocabulary · Main flow (+ Mermaid) · Interfaces & contracts · Invariants & edge cases · Decisions · Extension & limitations · References. Scale each section to the domain's complexity; never drop a section header.
- **Mermaid in every domain doc** (`00`–`09`): a `flowchart` or `sequenceDiagram` fenced block in *Main flow*. README needs none.
- **Decisions traceability:** when absorbing an ADR, cite its original number, e.g. `*(ex-ADR-0003)*`, so git history stays linkable.
- **References are real:** every `path/file.rs:line` citation must point at code that exists at authoring time. Verify by opening the cited line.
- **Do not duplicate user-facing docs.** Link to `docs/features.md`, `docs/configuration.md`, `docs/distribution.md`, `docs/setup.md` instead of restating them.
- **Conventional Commits.** Error/message copy quoted from code stays verbatim (English already).
- **Do not touch `src/`.** This is documentation only.

---

## Per-domain-doc procedure (implicitly part of Tasks 1–10)

Every domain-doc task runs this exact loop. Only the *inputs* (source files, absorbed ADRs/specs/plans, diagram focus) differ per task and are listed in the task.

- [ ] **Step A — Read the code.** Open every source file listed in the task's **Source** block. Note public entry points (`pub fn/struct/enum/trait`), the main call path, error types, and invariants enforced in code (not assumed).
- [ ] **Step B — Read the absorbed sources.** Open every ADR/spec/plan in the task's **Absorbs** block. Extract: the decision, its context, and the trade-off/alternatives — for the *Decisions* section.
- [ ] **Step C — Write the doc** at the task's **Create** path, all 9 template sections, in English. *Main flow* gets the Mermaid diagram. *Interfaces & contracts* shows real signatures copied from code. *References* lists the key `file:line` anchors and the relevant tests.
- [ ] **Step D — Verify references.** For each `file:line` cited, open it and confirm the symbol/line matches. Fix any drifted line numbers.
- [ ] **Step E — Verify Mermaid.** Validate the fenced `mermaid` block. If network is available: `npx -y @mermaid-js/mermaid-cli -i <file> -o /tmp/mmd.svg` (must exit 0). Otherwise syntax-review the block against Mermaid grammar (balanced nodes, valid arrows, no reserved-word node ids).
- [ ] **Step F — Language pass.** Re-read; confirm 100% English, no leftover PT-BR from the absorbed sources.
- [ ] **Step G — Commit.** `git add docs/architecture/<file> && git commit -m "docs(arch): add <domain> domain doc"`

---

## Task 1: `00-overview.md` + directory scaffold

Establishes the shared vocabulary and the cross-cutting principles every other doc links back to. Must come first.

**Files:**
- Create: `docs/architecture/00-overview.md`
- Source: `src/lib.rs`, `src/main.rs`, `README.md`, `docs/features.md`
- Absorbs: `docs/decisions/0001-mvp-pull-only.md` *(ex-ADR-0001)*, `docs/decisions/0002-git-e-llm-via-shell-out.md` *(ex-ADR-0002, the cross-cutting "git + LLM via shell-out" principle)*, `docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md`, `docs/superpowers/plans/2026-06-10-open-loops-mvp.md` (+ `.STATUS.md`)

**Produces (consumed by all later docs):** the canonical glossary terms — *open loop*, *evidence snapshot*, *resume context*, *pull-only* — and the end-to-end flow diagram the domain docs slot into. Use these exact term spellings everywhere.

- [ ] **Step 1:** Run the **Per-domain-doc procedure** Steps A–B with the Source/Absorbs above.
- [ ] **Step 2:** Write `00-overview.md`. *Main flow* = a single end-to-end `flowchart` showing how a `loops` invocation traverses the 8 runtime domains (discover → attribute sessions → query/filter → build inventory+evidence → optionally resume/distill), with cache/index and config/state as side stores. *Extension & limitations* must include the **roadmap** entry pointing to `docs/superpowers/specs/2026-06-26-library-maturity-oss-health-design.md` as upcoming work. *Decisions* = pull-only (ex-ADR-0001) and shell-out-to-git+LLM (ex-ADR-0002).
- [ ] **Step 3:** Run procedure Steps D–F.
- [ ] **Step 4:** Commit: `git commit -m "docs(arch): add overview and scaffold docs/architecture"`

---

## Task 2: `01-discovery.md`

**Files:**
- Create: `docs/architecture/01-discovery.md`
- Source: `src/scanner.rs`, `src/worktrees.rs` (reference `src/ignores.rs`, documented in Task 8)
- Absorbs: `docs/decisions/0005-repo-discovery-via-git.md` *(ex-ADR-0005)*, git half of `0002` *(ex-ADR-0002)*, `docs/superpowers/specs/2026-06-23-worktree-inventory-design.md`, `docs/superpowers/specs/2026-06-25-scanner-bare-worktree-discovery.md`, `docs/superpowers/plans/2026-06-23-worktree-inventory.md`, `docs/superpowers/plans/2026-06-25-scanner-bare-worktree-discovery.md`

**Diagram focus:** roots → walk (respecting `scan_depth` + ignores) → detect repos → enumerate branches/worktrees (incl. bare + linked worktrees) via git shell-out.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G with the inputs above.
- [ ] **Note:** *Invariants* must capture the tolerant-shell-out rule (a failing git call on one repo skips that repo with a warning, never aborts the scan) — confirm the exact behavior in `scanner.rs` before writing it.

---

## Task 3: `02-sessions-attribution.md`

**Files:**
- Create: `docs/architecture/02-sessions-attribution.md`
- Source: `src/sessions/mod.rs` (trait `SessionSource`, `SessionExcerpt`), `src/sessions/claude_code.rs`
- Absorbs: `docs/superpowers/specs/2026-06-25-worktree-session-attribution.md`, `docs/superpowers/plans/2026-06-25-worktree-session-attribution.md`

**Diagram focus:** session files → parse (tolerant: bad line = skip + warn) → `SessionExcerpt` → attribute excerpt to a worktree/branch.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** *Interfaces & contracts* must show the full `SessionSource` trait signature and `SessionExcerpt` fields. *Extension* must state the Phase-3 plan for additional harnesses (per `CLAUDE.md`: never couple session format outside `src/sessions/`).

---

## Task 4: `03-query-engine.md`

**Files:**
- Create: `docs/architecture/03-query-engine.md`
- Source: `src/query.rs`
- Absorbs: `docs/decisions/0003-query-engine.md` *(ex-ADR-0003)*, `docs/superpowers/plans/2026-06-24-query-engine-phase1.md`, `docs/superpowers/plans/2026-06-28-query-engine-phase4-contexts.md`

**Diagram focus:** query string → parse → `ScanPlan` → in-memory evaluation against discovered inventory.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** *Interfaces* must show the `ScanPlan` shape, the attribute names/grammar accepted by the parser, and `QueryError`. *Decisions* = why parse→plan→eval in memory rather than a query DB (ex-ADR-0003).

---

## Task 5: `04-inventory-evidence.md`

**Files:**
- Create: `docs/architecture/04-inventory-evidence.md`
- Source: `src/inventory.rs` (reference `src/output.rs`, documented in Task 9)
- Absorbs: `docs/decisions/0004-fase2-evidence-snapshot.md` *(ex-ADR-0004)*, `docs/superpowers/plans/2026-06-26-inventory-cache-phase3.md` (inventory portion)

**Diagram focus:** discovered repos+branches → compute ahead/behind + idle-for → assemble evidence snapshot → inventory rows.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** Define *evidence snapshot* precisely (what fields, computed when). Cross-link the render details to `08-cli-output.md`.

---

## Task 6: `05-resume-distill.md`

**Files:**
- Create: `docs/architecture/05-resume-distill.md`
- Source: `src/distill.rs`
- Absorbs: LLM half of `docs/decisions/0002-git-e-llm-via-shell-out.md` *(ex-ADR-0002)*

**Diagram focus:** gather evidence + session excerpts → build prompt → pipe to configurable `llm_command` (stdin→stdout) → resume sections; `--dry-run` short-circuit.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** *Invariants/edge cases* must cover: default `llm_command = "claude -p"`, the `cat`/`sed` test substitution pattern, and `--dry-run` behavior. *Decisions* = why shell-out to an arbitrary command vs an embedded SDK (ex-ADR-0002).

---

## Task 7: `06-cache-index.md`

**Files:**
- Create: `docs/architecture/06-cache-index.md`
- Source: `src/cache.rs`, `src/index/mod.rs`
- Absorbs: `docs/decisions/0008-sqlite-index.md` *(ex-ADR-0008)*, `docs/superpowers/plans/2026-06-29-sqlite-index-migration.md`, `docs/superpowers/plans/2026-06-26-inventory-cache-phase3.md` (cache portion)

**Diagram focus:** cache key `branch@head-sha` → hit/miss → SQLite index read/write → invalidation when head sha moves.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** *Decisions* must explain the move to a SQLite index (ex-ADR-0008) and what it replaced. State where the DB file lives (`~/.open-loops/`).

---

## Task 8: `07-config-state.md`

**Files:**
- Create: `docs/architecture/07-config-state.md`
- Source: `src/config.rs`, `src/state.rs`, `src/ignores.rs`
- Absorbs: no dedicated ADR — capture config/state/ignores rules as currently implemented.

**Diagram focus:** `loops init` → `~/.open-loops/config.toml` (+ `OPEN_LOOPS_HOME` override) → loaded config feeds discovery (`roots`, `scan_depth`, `ignores`) and resume (`llm_command`).

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** *Interfaces* enumerates every config key with type/default — cross-link to `docs/configuration.md` as the user-facing reference (do not duplicate the table; summarize + link). Document `ignores.rs` here; `01-discovery.md` references it.

---

## Task 9: `08-cli-output.md`

**Files:**
- Create: `docs/architecture/08-cli-output.md`
- Source: `src/cli.rs`, `src/cli_command.rs`, `src/main.rs`, `src/output.rs`
- Absorbs: no dedicated ADR — orchestration + render structure as implemented.

**Diagram focus:** `main` → parse args → dispatch subcommand (`loops`, `resume`, `worktrees`, `completions`, `init`) → orchestrate domains → `output.rs` render.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G.
- [ ] **Note:** Per `CLAUDE.md`, `cli.rs` is a thin orchestration layer covered by `tests/cli.rs` — say so and link the test. Document `output.rs` here as the shared render layer; `04-inventory-evidence.md` references it.

---

## Task 10: `09-build-ci-release.md`

**Files:**
- Create: `docs/architecture/09-build-ci-release.md`
- Source: `.github/workflows/` (`ci.yml`, `release-plz.yml`, `release.yml`), `rust-toolchain.toml`, `Cargo.toml`, `justfile`
- Absorbs: `docs/decisions/0006-ci-msrv-cross-os.md` *(ex-ADR-0006)*, `docs/decisions/0007-release-plz-cargo-dist-split.md` *(ex-ADR-0007)*, `docs/superpowers/specs/2026-06-25-ci-hardening-design.md`, `docs/superpowers/specs/2026-06-26-release-completeness-automation-design.md`, `docs/superpowers/plans/2026-06-26-ci-hardening-wave1.md`, `docs/superpowers/plans/2026-06-26-release-completeness-wave2-3.md`

**Diagram focus:** push/PR → CI matrix (ubuntu/macos/windows) + dedicated MSRV 1.89 job; merge Release PR → release-plz publishes crates.io + tag → `release.yml` (cargo-dist) → binaries + Homebrew tap + GitHub Release.

- [ ] **Step 1:** Run **Per-domain-doc procedure** A–G (Step A reads workflows/toml instead of `src/`).
- [ ] **Note:** Cross-link `docs/distribution.md` (user-facing) and the release checklist in `CLAUDE.md`. *Decisions* = cross-OS+MSRV matrix (ex-ADR-0006) and the release-plz / cargo-dist split (ex-ADR-0007).

---

## Task 11: `README.md` index + repo references

**Files:**
- Create: `docs/architecture/README.md`
- Modify (if they reference the old layout): `CLAUDE.md`, root `README.md`

**Interfaces — Consumes:** the filenames/titles of Tasks 1–10.

- [ ] **Step 1:** Write `docs/architecture/README.md`: one-paragraph purpose, a "how to navigate" note (read `00-overview` first), and a linked table of contents listing `00`–`09` with a one-line description each.
- [ ] **Step 2:** Grep the repo for references to the deleted dirs: `grep -rn "docs/decisions\|docs/superpowers/specs\|docs/superpowers/plans\|docs/audit" --include=*.md --include=*.rs .` Update `CLAUDE.md` (the "Estrutura" / spec pointers) and root `README.md` to point at `docs/architecture/` where appropriate. Leave the historical `library-maturity-oss-health` pointer intact.
- [ ] **Step 3:** Verify every link in `README.md` resolves to an existing file (`00`–`09` all created).
- [ ] **Step 4:** Commit: `git commit -m "docs(arch): add index and repoint references to docs/architecture"`

---

## Task 12: Delete consolidated sources & final verification

Runs only after Tasks 1–11 are committed and reviewed. Deletes the now-absorbed sources, including this plan and its spec (scaffolding).

**Files:**
- Delete dir: `docs/decisions/`
- Delete dir: `docs/audit/`
- Delete dir: `docs/superpowers/plans/`
- Delete (selectively): everything in `docs/superpowers/specs/` **except** `2026-06-26-library-maturity-oss-health-design.md`

- [ ] **Step 1 — DoD gate (before deleting).** Confirm each ADR 0001–0008 has its context/decision/trade-off traceable in some doc's *Decisions* section. Grep: `grep -rn "ex-ADR-000[1-8]" docs/architecture/` — expect all eight numbers present. If any missing, STOP and add it to the owning doc.
- [ ] **Step 2 — Verify the keeper.** Confirm `docs/superpowers/specs/2026-06-26-library-maturity-oss-health-design.md` exists and is referenced from `00-overview.md` (`grep -n "library-maturity" docs/architecture/00-overview.md`).
- [ ] **Step 3 — Delete.**

```bash
git rm -r docs/decisions docs/audit docs/superpowers/plans
git rm $(git ls-files 'docs/superpowers/specs/*' | grep -v 'library-maturity-oss-health')
```

- [ ] **Step 4 — Verify final tree.**

Run: `find docs/superpowers docs/architecture docs/decisions docs/audit -type f 2>/dev/null | sort`
Expected: `docs/architecture/` has 11 files (`README`, `00`–`09`); `docs/superpowers/specs/` has exactly one file (`library-maturity-oss-health`); `docs/superpowers/plans/`, `docs/decisions/`, `docs/audit/` gone.

- [ ] **Step 5 — Dangling-link sweep.**

Run: `grep -rn "docs/decisions\|docs/superpowers/plans\|docs/audit" --include=*.md --include=*.rs . | grep -v 'docs/architecture/'`
Expected: no output (every reference to a deleted path was repointed in Task 11).

- [ ] **Step 6 — Commit.** `git commit -m "docs: remove ADRs/plans/specs/audit consolidated into docs/architecture"`

---

## Self-Review

**Spec coverage:**
- 8 runtime domains + overview + 9th build/CI/release doc + README → Tasks 1–11. ✓
- Total consolidation (ADRs absorbed, dirs deleted) → Task 12. ✓
- English-first → Global Constraints + Step F in every doc task. ✓
- Template (9 sections) + Mermaid → Global Constraints + per-doc procedure. ✓
- `library-maturity-oss-health` preserved + roadmapped → Task 1 (roadmap) + Task 12 Step 2 (keeper). ✓
- `docs/audit/` deleted → Task 12. ✓
- ADR-0002 split across overview/discovery/distill → Tasks 1, 2, 6 all cite ex-ADR-0002. ✓ (overview = principle, discovery = git half, distill = LLM half.)

**Coverage of every absorbed source:** ADR 0001(T1) 0002(T1/2/6) 0003(T4) 0004(T5) 0005(T2) 0006(T10) 0007(T10) 0008(T7); specs mvp(T1) worktree-inventory(T2) scanner-bare(T2) worktree-session(T3) ci-hardening(T10) release-completeness(T10) library-maturity(kept); plans all mapped across T1–T10. No orphan source.

**Placeholder scan:** no TBD/TODO; each task lists exact files + absorbed sources + diagram focus. Doc *content* is authored at execution (prose), not pre-written here — by design for a documentation deliverable.

**Type consistency:** glossary terms fixed in Task 1 (*open loop*, *evidence snapshot*, *resume context*, *pull-only*) and reused verbatim; `ScanPlan`/`SessionSource`/`SessionExcerpt`/`QueryError` named consistently across Tasks 3/2/4.

## Execution note (Ultracode)

Tasks 2–10 are mutually independent (each reads only its own source files + sources and writes one new file). Task 1 first (fixes shared vocabulary), then 2–10 can fan out one agent per doc via a Workflow, then Task 11 (index, needs all filenames), then Task 12 (cleanup + DoD gate). Tasks 11–12 are sequential and gated on review.
