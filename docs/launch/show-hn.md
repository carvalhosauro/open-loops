# Launch drafts — open-loops

Copy for launching `open-loops`. Nothing here is posted automatically — review,
tweak the voice to yours, and post when ready. Replace `<GIF-URL>` with the demo
GIF once it's hosted (see the checklist).

---

## Show HN

**Title** (keep under ~80 chars, no "Show HN:" is added by the form):

```
Show HN: open-loops – recover the context of paused git branches from your AI sessions
```

**Body:**

> I kept losing the thread on half-finished branches. Not the code — the *context*:
> why I started it, what was done, what was left, what the next step was. That
> context already exists, scattered across git history and my AI-coding session
> logs; the friction is the manual archaeology of digging it back out.
>
> open-loops (`loops`) is a small Rust CLI that does the digging. It lists every
> unmerged branch across all your repos (most-idle first, no LLM, <5s), and on
> demand it reconstructs a resume doc for one of them — Why / Done / Remaining /
> Next step — by distilling the branch's git history and the AI sessions that
> touched it.
>
> Design choices that mattered to me:
> - **Pull-only and local-first.** It captures nothing ahead of time and never
>   writes inside your repos. It reads git and your session logs on demand, so it
>   works retroactively on branches you already have. The only network call is the
>   LLM command *you* configure (default `claude -p`, but any stdin→stdout command
>   works — swap in ollama, etc.).
> - **Every reconstruction ships a confidence score and a `## Sources` section**,
>   because attribution is heuristic and you should be able to audit it before you
>   trust it. `--dry-run` shows the evidence without calling the LLM.
> - **git is the source of truth.** All caches are disposable and rebuild on the
>   next run.
>
> It's Rust, MIT/Apache-2.0, on crates.io (`cargo install open-loops`), with a
> Homebrew tap and prebuilt binaries.
>
> Repo: https://github.com/carvalhosauro/open-loops
>
> Honest limitation: today it reads Claude Code sessions. The session layer is a
> trait (`SessionSource`) so adapters for Codex CLI / OpenCode / others are the
> obvious next step — happy to take pointers or PRs on which harness to add first.

**Tips**
- Post Tue–Thu, ~8–10am US Eastern for the widest window.
- Be in the thread for the first 2–3 hours to answer questions — engagement
  early is what moves it.
- Lead a first comment with the "why I built it" story if it isn't in the body.

---

## r/rust (and r/commandline)

**Title:**

```
open-loops: a pull-only Rust CLI that reconstructs the context of paused branches from git + your AI sessions
```

**Body:**

> Sharing a weekend-grown tool that turned into something I use daily. `loops`
> answers "what did I start and not finish, and where did I leave off?" across all
> my repos.
>
> It lists unmerged branches (fast, no LLM), and `loops resume <branch>` distills a
> Why/Done/Remaining/Next-step doc from that branch's commits and the AI-coding
> sessions that touched it. Pull-only and local — nothing is written inside your
> repos, and the only network call is the LLM command you configure.
>
> Rust-specific notes for this crowd: single lib+bin crate, typed domain errors
> (`thiserror`, no `anyhow` in the library API), `tracing` behind `--verbose`,
> proptest on the query engine, CI across Linux/macOS/Windows, MSRV 1.89.
>
> `cargo install open-loops` · MIT/Apache-2.0 · https://github.com/carvalhosauro/open-loops
>
> Feedback on the query language and on which AI harness to support next (the
> session source is a trait) very welcome.

*(For r/commandline, drop the Rust-internals paragraph and lead with the demo GIF.)*

---

## This Week in Rust

Submit a PR to https://github.com/rust-lang/this-week-in-rust adding a line under
"Crate of the Week" nominations or the project-updates section:

```
- [open-loops](https://github.com/carvalhosauro/open-loops) — a pull-only CLI that
  reconstructs the context of paused git branches from git history and your AI
  coding sessions.
```

---

## Short blurbs (X / Bluesky / Mastodon)

> Lost the thread on a half-finished branch? `open-loops` reconstructs *why you
> started it and what's left* from your git history + AI-coding sessions. Pull-only,
> local, Rust. `cargo install open-loops` 🧵 <GIF-URL>

> `loops` = the answer to "what did I start and not finish?" across all your repos.
> No notes, no tickets — it reads git and your AI sessions on demand. <repo-url>

---

## awesome-list PRs

Open a small PR adding one bullet to each:

- **awesome-rust** → *Applications / Utilities*
- **awesome-cli-apps** → *Git*
- **awesome-command-line-apps**

Suggested bullet:

```
- [open-loops](https://github.com/carvalhosauro/open-loops) - Reconstructs the
  context of paused git branches from git history and AI coding sessions
  (pull-only, local).
```

---

## Pre-launch checklist

- [ ] Demo GIF generated and hosted. From the repo:
      `agg docs/demo.cast docs/demo.gif` (install: `cargo install --git
      https://github.com/asciinema/agg`), commit it, and reference it at the top of
      the README so social posts can point at a raw GitHub URL.
- [ ] README hero reads well cold (someone who's never heard of it gets the pitch
      in <30s).
- [ ] `cargo install open-loops` works from a clean machine (verify the published
      version, not just local).
- [ ] A few `good first issue`s are open so drive-by interest can convert to PRs.
- [ ] You have ~2–3 hours free after posting to answer questions.
