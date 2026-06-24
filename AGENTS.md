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

## Estrutura

- `src/scanner.rs` — repos e branches via shell-out ao git
- `src/sessions/` — trait SessionSource + adapter claude_code (NUNCA acople
  formato de sessão fora deste diretório; Fase 3 adiciona outros harnesses)
- `src/distill.rs` — prompt + LLM via comando configurável (testes usam `cat`)
- `src/cache.rs` — cache por `branch@head-sha`
- `src/config.rs`, `src/ignores.rs` — estado em `~/.open-loops/`
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
git tag v0.1.0 && git push --tags   # CI builda binários + installers + release notes
```
