# Roadmap — open-loops

Track fino das fases pendentes. Ordem = sequência de execução recomendada
(respeita dependências). Marque `- [x]` ao concluir cada item.

## Fontes

| Bloco | Documento |
|---|---|
| Query engine, contexts, reports, inventory | [ADR 0003 — query engine](docs/decisions/0003-query-engine.md) |
| Descoberta bare + worktree | [Spec Fase A — scanner bare+worktree](docs/superpowers/specs/2026-06-25-scanner-bare-worktree-discovery.md) |
| Atribuição de sessão por worktree | [Spec Fase B — worktree session attribution](docs/superpowers/specs/2026-06-25-worktree-session-attribution.md) |

## Regras de ordenação (travadas)

- **Spec Fase A antes de tudo** — hoje `find_repos` acha **zero** repos no layout
  bare+worktree do autor; toda fase do query engine é inútil sobre um scan vazio.
- **Spec Fase A antes da ADR fase 3** — o common-dir absoluto é a identidade do
  hash do inventory ([ADR 0003 §161](docs/decisions/0003-query-engine.md), Spec A §8).
- **ADR fase 2 antes de 4 e 5** — contexts/reports precisam do push-down pra escopar.
- **Spec Fase B depende da Fase A mergeada.**

## Sequência

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

**Próximo bloqueador.** Depende de: —

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

### ⬜ Spec Fase B — atribuição de sessão por worktree  ·  [Spec B](docs/superpowers/specs/2026-06-25-worktree-session-attribution.md)

Depende de: **Spec Fase A**

- [ ] `worktree_map(repo)` parseia `git worktree list --porcelain` (helper puro testado)
- [ ] `open_loops` resolve `repo_path` por branch (worktree se checada out, senão fallback container)
- [ ] falha do `worktree list` → mapa vazio + warning (degrada, nunca aborta)
- [ ] `sessions/claude_code.rs`: confirmar que `excerpts` só assume o cwd encodado
- [ ] testes: parse porcelain, `open_loops` worktree vs sem-worktree, integração de sessão, regressão repo normal
- [ ] validação manual: `loops resume <branch-em-worktree>` traz excerpts em `~/repo/pigz`
- [ ] `docs/features.md`: casamento de sessão por worktree; ADR 0005 atualizado se necessário
- [ ] `just lint` + `just fmt`; cobertura no gate
- [ ] CHANGELOG atualizado

### ⬜ ADR fase 2 — push-down + split fase leve/pesada  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **Spec Fase A** (scan que acha repos + `repo_name` final)

- [ ] wire do `ScanPlan` no scan
- [ ] push-down de roots (subset de `cfg.roots` via `@`/`root:`)
- [ ] filtra repos por `repo_filter` **antes** de qualquer git
- [ ] split fase leve (sempre roda) / fase pesada (`rev-list` sob demanda)
- [ ] `need_ahead_behind` = renderiza colunas AHEAD/BEHIND **ou** query tem attr `ahead`/`behind`
- [ ] `ahead`/`behind` ficam `None` quando a fase pesada não roda; `render_table` imprime `-`

### ⬜ ADR fase 3 — inventory cache + refresh  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **Spec Fase A** (common-dir = identidade do hash), **ADR fase 2**

- [ ] `inventory.rs`: arquivo por repo em `~/.open-loops/inventory/<hash-common-dir>.json`
- [ ] memo de ahead/behind validado por `(head_sha, ab_base_sha)`
- [ ] write-through em todo scan (inclusive `loops api`)
- [ ] escrita atômica (tmp + rename)
- [ ] `--fresh` ignora o memo; `loops refresh [@ctx]` full reindex
- [ ] `inventory_ttl_secs` no config (default 0 = só validação por SHA)
- [ ] limpeza preguiçosa de órfãos no `refresh` ([ADR 0004](docs/decisions/0004-fase2-evidence-snapshot.md))

### ⬜ ADR fase 4 — contexts `@`  ·  [ADR 0003](docs/decisions/0003-query-engine.md)

Depende de: **ADR fase 2** (push-down)

- [ ] parse de `@nome` resolve `[contexts.nome]` do config
- [ ] `[contexts.X] filter = "..."` no `config.toml`
- [ ] `@none` / `@all` limpam o context default
- [ ] `default_context` (config) + `LOOPS_CONTEXT` (env) — só valem sem `@` na query
- [ ] `@ctx` explícito substitui o `default_context`
- [ ] remover erro "contexts not supported yet" do parser

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
