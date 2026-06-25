# Changelog
## unreleased

### Docs
- Rewrite README with progress lines and audit flow
- Add ADR 0003 for query engine and inventory cache

## v0.1.0 - 2026-06-24

### Docs
- Add open-loops MVP design spec
- Add DX section to spec (pre-commit, coverage gates, justfile)
- Add open-source posture section to spec
- Add documentation requirements section to spec
- Add phase 3 (multi-harness support) to spec roadmap
- Add MVP implementation plan
- Add execution status snapshot for session resume
- Correct status — task 6 implemented, reviews pending
- Add readme, agent map, user docs, adrs and github templates
- Document release flow
- Spec for worktree inventory, EN-first CLI, completions
- Implementation plan for worktree inventory + EN migration
- English test names in worktree plan
- English/minimal comments rule and quality flow tests in plan
- English docs, worktrees + completions, error-language rule

### Features
- Add config store with toml persistence
- Add persistent ignore list for dead loops
- Add git shell-out helper and default branch detection
- Discover repos and unmerged branches with context helpers
- Add session source trait and claude code adapter
- Add distillation cache keyed by branch and head sha
- Build evidence prompt and run configurable llm command
- Render inventory table sorted by staleness
- Wire list, init, ignore and resume commands
- Add completions command for shell autocomplete
- Add Worktree model and deterministic verdict
- Enumerate and classify repo worktrees
- Render worktree inventory table with cleanup commands
- Add worktrees command (alias wt)
- Add resume dry-run, confidence score, and v0.1.0 release prep

### Fixes
- Update repository links in documentation to reflect new ownership
- Address quality review (no expect, no toctou, docs)
- Strict origin prefix strip and stronger error assertion
- Warn on malformed for-each-ref lines instead of silently dropping
- Tolerate broken pipe when llm exits before reading stdin
- Exclude default branch from merged set

### Internals
- Add rust-best-practices and rust-testing skills
- Scaffold rust crate with dual MIT/Apache-2.0 license
- Add justfile, lefthook hooks and ci with 70% coverage gate
- Add e2e flow covering init, list, resume, cache and ignore
- Add cargo-dist release pipeline and git-cliff changelog
- Add hygiene review report and mark mvp status complete
- Add 10 coverage gap tests and audit report
- Add worktree helper to testutil
- Migrate CLI output, errors, comments, and test names to English
- English test strings in claude_code session tests
- Produce resume output in English
- Add quality-focused worktree and completion flow tests
- English comment in session tail reader
- English test names and comments in tests/cli.rs
