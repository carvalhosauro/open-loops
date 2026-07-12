# Security Policy

## Scope

`loops` is a local command-line tool. It has **no server, no network service, and
no daemon**: it reads your local git repositories and AI-session files, shells out
to the `git` binary, and invokes a configurable local LLM command. There is no
remote attack surface exposed by the tool itself.

Relevant trust boundaries:

- It executes the configured `llm_command` (default `claude -p`) as a subprocess
  via `sh -c`, passing the prompt on stdin.
- It shells out to `git`.
- It reads and writes state under `~/.open-loops/` (or `$OPEN_LOOPS_HOME`).

## Supported versions

Only the latest published release on [crates.io](https://crates.io/crates/open-loops)
receives security fixes.

## Reporting a vulnerability

Please report security issues **privately** — do not open a public GitHub issue.

- **Preferred:** open a private
  [GitHub Security Advisory](https://github.com/carvalhosauro/open-loops/security/advisories/new)
  on this repository.
- **Alternative:** email the maintainer at `gustavo.carvalho@pigz.com.br` with
  `[open-loops security]` in the subject.

Please include a description, reproduction steps, and the impact you observed. We
aim to acknowledge a report within a few days and will coordinate a fix and
disclosure timeline with you (responsible disclosure — please give us a
reasonable window before any public write-up).

## What is not a vulnerability

- **AI-session content on your own disk.** `loops` reads the session transcripts
  your AI harness already wrote under your home directory. That those files exist,
  and that a distilled summary may quote them, is by design — it is your own data
  on your own machine, not an exposure created by this tool.
- **The LLM command running arbitrary commands.** `llm_command` is
  user-configured and run as a local subprocess by design (it is how you plug in
  your LLM provider). Configuring a malicious command is equivalent to running it
  yourself.
- **Reconstructed context being wrong or hallucinated.** Attribution is heuristic
  and the `## Sources` section exists so you can audit it; inaccuracy is a quality
  issue, not a security vulnerability.
