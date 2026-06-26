# open-loops — mapa para agentes

CLI Rust. Binário: `loops`. Spec: `docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md`.

## Comandos de desenvolvimento

```bash
just setup   # instala hooks (lefthook)
just test    # cargo test
just lint    # clippy -D warnings
just fmt     # cargo fmt
just cov     # cobertura, gate 70% (core: alvo 85%)
```

CI runs on ubuntu/macos/windows; MSRV 1.89 enforced by dedicated `msrv` job.

## Estrutura

- `src/scanner.rs` — repos e branches via shell-out ao git
- `src/sessions/` — trait SessionSource + adapter claude_code (NUNCA acople
  formato de sessão fora deste diretório; Fase 3 adiciona outros harnesses)
- `src/distill.rs` — prompt + LLM via comando configurável (testes usam `cat`)
- `src/cache.rs` — cache por `branch@head-sha`
- `src/config.rs`, `src/ignores.rs` — estado em `~/.open-loops/`
- `src/query.rs` — parser de query → `ScanPlan` + avaliação em memória (ADR 0003)
- `src/cli.rs` — orquestração; camada fina, coberta por `tests/cli.rs`

## Convenções

- Conventional Commits (hook valida); mensagens de erro em EN, acionáveis
- Parsing de sessão é tolerante: linha ruim = skip + warning, nunca abort
- Testes criam repos git reais em tempdir (`src/testutil.rs`)
- Docs fazem parte do Definition of Done (checklist do PR)
- Fonte de verdade dos comandos/config: `docs/features.md` e `docs/configuration.md`

## Release

```bash
just changelog                      # atualiza CHANGELOG.md
git add CHANGELOG.md && git commit -m "docs: update changelog"
git tag v0.1.0 && git push --tags   # CI: binários, Homebrew tap, crates.io
```

Secrets (one-time): `CARGO_REGISTRY_TOKEN`, `HOMEBREW_TAP_TOKEN`. Tap repo:
`carvalhosauro/homebrew-tap`. Checklist: `docs/distribution.md`.

## Cursor Cloud specific instructions

- Toolchain Rust 1.89 é selecionado automaticamente via `rust-toolchain.toml`; só `rustc`/`cargo` + `git` são requisitos obrigatórios. Build/lint/test/run usam `cargo` diretamente (ver `justfile`).
- `just`, `lefthook`, `cargo-llvm-cov` e `git-cliff` NÃO ficam instalados por padrão. Rode os comandos do `justfile` via `cargo` (ex.: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`) ou instale as ferramentas sob demanda.
- A suíte `tests/cli.rs` cria repositórios git reais em tempdir e leva ~30s; tudo passa offline, sem serviços externos.
- `loops resume` (sem `--dry-run`) executa `llm_command` (default `claude -p`, que não existe aqui). Para exercitar o pipeline de distill sem um LLM real, aponte `llm_command` no config para um comando stdin→stdout, ex. `cat` ou `sed 's/^/LLM> /'`. `loops`, `loops resume --dry-run`, `loops worktrees` e `loops completions` funcionam sem LLM.
- Estado fica em `~/.open-loops/` (ou `OPEN_LOOPS_HOME`); use um tempdir como `OPEN_LOOPS_HOME` para experimentos sem poluir o ambiente.
