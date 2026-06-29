# open-loops — Spec: documentação de arquitetura por domínio (`docs/architecture/`)

- **Data:** 2026-06-29
- **Status:** rascunho — aguardando revisão do autor
- **Tipo:** reorganização de documentação (não altera código)
- **Escopo:** consolidar specs + plans + ADRs espalhados numa camada única de
  documentação viva, descritiva, organizada por domínio, focada em **fluxo**.

## Problema

O conhecimento de arquitetura do projeto está fragmentado em três gêneros que
contam a mesma história em pedaços e em momentos diferentes:

- `docs/decisions/` — 8 ADRs (o *porquê* de cada decisão, imutável).
- `docs/superpowers/specs/` — 7 designs por feature (o *que* foi desenhado).
- `docs/superpowers/plans/` — 11 planos de execução (o *como*, com checkboxes,
  já concluídos).

Quem quer entender "como funciona o componente X hoje" precisa cruzar os três e
reconstruir o estado atual a partir de artefatos históricos. Não existe um
documento único, por domínio, que descreva o fluxo ponta-a-ponta do sistema
como ele está implementado.

## Objetivo

Uma camada de documentação **descritiva e viva** em `docs/architecture/`: um
documento por domínio (bounded context), explicando em profundidade **como o
fluxo funciona hoje** — arquitetura, interfaces, fluxo de dados, invariantes e
as decisões que moldaram cada domínio. Mantida junto do código e atualizada
quando ele muda.

Não-objetivos: não é formato RFC/proposta (não olha para mudanças futuras,
exceto a seção *Extensão & limitações*); não altera código; não documenta
features ainda não implementadas como se existissem.

## Decisões de design (travadas no brainstorm)

1. **Recorte por capacidade/bounded-context**, não 1 arquivo = 1 doc. 8 domínios
   de runtime + 1 de build/CI/release = 9 docs + overview + índice.
2. **Consolidação total**: cada doc absorve as decisões relevantes (ex-ADRs)
   numa seção *Decisões*. `docs/decisions/`, `docs/superpowers/specs/` e
   `docs/superpowers/plans/` são **deletados** após a absorção (git preserva o
   histórico).
3. **Template completo por seções** (ver abaixo), com diagrama Mermaid de fluxo
   em cada doc.
4. **Local:** pasta nova `docs/architecture/`, arquivos numerados para dar ordem
   de leitura.
5. **English-first:** todos os docs em `docs/architecture/` são escritos em
   **inglês** — coerente com a regra do projeto (README: "All user-facing output
   is in English: CLI messages, errors, resume sections, and docs"). O conteúdo
   absorvido dos ADRs/specs/plans (em PT) é re-escrito em inglês, não traduzido
   mecanicamente.

## Estrutura final

```
docs/architecture/
  README.md                  índice + como navegar + convenções
  00-overview.md             conceito "open loop", fluxo ponta-a-ponta, pull-only, roadmap
  01-discovery.md            descoberta de repos/branches/worktrees via git
  02-sessions-attribution.md SessionSource + adapter claude_code + atribuição worktree→sessão
  03-query-engine.md         parser de query → ScanPlan → avaliação em memória
  04-inventory-evidence.md   listagem `loops`, ahead/behind, idle, evidence snapshot
  05-resume-distill.md       prompt + LLM via comando configurável
  06-cache-index.md          cache branch@head-sha + índice SQLite
  07-config-state.md         config.toml, estado em ~/.open-loops, ignores
  08-cli-output.md           orquestração CLI, parsing de args, camada de render
  09-build-ci-release.md     matriz CI cross-OS, MSRV, release-plz + cargo-dist
```

## Template por documento

Cada doc de domínio segue estas seções (escaladas à complexidade do domínio):

1. **Propósito** — o que o domínio faz e por que existe (1–2 parágrafos).
2. **Mapa do domínio** — módulos/arquivos que o compõem e os pontos de entrada
   públicos (entrypoints). Tabela arquivo → responsabilidade.
3. **Conceitos & vocabulário** — termos do domínio (ex.: *open loop*,
   *evidence snapshot*, `ScanPlan`, `SessionExcerpt`, `branch@head-sha`).
4. **Fluxo principal** — passo-a-passo do caminho principal + diagrama Mermaid
   (`flowchart` ou `sequenceDiagram`). Núcleo do documento.
5. **Interfaces & contratos** — assinaturas públicas, structs/enums de dados,
   entradas/saídas, erros tipados.
6. **Invariantes & edge cases** — regras que sempre valem (ex.: parsing de
   sessão tolerante: linha ruim = skip + warning, nunca aborta) e casos de borda.
7. **Decisões** — ADRs absorvidos: contexto, decisão e trade-offs. Cita o número
   original (ex.: *ex-ADR-0003*) para rastreabilidade no git.
8. **Extensão & limitações** — pontos de extensão, limitações conhecidas e
   trabalho planejado (ex.: Fase 3 outros harnesses; roadmap library-maturity).
9. **Referências** — `caminho/arquivo.rs:linha` dos pontos-chave, testes
   relevantes, docs de domínio vizinhos, docs user-facing (`features.md`, etc.).

## Mapeamento domínio → código → fontes absorvidas

| Doc | Código-fonte | Absorve |
|---|---|---|
| 00-overview | lib.rs, main.rs (visão geral) | ADR-0001 (mvp pull-only), ADR-0002 (git+LLM via shell-out, princípio cross-cutting), spec+plan `mvp` |
| 01-discovery | scanner.rs, worktrees.rs | ADR-0005 (repo-discovery-via-git), metade-git de ADR-0002, specs `worktree-inventory` + `scanner-bare-worktree-discovery`, plans correspondentes |
| 02-sessions-attribution | sessions/ (trait `SessionSource`, adapter claude_code, `SessionExcerpt`) | spec+plan `worktree-session-attribution` |
| 03-query-engine | query.rs | ADR-0003 (query-engine), plans `query-engine-phase1` + `query-engine-phase4-contexts` |
| 04-inventory-evidence | inventory.rs (+ output.rs como render) | ADR-0004 (fase2-evidence-snapshot), plan `inventory-cache-phase3` |
| 05-resume-distill | distill.rs | metade-LLM de ADR-0002 |
| 06-cache-index | cache.rs, index/ | ADR-0008 (sqlite-index), plan `sqlite-index-migration`, parte cache de `inventory-cache-phase3` |
| 07-config-state | config.rs, state.rs, ignores.rs | (sem ADR dedicado; regras de config/estado/ignores) |
| 08-cli-output | cli.rs, cli_command.rs, output.rs | (estrutura de orquestração CLI e camada de render) |
| 09-build-ci-release | (infra: workflows, release-plz, cargo-dist) | ADR-0006 (ci-msrv-cross-os), ADR-0007 (release-plz + cargo-dist), specs `ci-hardening` + `release-completeness-automation`, plans `ci-hardening-wave1` + `release-completeness-wave2-3`; cross-ref `docs/distribution.md` |

Notas de fronteira:
- `output.rs` é camada de render compartilhada — descrita em 08-cli-output e
  referenciada por 04-inventory-evidence.
- `ignores.rs` mora em 07-config-state e é referenciado por 01-discovery.
- `worktrees.rs` mora em 01-discovery (descoberta), `worktree-session-attribution`
  vai em 02.

## Documentos intocados

`docs/features.md`, `docs/configuration.md`, `docs/distribution.md`,
`docs/setup.md`, `docs/demo.cast` permanecem como referência user-facing. Os
docs de arquitetura **linkam** para eles em vez de duplicar conteúdo.

## Remoções

- `docs/decisions/` (8 ADRs) — absorvidos, deletados.
- `docs/superpowers/specs/` (exceto ver abaixo) — absorvidos, deletados.
- `docs/superpowers/plans/` (11 + STATUS) — absorvidos, deletados.
- `docs/audit/` (3 relatórios de revisão) — descartável, deletado.

**Exceção — `library-maturity-oss-health`:** spec em rascunho, **não
implementada**, trabalho a iniciar em breve. NÃO é deletada. Permanece em
`docs/superpowers/specs/` como o design da feature futura e é referenciada como
roadmap na seção *Extensão & limitações* do `00-overview`.

**Scaffolding desta migração:** este próprio design doc e o plano de
implementação que dele deriva são andaime — uma vez gerados os docs de
arquitetura, são deletados junto com o resto de `specs/` e `plans/` (seu
conteúdo já vive em `docs/architecture/` + git). O README e o `00-overview`
passam a ser a fonte de verdade da arquitetura.

## Verificação (Definition of Done)

- Os 11 arquivos de `docs/architecture/` existem, **escritos em inglês**, e
  seguem o template.
- Todo ADR (0001–0008) tem seu contexto/decisão/trade-off rastreável em alguma
  seção *Decisões* (nenhuma decisão perdida).
- Diagramas Mermaid renderizam (sintaxe válida).
- Referências `file:linha` apontam para código existente (checagem por
  amostragem).
- `docs/decisions/`, `docs/audit/`, `docs/superpowers/plans/` removidos;
  `docs/superpowers/specs/` contém apenas `library-maturity-oss-health`.
- Links internos (entre docs de arquitetura e para docs user-facing) resolvem.
- README de `docs/` raiz / CLAUDE.md atualizados para apontar a nova camada (se
  houver referência ao layout antigo).
