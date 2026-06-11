# Status de Execução — Plano MVP open-loops

> Snapshot para retomada de sessão. Atualizado em: 2026-06-11.
> Plano: `docs/superpowers/plans/2026-06-10-open-loops-mvp.md` (15 tasks)
> Spec: `docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md`
> Branch de trabalho: `feat/mvp` (base: `main`)

## Método de execução

Subagent-driven development (skill `superpowers:subagent-driven-development`):
1 implementer por task (TDD) → spec reviewer → code quality reviewer → fixes até aprovação → próxima task.

**Convenções obrigatórias nos dispatches de subagents:**
- Prompt deve conter o texto COMPLETO da task do plano (subagent não lê o arquivo do plano).
- Incluir: "Your final message IS your report — never end with a greeting or filler" (sem isso, subagents terminam com "Ready." e perdem o relatório).
- Commits: Conventional Commits + linha em branco + `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- Workaround: se comando falhar com `rtk: No such file or directory`, repetir prefixado com `command `.
- Subagents de código devem ler antes: `.agents/skills/rust-best-practices/SKILL.md` e `.agents/skills/rust-testing/SKILL.md`.
- Regra de qualidade aplicada pelos reviewers: sem `expect`/`unwrap` fora de testes; erros em PT acionáveis; docs `///` com `# Errors` em fns públicas.

## Tabela de tasks

| # | Task | Status | Commits | Reviews |
|---|------|--------|---------|---------|
| 1 | Scaffold crate + licenças | ✅ concluída | `1c14bf6` | spec ✅ / quality ✅ |
| 2 | DX: justfile, lefthook, CI gate 70% | ✅ concluída | `d6579da` | spec ✅ / quality ✅ (minor: sem cache cargo no CI — backlog) |
| 3 | Módulo `config` (Config + Store) | ✅ concluída | `d73312d` | spec ✅ / quality ✅ |
| 4 | Módulo `ignores` | ✅ concluída | `536e3ef` + fix `1253c13` | spec ✅ / quality ✅ (após fixes: sem expect, sem TOCTOU, docs) |
| 5 | testutil + scanner `git()`/`default_branch()` | ✅ concluída | `e45cdee` + fix `cb4825d` | spec ✅ / quality ✅ (após fixes: strip_prefix estrito, asserção forte) |
| 6 | Scanner completo (OpenLoop, find_repos, open_loops, scan, helpers) | 🔶 implementada, REVIEWS PENDENTES (implementer rodou antes da interrupção; 12 testes verdes) | `4fc2430` | spec ⬜ / quality ⬜ |
| 7 | Sessions: trait SessionSource + adapter ClaudeCode | ⬜ pendente | — | — |
| 8 | Módulo `cache` | ⬜ pendente | — | — |
| 9 | Módulo `distill` | ⬜ pendente | — | — |
| 10 | Módulo `output` | ⬜ pendente | — | — |
| 11 | CLI (clap + run_list/init/ignore/resume + main) | ⬜ pendente | — | — |
| 12 | Teste E2E (tests/cli.rs) | ⬜ pendente | — | — |
| 13 | Documentação (README, AGENTS.md, docs/, ADRs, templates) | ⬜ pendente | — | — |
| 14 | cargo-dist + git-cliff (release pipeline) | ⬜ pendente | — | — |
| 15 | Verificação final contra spec + dogfood cronometrado | ⬜ pendente | — | — |

**Próxima ação ao retomar:** despachar SPEC REVIEWER da Task 6 sobre o commit `4fc2430` (base `cb4825d`) — implementação já existe e os 12 testes passam; falta spec review + quality review (+ fixes se houver). Depois seguir 7 → 8 → 9 → 10 → 11 → 12 → 13 → 14 → 15.

## Dependências entre tasks

```
6 (scanner: OpenLoop) ──► 8 (cache usa OpenLoop)
6 ──► 9 (distill usa OpenLoop)      7 (sessions: SessionExcerpt) ──► 9 (distill usa SessionExcerpt)
6 ──► 10 (output usa OpenLoop)
3,4,6,7,8,9,10 ──► 11 (CLI orquestra todos)
11 ──► 12 (E2E exercita o binário)
11,12 ──► 13 (docs descrevem comandos reais)
1 ──► 14 (cargo-dist lê Cargo.toml; independente do resto do código)
todas ──► 15 (verificação final)
```

7 é independente de 6 (pode rodar antes ou depois; 9 precisa de ambas).

## Critério de pronto (Definition of Done)

**Por task:**
1. TDD: evidência de red (teste falhando) antes de green.
2. `cargo test` completo verde; `cargo clippy --all-targets -- -D warnings` limpo; `cargo fmt --check` limpo.
3. Spec review ✅ (reviewer independente compara código vs task do plano).
4. Quality review Approved (reviewer independente; fixes aplicados e re-revisados se necessário).
5. Commit Conventional Commits com trailer do co-autor.

**Do MVP (Task 15):**
- Suite completa + cobertura ≥70% linhas (`cargo llvm-cov --fail-under-lines 70`).
- Dogfood real cronometrado: `loops` <5s; `loops resume` frio <60s, cacheado <1s.
- Checklist da spec (seção "Step 3" da Task 15 do plano) todo ✓.
- Merge de `feat/mvp` → `main` via skill `superpowers:finishing-a-development-branch` + review final do conjunto.

## Backlog de melhorias não-bloqueantes (anotadas pelos reviewers)

- CI: adicionar cache de cargo (rust-cache) — minor da Task 2.
- `ignores`: `set.clone()` por add aceitável na escala atual; teste de arquivo vazio poderia assertar emptiness explícita.
- `scanner`: caminho de fallback `master` e erro "não achei branch default" sem testes dedicados.
- `Cargo.toml`: `repository = "https://github.com/gustavo/open-loops"` é placeholder — confirmar user/org real antes do release.

## Ambiente

- `just` e `lefthook` NÃO instalados na máquina — hooks inativos até rodar `just setup`; CI valida tudo.
- Testes: 8 passando (config 4, ignores 2, scanner 2).
