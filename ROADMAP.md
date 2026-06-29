# Roadmap — open-loops

Track fino das fases pendentes. Ordem = sequência de execução recomendada
(respeita dependências). Marque `- [x]` ao concluir cada item.

## Fontes

| Bloco | Documento |
|---|---|
| Query engine, contexts, reports, inventory | [ADR 0003 — query engine](docs/decisions/0003-query-engine.md) |
| Descoberta bare + worktree | [Spec Fase A — scanner bare+worktree](docs/superpowers/specs/2026-06-25-scanner-bare-worktree-discovery.md) |
| Atribuição de sessão por worktree | [Spec Fase B — worktree session attribution](docs/superpowers/specs/2026-06-25-worktree-session-attribution.md) |
| CI hardening (WAVE 1) | [Spec CI hardening](docs/superpowers/specs/2026-06-25-ci-hardening-design.md) |
| Release artefato + automação (WAVE 2/3) | [Spec release completeness](docs/superpowers/specs/2026-06-26-release-completeness-automation-design.md) |
| Maturidade da lib + saúde OSS (WAVE 4) | [Spec library maturity](docs/superpowers/specs/2026-06-26-library-maturity-oss-health-design.md) |

## Regras de ordenação (travadas)

- **Spec Fase A antes do query engine avançado** — descoberta bare+worktree era pré-requisito
  do scan; concluída (✅).
- **Spec Fase A antes da ADR fase 3** — o common-dir absoluto é a identidade do
  hash do inventory ([ADR 0003 §161](docs/decisions/0003-query-engine.md), Spec A §8).
- **ADR fase 2 antes de 4 e 5** — contexts/reports precisam do push-down pra escopar.
- **Spec Fase B depende da Fase A mergeada.**
- **WAVE 1 (CI) antes de WAVE 2/3 (release)** — release-plz assume CI endurecido.
- **WAVE 2/3 antes de WAVE 4** — badge crates.io e release-plz landados; erros tipados
  e proptest desenhados para a matriz cross-OS do WAVE 1.
- **WAVE 4 é ortogonal às fases ADR 2–6** — pode rodar em paralelo depois dos pré-reqs.

## Sequência — query engine

### ✅ Fase 1a — parser de query → ScanPlan  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

- [x] `query.rs`: parse de termos soltos, atributos (`repo`/`branch`/`key`/`root`/`idle`/`ahead`/`behind`), tags (`+ignored`/`-ignored`)
- [x] comparadores (`>` `<` `>=` `<=`) + durações (m/h/d/w)
- [x] `ScanPlan` + avaliação em memória (`matches`)
- [x] contexts/reports/`+stale` reservados com erro acionável

### ✅ Fase 1b — chave canônica de 3 segmentos  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

- [x] `OpenLoop.root_label` + `key()` `root_label/repo/branch`
- [x] `Cache::path` chaveado por `root_label`
- [x] `resolve_loop` casa contra chave de 3 segmentos; `resume` inclui ignorados
- [x] aliases por root + erro de colisão de label
- [x] superfície clap `loops [query]`
- [x] migração documentada (`docs/configuration.md`, CHANGELOG)

### ✅ Spec Fase A — descoberta bare + worktree  ·  [Spec A](docs/superpowers/specs/2026-06-25-scanner-bare-worktree-discovery.md)

Depende de: —

- [x] `walk`: detectar `.git` arquivo **ou** diretório (`exists()` no lugar de `is_dir()`)
- [x] `walk`: detectar bare por probe estrutural (`HEAD` + `objects/` + `refs/`)
- [x] `git_common_dir(path)` via `rev-parse --path-format=absolute --git-common-dir`
- [x] dedup por common-dir em `find_repos`/`scan` (N worktrees → 1 repo)
- [x] `repo_name_from_common_dir` (função pura, regra `.git`/`.bare`/`foo.git`)
- [x] `scan_depth: usize` em `Config` (default 4), substitui `MAX_DEPTH` fixo
- [x] `cli.rs` passa `cfg.scan_depth` adiante
- [x] ADR 0005 — descoberta por interrogação ao git
- [x] `docs/features.md` + `docs/configuration.md`: descoberta e `scan_depth`
- [x] `testutil`: helpers `init_bare_repo` / `add_worktree`
- [x] testes: normal, ponteiro `.git`, bare puro, dedup, tabela de nomes, `open_loops` em bare, `scan_depth`, `tests/cli.rs`
- [x] validação manual: `loops` lista branches em `~/repo/pigz`
- [x] `just lint` + `just fmt` limpos; cobertura no gate
- [x] CHANGELOG atualizado

### ✅ Spec Fase B — atribuição de sessão por worktree  ·  [Spec B](docs/superpowers/specs/2026-06-25-worktree-session-attribution.md)

Depende de: **Spec Fase A**

- [x] `worktree_map(repo)` parseia `git worktree list --porcelain` (helper puro testado)
- [x] `open_loops` resolve `repo_path` por branch (worktree se checada out, senão fallback container)
- [x] falha do `worktree list` → mapa vazio + warning (degrada, nunca aborta)
- [x] `sessions/claude_code.rs`: confirmar que `excerpts` só assume o cwd encodado
- [x] testes: parse porcelain, `open_loops` worktree vs sem-worktree, integração de sessão, regressão repo normal
- [x] validação manual: `loops resume <branch-em-worktree>` traz excerpts em `~/repo/pigz`
- [x] `docs/features.md`: casamento de sessão por worktree; ADR 0005 atualizado se necessário
- [x] `just lint` + `just fmt`; cobertura no gate
- [x] CHANGELOG atualizado

### ✅ ADR fase 2 — push-down + split fase leve/pesada  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **Spec Fase A** (scan que acha repos + `repo_name` final). `@` contexts ficam na fase 4.

- [x] wire do `ScanPlan` no scan
- [x] push-down de roots (subset de `cfg.roots` via `root:`; `@` na fase 4)
- [x] filtra repos por `repo_filter` **antes** de qualquer git
- [x] split fase leve (sempre roda) / fase pesada (`rev-list` sob demanda)
- [x] `need_ahead_behind` = renderiza colunas AHEAD/BEHIND **ou** query tem attr `ahead`/`behind`
- [x] `ahead`/`behind` ficam `None` quando a fase pesada não roda; `render_table` imprime `-`

### ✅ ADR fase 3 — inventory cache + refresh  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **Spec Fase A** (common-dir = identidade do hash), **ADR fase 2**

- [x] `inventory.rs`: arquivo por repo em `~/.open-loops/inventory/<hash-common-dir>.json`
- [x] memo de ahead/behind validado por `(head_sha, ab_base_sha)`
- [x] write-through em todo scan (inclusive `loops api`)
- [x] escrita atômica (tmp + rename)
- [x] `--fresh` ignora o memo; `loops refresh [query]` full reindex
- [x] `inventory_ttl_secs` no config (default 0 = só validação por SHA)
- [x] limpeza preguiçosa de órfãos no `refresh` ([ADR 0004](docs/decisions/0004-fase2-evidence-snapshot.md))

### ✅ ADR fase 4 — contexts `@`  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **ADR fase 2** (push-down)

- [x] parse de `@nome` resolve `[contexts.nome]` do config
- [x] `[contexts.X] filter = "..."` no `config.toml`
- [x] `@none` / `@all` limpam o contexto em `state.toml`
- [x] contexto ativo em `state.toml`; `@ctx` na CLI grava e filtra
- [x] remover erro "contexts not supported yet" do parser

### ✅ SQLite index — cache descartável de scan + sessões  ·  [ADR 0008](docs/decisions/0008-sqlite-index.md)

Depende de: **Spec Fase A** (common-dir = identidade), **ADR fase 3** (inventory).
git permanece fonte da verdade; o índice é descartável e auto-recuperável.

- [x] dependência `rusqlite` (feature `bundled`, sem libsqlite3 do sistema; MSRV 1.89)
- [x] `src/index/mod.rs`: `Index` em `<base>/index.db` (WAL), schema `user_version=1`, open tolerante (corrupção → rebuild → fallback in-memory)
- [x] cache de `--git-common-dir` no dedup (resolve #17)
- [x] gate por refs-fingerprint (mtime nanos de HEAD/packed-refs/refs/worktrees + `default_sha`) pula `rev-list`/`for-each-ref` em scan quente (resolve #13)
- [x] FTS de sessões: probe de menção em tail limitado, sem ler arquivo inteiro (resolve #14)
- [x] ranking estável de sessões; vazias filtradas antes do `max_sessions` (resolve #15)
- [x] wire ao vivo no `cli.rs` (`scan_indexed` + `excerpts_indexed`); JSON inventory preservado
- [x] `loops refresh` reconstrói o índice e `Index::prune_missing_repos` remove repos sumidos do disco
- [x] `regress.sh` verde; e2e manual + self-heal (corromper/apagar `index.db`)
- [x] ADR 0008; `features.md` + `configuration.md` + CHANGELOG atualizados
- [ ] #16 (estratégia de fan-out de threads) e #31 (coluna de path no `loops`) seguem fora de escopo

### ⬜ ADR fase 5 — reports `:` + `+stale` + help  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **ADR fase 2**, **ADR fase 4**

- [ ] parse de `:nome` expande `[reports.nome]` como sub-query (guard de profundidade = 1)
- [ ] `[reports.X] filter = "..."` no `config.toml`
- [ ] `+stale` = `idle:>{stale_threshold}`; `stale_threshold` no config (default 14d)
- [ ] `loops help query`
- [ ] remover erros "not supported yet" (`:report` e `+stale`)

### ⬜ ADR fase 6 — filtro em `worktrees [query]`  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **Spec Fase A**, **ADR fase 2** (camada de filtro), reuso da coleta `--porcelain` (Spec Fase B)

- [ ] `loops worktrees [query]` usa a mesma camada parse → ScanPlan → filtro
- [ ] reusa o parse de `worktree list --porcelain` (helper compartilhado com a Spec Fase B)
- [ ] `resume` já é engine-based desde a fase 1b — sem trabalho extra

## Sequência — infra & maturidade

Trilha paralela ao query engine. Ordem entre waves travada em §Regras de ordenação.

### ✅ WAVE 1 — CI hardening  ·  [Spec](docs/superpowers/specs/2026-06-25-ci-hardening-design.md)

Depende de: —

- [x] `ci.yml`: matriz ubuntu + macos + windows, `fail-fast: false`
- [x] job `msrv` em 1.89; `--locked` em clippy/test/msrv/coverage
- [x] `Swatinem/rust-cache` nos jobs que compilam
- [x] `concurrency` + env global (`RUSTFLAGS: "-D warnings"`, etc.)
- [x] `deny.toml` + job `audit` (cargo-deny, matriz advisories vs licenses)
- [x] `.github/dependabot.yml` (cargo + github-actions)
- [x] SHA-pin em todos os `uses:`
- [x] `test (windows-latest)` verde (corrigir falhas de path/CRLF se expostas)
- [x] README badges (CI, crates.io, MSRV, license)
- [x] ADR `0006-ci-msrv-cross-os.md`

### ✅ WAVE 2/3 — artefato de release + automação  ·  [Spec](docs/superpowers/specs/2026-06-26-release-completeness-automation-design.md)

Depende de: **WAVE 1**

- [x] `build.rs`: completions (4 shells) + man page (`clap_mangen`)
- [x] `dist-workspace.toml`: empacota artefatos gerados + LICENSE/README/CHANGELOG no tarball
- [x] `Cargo.toml`: metadados crates.io (`rust-version`, `categories`, …) + `[profile.release]`
- [x] `release-plz.toml` + `.github/workflows/release-plz.yml` (PAT `RELEASE_PLZ_TOKEN`)
- [x] deletar `publish-crate.yml` (crates.io via release-plz)
- [ ] release patch ponta-a-ponta: merge Release PR → tag → `release.yml` (infra pronta; validar após configurar `RELEASE_PLZ_TOKEN` e merge do primeiro Release PR)
- [x] ADR `0007-release-plz-cargo-dist-split.md`

### ⬜ WAVE 4 — maturidade da lib + saúde OSS  ·  [Spec](docs/superpowers/specs/2026-06-26-library-maturity-oss-health-design.md)

Depende de: **WAVE 1**, **WAVE 2/3**

#### 4.1 — Error typing (`thiserror`)
- [ ] `src/error.rs` + migração completa (PRs 4.1a→d); `anyhow` só na borda CLI
- [ ] testes com `matches!` em vez de string-matching de stderr

#### 4.2 — Unit tests + proptest
- [ ] proptest em `query.rs` (≥ 3 propriedades)
- [ ] `build_prompt` com múltiplos excerpts; gate 85% no core

#### 4.3 — Observabilidade
- [ ] `tracing` na lib; `--verbose` / `RUST_LOG`; ADR `0009-typed-errors-tracing.md`

#### 4.4 — Community health
- [ ] `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`
- [ ] README: badges + link contributing (se WAVE 1 ainda não cobriu badges)
