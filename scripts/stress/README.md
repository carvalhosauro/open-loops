# open-loops stress + regression harness

A committed, **deterministic, reproducible** replacement for the previous
ad-hoc stress testing (throwaway tempdirs that were deleted, so nothing could be
re-run or compared). Everything here is checked in, isolated, and re-runnable.

Two black-box tools driving the release binary (`target/release/loops`):

- **`bench.sh`** — performance: generate fixtures at a chosen scale, time the
  relevant `loops` command, print `scenario | command | wall_s | max_rss_mb`.
- **`regress.sh`** — behavior: assert the observable contract of every
  subcommand; exits nonzero if any assertion fails (CI-able).

Both build fixtures with `gen_fixtures.sh` and share helpers in `lib.sh`.

## Determinism contract

The whole point is repeatability, so the harness pins everything that would
otherwise drift:

- **Fixed identity + clock.** `lib.sh` exports `GIT_AUTHOR_*`/`GIT_COMMITTER_*`
  and a constant `GIT_*_DATE` (baseline `2026-01-01T00:00:00Z`, offset per
  commit by an integer index for stable ordering). No `$RANDOM`, no
  clock-derived fixture content; all counts derive only from the scale param.
- **Idle grows, structure does not.** `loops` computes `IDLE` against the
  *current* wall clock, so absolute idle numbers increase over time. Assertions
  therefore check **structure** (ordering, columns, presence/absence), never an
  absolute idle-day count. `idle:` filters are tested with always-true (`>1d`)
  and always-false (`>9999w`) thresholds.
- **Full isolation.** Every case runs under a fresh `OPEN_LOOPS_HOME=$(mktemp -d)`
  and its own sessions dir. The harness never touches real `~/.open-loops` or
  `~/.claude`.

## How to run

```bash
# behavior regression (fast, ~10s)
just regress
# or directly:
bash scripts/stress/regress.sh
bash scripts/stress/regress.sh --scale 20      # bigger fixtures, same assertions

# performance benchmark
just stress                                     # modest scales, ~1-2 min
just stress --heavy                             # the bottleneck-exposing scales
bash scripts/stress/bench.sh --out bench.txt    # also write the table to a file
bash scripts/stress/bench.sh --keep             # keep generated fixtures

# point at a different binary
LOOPS_BIN=/path/to/loops bash scripts/stress/regress.sh
```

`LOOPS_BIN` defaults to `<repo>/target/release/loops` and errors early if it is
missing or not executable.

## Scenario catalog

Each scenario is a deterministic builder in `gen_fixtures.sh`
(`gen_fixtures.sh <scenario> <outdir> [scale]`, prints the generated root path):

| Scenario | What it probes | Code path exercised |
|---|---|---|
| `many-repos` | scale independent repos, each with one unmerged branch | `scanner::find_repos` walk + dedup by `--git-common-dir`, parallel `open_loops` (`branch --merged`, `for-each-ref`) |
| `many-branches` | one repo with scale unmerged branches | per-branch `rev-list --left-right --count` (the historical hot loop) |
| `many-worktrees` | one repo with scale worktrees, mixed verdicts | `worktrees::scan_worktrees` → `worktree list --porcelain` + per-wt `status`/`log` |
| `big-session` | bare+worktree container + an S-MB session at the encoded **worktree** path | `sessions::claude_code` tail read (`read_tail_text`, `max_session_kb` truncation) |
| `wide-tree` | many non-repo dirs + repos at depths 1/4/5 | `scanner::walk` recursion, `scan_depth` boundary, hidden/`SKIP_DIRS` pruning |
| `bare-worktree` | canonical Fase B layout + discriminating sessions | `repo_path` = worktree-when-checked-out (resume session matching) |
| `pathological` | broken/no-commit/bare repos, detached + prunable worktrees, slash + unicode branches | warning-not-abort degradation, verdict classification, branch-name handling |

### The Fase B discriminator (the regression that matters)

`bare-worktree` writes the **same** session text under two encoded paths:

- under the encoded **worktree** path (`…-my-app-probe`) — `resume feat/probe`
  **must** surface it (`SENTINEL_WORKTREE`);
- under the encoded **container** path (`…-my-app`) — it **must not** be
  surfaced (`SENTINEL_CONTAINER`).

This is a real discriminating assertion, not commit-text theater: it passes only
while `repo_path` resolves to the checked-out worktree. A revert of that fix
(falling back to the container/common-dir) flips the container session into the
output and the assertion fails.

## Baseline numbers

Recorded verbatim from the prior heavy run. **Environment: 16-core WSL2, git
2.43, loops 1.1.1.**

| Probe | Result |
|---|---|
| 500 repos | 6.8 s / RSS 12 MB |
| 2000 branches | 10.0 s (rev-list 7.8 s) |
| 150 worktrees (`loops worktrees`) | 2.59 s |
| 300 MB session | 0.9 s / RSS ≈ 296 MB |
| 300 repos dedup | 1.08 s |
| 22k-dir wide tree | 0.46 s |

These are the `--heavy` scales (`~500 repos`, `~2000 branches`, `~150
worktrees`, `~300 MB session`). The default (non-heavy) scales are smaller and
exist for a quick local sanity pass.

## Comparing a new run against the baseline

1. Capture a run: `bash scripts/stress/bench.sh --heavy --out new.txt`.
2. Confirm the header matches the baseline environment (loops version, git
   version, `nproc`). RSS and wall time are only comparable on the same machine
   class — the table header records all three so a stale comparison is obvious.
3. Compare row-by-row against the baseline table above. A meaningful regression
   is a wall-time or RSS figure well outside run-to-run noise (rule of thumb:
   >30% slower, or RSS materially above the session size for `big-session`).
4. For behavior, `regress.sh` is the gate: any nonzero exit is a regression.
   Run it against the suspect binary with `LOOPS_BIN=…`.

The `300 MB session → RSS ≈ 296 MB` row reflects that the session is read into
memory; if a future change streams the tail instead, RSS should drop well below
the file size — a *good* regression worth recording as a new baseline.

## Known bottlenecks

Tracked in GitHub issues (numbers filled in by the controller):

- **Per-branch `rev-list --left-right --count`** dominates `many-branches`
  (~7.8 s of the 10.0 s at 2000 branches): one git invocation per branch. See #TBD.
- **Whole-session read into memory** for `resume`: RSS tracks the session file
  size (300 MB session → ~296 MB RSS) instead of streaming the tail window. See #TBD.

## Files

- `lib.sh` — sourced helpers: strict mode, `LOOPS_BIN` resolution, pinned-env
  `g()` git wrapper, `encode_project_path`, fixed-date commit helper, repo/branch/
  worktree/session builders, config setters, `timed()` (wall + max RSS via
  `/usr/bin/time -v`, graceful fallback to bash `SECONDS`), assert helpers +
  PASS/FAIL counter.
- `gen_fixtures.sh` — the deterministic scenario builders.
- `bench.sh` — the performance harness.
- `regress.sh` — the behavior-regression harness.
