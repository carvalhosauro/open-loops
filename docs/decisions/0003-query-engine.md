# ADR 0003: Query engine, contexts e inventory cache

Data: 2026-06-24 (revisado após brainstorm + revisão adversarial) · Status: aceito

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
   amortizar a parte cara do scan (ahead/behind) em cache reutilizável. O ganho
   é maior em queries com escopo (`loops api`); o `loops` sem escopo fica mais
   barato no repeat (pula o `rev-list` memoizado) mas ainda paga a fase leve.
4. **Consistência** — o mesmo motor de **parse + filtro** alimenta `loops`,
   `loops worktrees` e `loops resume` (este último exige match único).

Inspiração: filtros e contexts do [Taskwarrior](https://taskwarrior.org/)
— query declarativa sobre dados já existentes, sem CRUD manual por loop.

## Decisão

Introduzir um módulo `query` que:

1. Parseia a query em um **`ScanPlan`** antes de qualquer I/O pesado.
2. Executa git em **duas fases** (leve sempre → pesada memoizada sob demanda).
3. Persiste um **inventory cache** por repositório em `~/.open-loops/inventory/`
   que memoiza **apenas** a parte cara (ahead/behind), validada por SHA.
4. Suporta **contexts** (escopo persistente) e **reports** (queries salvas).
5. Adota uma **chave canônica sempre prefixada por root** (`root-label/repo/branch`).

### Sintaxe de query

**Termos soltos** — split **só por whitespace** (uma `/` é literal dentro do
termo); AND implícito; cada termo casa por substring (case-insensitive) em
`repo`, `branch` ou na chave canônica (que já é prefixada):

```bash
loops api                  # todas as branches do repo "api"
loops api feat/login       # dois termos: "api" AND "feat/login"
loops work/api             # um termo: substring "work/api" na chave prefixada
```

**Atributos** — estilo Taskwarrior (`attr:valor`):

| Atributo | Tipo | Comparadores | Exemplos |
|---|---|---|---|
| `repo` | substring | — | `repo:api` |
| `branch` | substring/prefixo | — | `branch:feat/` |
| `root` | path/prefixo de root | — | `root:~/work` |
| `key` | substring na chave | — | `key:work/api/feat/x` |
| `idle` | duração | `>` `<` `>=` `<=` (operador **obrigatório**) | `idle:>7d`, `idle:<2d` |
| `ahead` / `behind` | inteiro | `>` `<` `>=` `<=` ou igualdade nua | `behind:>0`, `ahead:0` |

- **Gramática de duração**: `<N>(m|h|d|w)` — minutos, horas, dias, semanas.
- `idle:7d` sem comparador é erro com dica (igualdade exata sobre duração não
  faz sentido). `ahead`/`behind` aceitam igualdade nua (`ahead:0`).
- **`ahead`/`behind` em qualquer query forçam a fase pesada** (ver
  `need_ahead_behind` abaixo), mesmo em `resume`.
- `root:` faz tilde-expand + canonicalização antes de casar por prefixo contra
  as roots (já canonicalizadas) do config. Se nenhuma root casa, o resultado é
  vazio (com a dica padrão), não erro.

**Tags virtuais** — sem estado extra; modificam o conjunto:

| Tag | Efeito |
|---|---|
| `-ignored` | default: esconde loops ignorados |
| `+ignored` | inclui ignorados (auditoria) |
| `+stale` | atalho para `idle:>{stale_threshold}` (default 14d, configurável) |

**Contexts** — escopo persistente no `config.toml`:

```toml
[contexts.work]
filter = "root:~/work"

[contexts.personal]
filter = "root:~/personal"
```

Uso: `loops @work`, `loops @work api`. O prefixo `@` distingue contexto de
repo chamado `work`. **`@none`** (ou `@all`) limpa o contexto default para uma
visão completa pontual.

**Reports** — queries salvas, invocadas com `:`:

```toml
[reports.stale-work]
filter = "@work idle:>14d"

[reports.hot]
filter = "idle:<3d"
```

Uso: `loops :stale-work`. O filtro de um report é parseado como uma sub-query:
pode embutir **um** `@context`, mas **não** pode referenciar outro `:report`
(guard de profundidade = 1; violação é erro). Contexts e reports diferem por
**intenção**, não por sintaxe:

| | Context (`@nome`) | Report (`:nome`) |
|---|---|---|
| Pergunta | *Em qual universo estou?* | *Qual recorte quero ver agora?* |
| Uso | Hábito diário (trabalho vs pessoal) | Auditoria periódica (stale, hot) |
| Ativação | Escopo implícito na query | Invocação explícita |

**Composição e precedência** — tudo é AND:

```
efetivo = (default_context, salvo se @ explícito ou @none)
          ∧ @context explícito
          ∧ expand(:report)
          ∧ termos/atributos/tags ad-hoc
```

- `@context` explícito **substitui** o `default_context` (comportamento
  Taskwarrior). `default_context` (config) e `LOOPS_CONTEXT` (env) só valem
  quando não há `@` na query; `@none`/`@all` os ignora.
- `OR` e parênteses ficam fora da v1 (só AND); suficiente para os casos
  conhecidos.

### Chave canônica

A chave é **sempre** `root-label/repo/branch` (ex.: `work/api/feat/billing`),
exibida na tabela e usada em `ignore`/`resume`/cache. Prefixo sempre presente,
**estável**: adicionar um segundo repo chamado `api` sob outra root nunca muda
a chave do primeiro.

- `root-label` = alias configurado para a root, senão o basename dela.
- **Colisão de label** (duas roots com mesmo basename e sem alias) é **erro
  acionável** em qualquer comando que escaneia (`loops`, `resume`, `worktrees`):
  `roots A and B share label 'repos'; set an alias in config.toml`.

**Contrato de implementação (consumidores da chave):**

```rust
struct OpenLoop {
    root_label: String,          // novo: resolvido da root dona no scan
    repo_name: String,
    repo_path: PathBuf,
    branch: String,
    head_sha: String,
    last_commit: DateTime<Utc>,
    ahead: Option<u32>,          // None quando a fase pesada não rodou (ex.: resume sem ahead/behind)
    behind: Option<u32>,
}

impl OpenLoop {
    fn key(&self) -> String {
        format!("{}/{}/{}", self.root_label, self.repo_name, self.branch)
    }
}
```

- `OpenLoop::key()` passa de `repo_name/branch` para `root_label/repo_name/branch`.
- **Distill cache** (`cache.rs`): o path passa de `cache/<repo>/<branch>@<sha>.md`
  para `cache/<root_label>/<repo>/<branch_escapado>@<head_sha>.md` (a `/` na
  branch continua escapada para `__`). É essa mudança — não só o `key()` — que
  corrige a colisão latente entre repos de mesmo nome. Cache é descartável; sem
  migração.
- `resolve_loop` (`cli.rs`) passa a casar a substring contra a chave de 3
  segmentos.
- `render_table` (`output.rs`) imprime `-` quando `ahead`/`behind` são `None`.
  No caminho da tabela `need_ahead_behind` é sempre true, então na prática são
  sempre `Some`; o `-` é defensivo.

**Migração (breaking change, pré-1.0, documentada):**

- `ignores.toml`: as chaves agora são prefixadas; entradas antigas não casam.
  Sem shim de compatibilidade — o break é documentado no CHANGELOG e o usuário
  re-adiciona os ignores.
- `resume` resolve por chave **inclusive** loops ignorados (você pode retomar
  algo que ignorou); a listagem é que esconde ignorados por default.

## Como funciona

### Pipeline

```
query string
    → parse (query.rs)
    → resolve contexts/reports → ScanPlan { roots, repo_filter, branch_filter,
                                            attr_filters, include_ignored,
                                            need_ahead_behind }
    → find_repos(roots)                    # só roots do plano
    → filter repos by repo_filter          # antes de git pesado
    → per repo: fase leve (sempre) — usa inventory para memoizar a fase pesada
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
    include_ignored: bool,            // +ignored
    need_ahead_behind: bool,
}
```

**Derivação de `need_ahead_behind`** (corrigida): true se a saída renderiza as
colunas AHEAD/BEHIND **ou** se a query contém um atributo `ahead`/`behind`:

```
need_ahead_behind = renders_ab_columns || query_tem_attr_ahead_ou_behind
```

Assim `loops` (tabela) → true; `loops resume api` → false; `loops resume api
behind:>0` → true (senão o filtro não teria o que avaliar).

Push-down de escopo:

| Query | Efeito |
|---|---|
| `loops @work` | `find_repos` só em roots de trabalho |
| `loops api` | filtra repos cujo nome casa antes de qualquer git |
| `loops @work api` | 1 root × 1 repo |

### Superfície de CLI (clap)

Hoje `loops` não tem positional de query; só subcommands. A mudança adiciona um
positional variádico de topo que coexiste com os subcommands:

```rust
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,   // resume, ignore, worktrees, init, completions, refresh
    query: Vec<String>,         // ação default: listar com a query
}
```

- Se o primeiro token nomeia um subcommand conhecido → dispatch para ele.
- Senão, todos os positionals viram a query da **ação default** (`run_list`).
- **Shadow rule**: um repo chamado igual a um subcommand (ex.: `resume`) fica
  sombreado; para filtrá-lo use `repo:resume`. Documentar em `loops help query`.

### Git em duas fases

**Fase leve** (por repo, **sempre roda**, ~4-6 git calls — `default_branch` já
custa 1-2 chamadas):

- `default_branch` + `rev-parse` do SHA da default
- `for-each-ref refs/heads --format='%(refname:short)%09%(objectname)%09%(committerdate:iso8601-strict)'`
  → branch, `head_sha`, `last_commit`
- `branch --merged` (para excluir branches mergeadas do conjunto de loops)

Como a fase leve sempre roda, `last_commit`, `head_sha` e o conjunto de loops
(mergeadas excluídas) são **sempre atuais** — nunca servidos stale do cache.
Basta para: listagem, `idle:`, ordenação por staleness, `repo:`, `branch:`.

**Fase pesada** (por branch, sob demanda + memoizada):

- `rev-list --left-right --count {default}...{branch}` → ahead/behind

Só roda quando `need_ahead_behind` é true **e** o valor não está memoizado para
o par `(head_sha, default_sha)`. Quando não roda, `ahead`/`behind` ficam `None`.

### Inventory cache

Arquivo por repo em `~/.open-loops/inventory/<hash-do-path-canônico>.json`.
O cache **não** evita a fase leve; ele memoiza **apenas** o `rev-list` caro.
Por isso guarda só o que a validação lê — `last_commit`, `merged` e `root_label`
**não** entram (são sempre recomputados/derivados, nunca lidos do cache):

```json
{
  "repo_path": "/home/you/work/api",
  "indexed_at": "2026-06-24T10:00:00Z",
  "loops": [
    {
      "branch": "feat/billing",
      "head_sha": "def456",
      "ab_base_sha": "abc123",
      "ahead": 5,
      "behind": 0
    }
  ]
}
```

- `repo_path` é lido para confirmar identidade / detectar colisão de hash.
- `indexed_at` é lido só para TTL (`inventory_ttl_secs`).
- Cada entrada de `loops` é um memo de ahead/behind com suas chaves de validação
  (`head_sha` da branch e `ab_base_sha` = SHA da default no momento do cálculo).

**Validação (por branch, correta para movimento de qualquer branch):**

1. A fase leve roda e produz o `head_sha` atual de cada branch e o
   `default_sha` atual.
2. `ahead`/`behind` são reaproveitados do cache **só se** existir entrada com
   `head_sha` igual ao atual **e** `ab_base_sha` igual ao `default_sha` atual.
3. Caso contrário, recomputa `rev-list` e atualiza a entrada.

Isso resolve o caso dominante que a invalidação por `default_head` deixava
escapar: um commit novo na própria branch muda seu `head_sha`, invalidando o
memo — sem depender de movimento da default.

**Write-through e atomicidade:**

- Todo scan (inclusive `loops api`) persiste o inventory atualizado dos repos
  que tocou — então queries filtradas também aquecem o cache.
- Escrita atômica (tmp + rename) evita corrupção quando dois terminais rodam
  `loops` ao mesmo tempo. Em concorrência o último a escrever vence (lost-update
  benigno: o próximo scan recomputa o que faltar).
- Como `root_label` **não** é persistido, renomear um alias no config nunca
  dessincroniza o cache: a chave de distill é derivada de config + path no
  momento do uso, sempre com o label atual.

**Política de uso:**

| Comando | Cache |
|---|---|
| `loops` (sem query) | fase leve em tudo; memoiza/atualiza ahead/behind |
| `loops api` | fase leve só nos repos casados; memoiza + write-through |
| `loops resume api/x` | fase leve resolve o loop (SHA vivo); pula fase pesada (salvo se a query tem `ahead`/`behind`); git pesado de destilação (log, diffstat, sessões, LLM) só no resume |

`--fresh` (flag de `loops [query]` e `loops resume`) ignora o memo e recomputa
ahead/behind no escopo. `loops refresh [@ctx]` recomputa a fase pesada de
**todas** as branches do escopo (full reindex). TTL configurável
(`inventory_ttl_secs`, default 0 = só validação por SHA). Arquivos órfãos (repo
movido/apagado) são limpos preguiçosamente no `refresh` (cf. ADR 0004).

**resume e o cache de destilação:** o resume roda a fase leve (→ `head_sha`
vivo) para estreitar a 1 match, logo a chave do cache de destilação
(`root_label/repo/branch@head_sha`, ver "Chave canônica") é correta por
construção — sem recomputo especial.

### Comportamento por comando

| Comando | Query | Resultado |
|---|---|---|
| `loops [query]` | opcional | 0..N loops → tabela |
| `loops worktrees [query]` | opcional | mesmos filtros, domínio worktree |
| `loops resume <query>` | obrigatória | filtra → **exige 1 match** → destila |
| `loops refresh [@ctx]` | opcional | reindexa inventory de tudo ou de um context |

**Motor unificado — escopo exato:** o que `loops`, `worktrees` e `resume`
compartilham é **parse → ScanPlan → filtro** (roots, `repo`/`branch`, `idle`,
ignored). A **coleta** difere: `worktrees` usa `git worktree list --porcelain`
+ status/log por worktree, **não** tem ahead/behind nem inventory memo —
`need_ahead_behind` é ignorado nesse domínio. "Mesmo engine" = mesma camada de
filtro, não a mesma coleta.

Query sem resultados:

```
No loops match: @work api idle:>30d
(hint: run `loops` to list all, or `loops help query`)
```

### Config (campos novos)

Além de `roots`/`llm_command`/`sessions_dir`/`max_sessions`/`max_session_kb`
(ver `docs/configuration.md`):

```toml
roots = ["/home/you/work", "/home/you/personal"]

# alias por root, chaveado pelo path canônico (resolve colisão de label)
[aliases]
"/home/you/work" = "w"

default_context = "work"        # opcional; sobreposto por @ctx explícito / @none
stale_threshold = "14d"         # +stale = idle:>{stale_threshold}
inventory_ttl_secs = 0          # 0 = validação só por SHA

[contexts.work]
filter = "root:~/work"

[reports.stale-work]
filter = "@work idle:>14d"
```

Env: `LOOPS_CONTEXT=work` equivale a `default_context` (vale só sem `@` na query).

## Por quê

1. **Taskwarrior-like sem CRUD** — o inventário vem do git; query filtra, não
   mantém estado por loop. Tags são virtuais; contexts/reports vivem no config.
2. **Push-down antes de git** — o ganho real está em não spawnar subprocessos
   para repos/branches que a query já exclui, não em filtrar um `Vec` em RAM.
3. **Fase leve sempre + pesada memoizada** — a fase leve (~4-6 calls/repo) é
   barata e mantém `last_commit`/`head_sha`/conjunto-de-loops sempre corretos;
   só o `rev-list` O(branches) é memoizado. Trocamos "repeat instantâneo só com
   JSON" por "sempre correto + leve barato + pesado memoizado" — escolha
   consciente: nunca exibir estado de branch desatualizado.
4. **Inventory cache separado do cache de destilação** — o inventory memoiza
   ahead/behind (validado por `head_sha`/`default_sha`); a destilação é chaveada
   por `head_sha` da branch. Misturar invalidaria errado.
5. **Motor unificado na camada de filtro** — resume, list e worktrees
   compartilham parse + ScanPlan + filtro; coleta e pós-processamento diferem.
6. **Chave sempre prefixada** — estabilidade vence verbosidade: a chave de um
   repo nunca muda ao adicionar outro de mesmo nome. Como é breaking de qualquer
   forma, quebra-se uma vez, limpo e documentado.

## Consequências

**Positivas**

- `loops api` e `loops @work` escalam com o número de repos **no escopo**,
  não com o total configurado.
- Repeats reaproveitam ahead/behind do memo e nunca exibem estado stale.
- Separação trabalho/pessoal sem múltiplos binários ou configs.
- Chave canônica corrige a colisão latente de repos de mesmo nome no cache de
  destilação.

**Negativas / riscos**

- **`loops` sem escopo não fica instantâneo no repeat**: ainda paga a fase leve
  (~4-6 calls × N repos); só economiza o `rev-list` memoizado. O ganho grande é
  para queries com escopo, que pulam repos fora do filtro. Ex.: 50 repos sem
  escopo ≈ 200-300 chamadas leves por execução (heavy memoizado); `loops api`
  toca só os repos que casam.
- Chave sempre prefixada é breaking change para `ignores` e paths de cache
  existentes — mitigado por ser pré-1.0, com nota de migração no CHANGELOG.
- Parser de query adiciona superfície de testes e docs (`loops help query`).
- Colisão de root-label exige alias manual (erro acionável, não silencioso).
- OR e parênteses ficam fora da v1 (só AND); suficiente para os casos
  conhecidos.

## Fases de implementação

| Fase | Entrega | Dependências |
|---|---|---|
| 1a | `query.rs`: parse → `ScanPlan` (termos, atributos, tags) | — |
| 1b | Chave sempre prefixada: `OpenLoop.root_label` + `key()` de 3 segmentos + `Cache::path` + `resolve_loop` + `ignores`; alias/colisão; superfície clap `loops [query]`; documenta o break | 1a |
| 2 | Wire do `ScanPlan` no scan: push-down + split fase leve/pesada + `need_ahead_behind` | 1a, 1b |
| 3 | `inventory.rs`: memo por SHA (validação via for-each-ref, write-through, atômico) + `refresh`/`--fresh` | 2 |
| 4 | Contexts `@nome`/`@none`/`@all` + config + `default_context`/`LOOPS_CONTEXT` (precisa do push-down da fase 2 para escopar de fato) | 1a, 2 |
| 5 | Reports `:nome` + `+stale` + `loops help query` | 2, 4 |
| 6 | Mesma camada de filtro em `worktrees [query]`; resume já é engine-based após 1b | 1b, 2 |

A Fase 1b (chave) é antecipada porque é breaking e toca resume/ignore/cache —
landar uma vez, cedo, evita migração dupla. Fase 1 entrega o valor principal
(escopo + `loops api`); fase 3 entrega a economia de processamento. A dica
`loops help query` na mensagem de "sem resultados" só vira comando real na fase
5; até lá a dica de help fica latente.

## Fora de escopo (v1)

- Sintaxe SQL ou expressões com OR/parênteses.
- Tags manuais por loop (`loops tag work …`).
- Daemon em background; warm via cron/`loops refresh` é suficiente.
- Filtro que evita `find_repos` walk além de restringir roots — o walk é
  barato vs git; otimização prematura.
- Inventory memo para worktrees — coleta diferente; pode vir depois se medível.
