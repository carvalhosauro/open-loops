# ADR 0003: Query engine, contexts e inventory cache

Data: 2026-06-24 · Status: aceito

## Contexto

Usuários com múltiplas roots (trabalho, pessoal), dezenas de repositórios e
várias branches por repo veem o inventário (`loops`) como ruído: tudo é
escaneado e listado antes de filtrar. O `resume` já aceita query fuzzy, mas
a listagem é all-or-nothing.

O scan atual (`find_repos` → `open_loops` em paralelo) é eager: para cada
branch não mergeada roda `rev-list --left-right --count` (ahead/behind). Com
10 repos × 8 branches ≈ 80 subprocessos git só para montar a tabela.

## Propósito

1. **Escopo** — ver só o que importa agora (`loops api`, `loops @work`).
2. **Vocabulário estável** — separar trabalho/pessoal sem decorar paths
   (`@work`, `@personal`).
3. **Performance** — não varrer nem interrogar git além do que a query exige;
   amortizar scan completo em cache reutilizável.
4. **Consistência** — o mesmo motor de query alimenta `loops`, `loops
   worktrees` e `loops resume` (este último exige match único).

Inspiração: filtros e contexts do [Taskwarrior](https://taskwarrior.org/)
— query declarativa sobre dados já existentes, sem CRUD manual por loop.

## Decisão

Introduzir um módulo `query` que:

1. Parseia a query em um **`ScanPlan`** antes de qualquer I/O pesado.
2. Executa git em **duas fases** (leve → pesada sob demanda).
3. Persiste um **inventory cache** por repositório em `~/.open-loops/inventory/`.
4. Suporta **contexts** (escopo persistente) e **reports** (queries salvas).

### Sintaxe de query

**Termos soltos** — AND implícito; cada termo casa por substring
(case-insensitive) em `repo`, `branch` ou `key`:

```bash
loops api                  # todas as branches do repo "api"
loops api feat/login       # repo + branch
loops work/api             # desambigua quando há colisão de nome
```

**Atributos** — estilo Taskwarrior (`attr:valor`, comparadores `>`, `<`,
`>=`, `<=`):

| Atributo | Descrição | Exemplos |
|---|---|---|
| `repo` | nome ou chave com prefixo de root | `repo:api`, `repo:work/api` |
| `branch` | nome da branch | `branch:feat/` |
| `root` | path/prefixo de uma root configurada | `root:~/work` |
| `idle` | tempo desde o último commit | `idle:>7d`, `idle:<2d` |
| `ahead` / `behind` | contagem vs default branch | `behind:>0` |
| `key` | chave completa | `key:work/api/feat/x` |

**Tags virtuais** — sem estado extra; modificam o conjunto:

| Tag | Efeito |
|---|---|
| `-ignored` | default: esconde loops ignorados |
| `+ignored` | inclui ignorados (auditoria) |
| `+stale` | atalho para `idle:>14d` (threshold configurável) |

**Contexts** — escopo persistente no `config.toml`:

```toml
[contexts.work]
filter = "root:~/work"

[contexts.personal]
filter = "root:~/personal"
```

Uso: `loops @work`, `loops @work api`. O prefixo `@` distingue contexto de
repo chamado `work`.

**Reports** — queries salvas, invocadas com `:`:

```toml
[reports.stale-work]
filter = "@work idle:>14d"

[reports.hot]
filter = "idle:<3d"
```

Uso: `loops :stale-work`. Um report pode compor um context com filtros
adicionais; contexts e reports diferem por **intenção**, não por sintaxe:

| | Context (`@nome`) | Report (`:nome`) |
|---|---|---|
| Pergunta | *Em qual universo estou?* | *Qual recorte quero ver agora?* |
| Uso | Hábito diário (trabalho vs pessoal) | Auditoria periódica (stale, hot) |
| Ativação | Escopo implícito na query | Invocação explícita |

Opcional: `default_context = "work"` no config ou `LOOPS_CONTEXT=work` para
filtrar `loops` sem argumentos.

### Chave canônica

Quando o mesmo `repo_name` existe em roots diferentes, a chave exibida e
usada em `ignore`/`resume` passa a ser `{root-label}/{repo}/{branch}` (ex.:
`work/api/feat/billing`). `root-label` é o basename da root ou alias no
config. Colisões sem prefixo listam ambas com aviso de desambiguação.

## Como funciona

### Pipeline

```
query string
    → parse (query.rs)
    → ScanPlan { roots, repo_filter, branch_filter, attr_filters, need_ahead_behind }
    → find_repos(roots)                    # só roots do plano
    → filter repos by repo_filter          # antes de git pesado
    → per repo: inventory cache hit?
         sim  → carrega JSON
         não  → open_loops (fase leve [+ pesada se need_ahead_behind])
    → eval attr_filters em memória
    → render tabela / pick único (resume)
```

### ScanPlan

Estrutura derivada da query **antes** do scan:

```rust
struct ScanPlan {
    roots: Vec<PathBuf>,              // subset de cfg.roots (@work, root:...)
    repo_filter: Option<Pattern>,
    branch_filter: Option<Pattern>,
    attr_filters: Vec<AttrFilter>,    // idle:>7d, behind:>0, ...
    need_ahead_behind: bool,          // false se query/tabela não precisam
    include_ignored: bool,            // +ignored
}
```

Push-down de escopo:

| Query | Efeito |
|---|---|
| `loops @work` | `find_repos` só em roots de trabalho |
| `loops api` | `open_loops` só em repos cujo nome casa |
| `loops @work api` | 1 root × 1 repo |

### Git em duas fases

**Fase leve** (por repo, ~3 git calls):

- `default_branch`
- `branch --merged`
- `for-each-ref refs/heads` → branch, sha, `last_commit`

Basta para: listagem, `idle:`, ordenação por staleness, `repo:`, `branch:`.

**Fase pesada** (por branch, sob demanda):

- `rev-list --left-right --count` → ahead/behind

Só roda quando `need_ahead_behind` é true — query referencia `ahead`/`behind`,
ou a tabela pede essas colunas. Branches eliminadas pelos filtros leves não
pagam fase pesada.

### Inventory cache

Arquivo por repo em `~/.open-loops/inventory/<hash>.json`:

```json
{
  "repo_path": "/home/you/work/api",
  "default_head": "abc123",
  "indexed_at": "2026-06-24T10:00:00Z",
  "loops": [
    {
      "branch": "feat/billing",
      "head_sha": "...",
      "last_commit": "...",
      "ahead": 5,
      "behind": 0
    }
  ]
}
```

**Invalidação** (barata, por repo):

1. `git rev-parse HEAD` na default branch.
2. Se `default_head` igual ao cache → repo fresco; pula git.
3. Se diferente → reindexa só aquele repo.

**Política de uso:**

| Comando | Cache |
|---|---|
| `loops` (sem query) | full scan; **escreve** inventory |
| `loops api` | **lê** cache; git só em miss/stale |
| `loops resume api/x` | lê cache para resolver loop; git pesado (log, diffstat, sessões, LLM) só no resume |

`loops refresh` (opcional) reindexa tudo ou um context (`loops refresh @work`).
TTL configurável (`inventory_ttl_secs`, default 0 = só invalidação por HEAD).

### Comportamento por comando

| Comando | Query | Resultado |
|---|---|---|
| `loops [query]` | opcional | 0..N loops → tabela |
| `loops worktrees [query]` | opcional | mesmos filtros, domínio worktree |
| `loops resume <query>` | obrigatória | filtra → **exige 1 match** → destila |

Query sem resultados:

```
No loops match: @work api idle:>30d
(hint: run `loops` to list all, or `loops help query`)
```

## Por quê

1. **Taskwarrior-like sem CRUD** — o inventário vem do git; query filtra, não
   mantém estado por loop. Tags são virtuais; contexts/reports vivem no config.
2. **Push-down antes de git** — o ganho real está em não spawnar subprocessos
   para repos/branches que a query já exclui, não em filtrar um `Vec` em RAM.
3. **Fase leve + pesada** — ahead/behind é O(branches) em subprocessos; a
   maioria das queries (repo, idle, context) não precisa dessa coluna.
4. **Inventory cache separado do cache de destilação** — inventário muda a
   cada commit; destilação muda por HEAD-sha de branch. Misturar invalidaria
   demais ou de menos. Scan completo (`loops`) aquece o inventory; queries
   filtradas ficam quase gratuitas em seguida.
5. **Motor unificado** — resume, list e worktrees compartilham parse e filtros;
   só o pós-processamento difere (tabela vs pick-1 vs worktree verdict).
6. **Contexts vs reports** — context responde "onde estou" (escopo); report
   responde "o que quero ver" (receita). Overlap intencional; reports podem
   esperar a v2 se contexts + query ad-hoc bastarem.

## Consequências

**Positivas**

- `loops api` e `loops @work` escalam com o número de repos **no escopo**,
  não com o total configurado.
- Queries repetidas após `loops` ou `loops refresh` são ~instantâneas (leitura
  de JSON + filtro em memória).
- Separação trabalho/pessoal sem múltiplos binários ou configs.

**Negativas / riscos**

- Inventory cache pode ficar stale entre commits e o próximo `loops` — mitigado
  por invalidação via `default_head`; flag `--fresh` força reindex.
- Parser de query adiciona superfície de testes e docs (`loops help query`).
- Chave com prefixo de root é breaking change para `ignore` existentes — exige
  nota de migração ou compatibilidade com chave curta quando não há colisão.
- OR e parênteses ficam fora da v1 (só AND); suficiente para os casos
  conhecidos.

## Fases de implementação

| Fase | Entrega | Dependências |
|---|---|---|
| 1a | `query.rs`: parse → `ScanPlan`, push-down de roots/repo | — |
| 1b | `loops [query…]` com termos soltos e atributos básicos | 1a |
| 2 | Git fase leve; ahead/behind lazy | 1a |
| 3 | Inventory cache por repo + `loops refresh` | 2 |
| 4 | Contexts `@nome` no config | 1a |
| 5 | Reports `:nome`, tags `+stale`, `loops help query` | 4 |
| 6 | Mesmo engine em `worktrees` e `resume`; chave com root-prefix | 1b |

Fase 1 entrega o valor principal (escopo + `loops api`); fase 3 entrega a
economia de processamento futuro.

## Fora de escopo (v1)

- Sintaxe SQL ou expressões com OR/parênteses.
- Tags manuais por loop (`loops tag work …`).
- Daemon em background; warm via cron/`loops refresh` é suficiente.
- Filtro que evita `find_repos` walk além de restringir roots — o walk é
  barato vs git; otimização prematura.
