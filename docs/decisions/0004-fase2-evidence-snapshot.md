# ADR 0004: Fase 2 — evidence snapshot, modo híbrido e skill resume

Data: 2026-06-24 · Status: aceito

## Contexto

O MVP (ADR 0001) valida retomada via pull: `loops resume` varre git, casa sessões
de IA por heurística (janela temporal + menção da branch) e destila via LLM.
Funciona retroativamente, mas a retomada fria custa ~30–60s e o match de sessão
é impreciso — mitigado pela seção Sources e pelo cache por `branch@HEAD-sha`.

A Fase 2 adiciona captura proativa no fim da sessão de IA (hook SessionEnd do
Claude Code) e integração in-harness via skill `/loops:resume`. A spec original
menciona também modo híbrido (snapshot quando existe; fallback no pull) e
destilação assíncrona em background para retomada <1s.

Restrição do Claude Code: o hook `SessionEnd` tem timeout default de **1,5s**
(configurável até 60s). Destilação LLM leva dezenas de segundos — não cabe de
forma síncrona no hook. Isso força separar **evidence** (fatos brutos, gravação
rápida) de **distilled** (resumo nas 4 seções, produzido pelo LLM).

Por fim, snapshots e cache vivem em `~/.open-loops/` — estado semântico que o OS
não gerencia. Hoje o cache de destilação já cresce sem GC automático (um arquivo
por `branch@sha`); evidence snapshot herda o mesmo risco se acumular por sessão
ou por commit sem política explícita.

## Propósito

1. **Captura no ponto certo** — gravar contexto quando a sessão de IA termina,
   usando o `transcript_path` exato (sem heurística sessão→branch).
2. **Retomada híbrida** — usar snapshot quando fresco; cair no pull quando
   stale ou ausente (branches antigas, hook não instalado, primeira sessão).
3. **Retomada in-harness** — skill `/loops:resume` injeta o contexto destilado
   direto na nova sessão de IA, eliminando copy-paste do terminal.
4. **Crescimento limitado** — política de retenção explícita; a CLI é dona do
   lifecycle em `~/.open-loops/`, não o OS.

## Objetivos (critérios de sucesso)

| Objetivo | Meta |
|---|---|
| Hook não bloqueia SessionEnd | < 1s no caminho síncrono |
| Fast path no resume | Pula grep de sessões quando snapshot fresco |
| Retomada fria após snapshot | < 60s (LLM no resume, não no hook) |
| Retomada com cache existente | < 1s (inalterado) |
| Disco | O(slots por branch aberta), não O(sessões encerradas) |
| Auditoria | Output indica fonte: `snapshot` vs `pull` |

## Decisão

### Escopo do MVP da Fase 2: evidence-only

A primeira entrega da Fase 2 grava **apenas evidence** no hook — sem
destilação assíncrona em background. O LLM roda no `loops resume` (ou na skill),
como hoje, mas reutilizando evidence já coletada.

Destilação async no hook fica para subfase posterior (configurável), depois de
validar no dogfood custo, confiabilidade e necessidade de retomada <1s sem cache
prévio.

### Evidence vs distilled

| Camada | Conteúdo | Quem produz | Onde |
|---|---|---|---|
| **Evidence** | commits, diffstat, tail da sessão (`transcript_path`) | `loops` sem LLM | `snapshots/` |
| **Distilled** | Why / Done / Remaining / Next step + Sources | LLM | `cache/` |

Evidence é o *input* da destilação — o mesmo material que `gather_resume_evidence`
monta hoje, exceto que o trecho de sessão vem do transcript do hook (confiança
alta), não do grep heurístico.

### Armazenamento: um slot por branch

```
~/.open-loops/
├── config.toml
├── ignores.toml
├── cache/                    # distilled, keyed por branch@sha (existente)
└── snapshots/              # evidence, um arquivo por branch
    └── <repo>/
        └── <branch>.json     # sobrescrito a cada SessionEnd
```

**Não** usar `branch@sha` para snapshots no MVP. Cada `loops snapshot`
sobrescreve o arquivo da branch. Na leitura, comparar `snapshot.head_sha` com o
HEAD atual: match → fast path; mismatch → stale → fallback pull.

Teto de crescimento: número de branches que já receberam snapshot (~open loops),
não número de sessões encerradas.

### Hook SessionEnd

Novo subcomando `loops snapshot`:

1. Lê JSON do hook via stdin (`cwd`, `transcript_path`, `reason`, `session_id`).
2. Ignora `reason` em `clear` (sessão descartada).
3. Resolve repo git a partir de `cwd`; obtém branch e HEAD sha.
4. Pula se não for open loop (branch default ou `ahead == 0`).
5. Coleta evidence (git log, diffstat, tail do `transcript_path`).
6. Grava `snapshots/<repo>/<branch>.json`.

Instalação via `loops setup hooks` — merge idempotente em
`~/.claude/settings.json`:

```json
{
  "hooks": {
    "SessionEnd": [{
      "hooks": [{
        "type": "command",
        "command": "/path/to/loops snapshot",
        "timeout": 10
      }]
    }]
  }
}
```

Sem `async: true` no MVP evidence-only (hook termina em < 1s). Evento primário:
`SessionEnd`. Hook `Stop` (por turno) fica fora do MVP — muito ruído.

### Modo híbrido em `loops resume`

```
resolve branch
    → cache hit (HEAD atual)?     → print (< 1s)
    → snapshot fresco (sha ok)?   → distill com evidence salva (fast path)
    → senão                       → pull completo (comportamento atual)
```

O fast path pula `matching AI sessions…` — o excerpt já está no snapshot.
Após destilar, grava cache como hoje. O output inclui a fonte:

```markdown
**Source:** snapshot (captured at session end)
**Confidence:** high — session transcript from hook, not heuristic match
```

vs pull:

```markdown
**Source:** pull (live scan)
```

### Skill `/loops:resume`

Arquivo versionado no repositório; instalado em `~/.claude/skills/` via
`loops setup skill`. Invoca `loops resume` com injeção dinâmica (`!`…``) e
instrui o modelo a adotar as 4 seções sem re-perguntar contexto já coberto.

A skill não altera o que o `loops` produz — muda *onde* o contexto aparece
(dentro do harness de IA). Funciona mesmo sem hook (pull/cache); o hook só
torna o caminho mais rápido e preciso.

### Retenção e limpeza — responsabilidade da CLI

O OS **não** limpa `~/.open-loops/`. Regras:

| Evento | Ação | Automático |
|---|---|---|
| `loops snapshot` na mesma branch | Sobrescreve snapshot | Sim |
| `loops resume` com sha stale | Ignora snapshot; pull | Sim (leitura) |
| `loops ignore <repo/branch>` | Apaga snapshot **e** caches da branch | Sim |
| Branch sumiu do inventário (merge/delete) | Prune de snapshot + caches órfãos | Sim (no scan) |
| `loops prune` | Lista/apaga órfãos + caches stale por sha | Manual (`--apply`) |
| Resume OK + cache gravado | **Manter** snapshot (fallback barato) | — |

Snapshot e cache **não** são apagados juntos por regra geral — camadas
diferentes. Exceções unificadas: `ignore`, órfãos, `loops prune`.

TTL opcional futuro: `snapshot_ttl_days` no config (default: sem TTL no MVP).

## Como funciona

### Formato do snapshot (evidence)

```json
{
  "captured_at": "2026-06-24T18:00:00Z",
  "head_sha": "abc123",
  "branch": "feat/login",
  "repo_name": "my-app",
  "repo_path": "/home/me/work/my-app",
  "session_id": "...",
  "transcript_path": "~/.claude/projects/.../session.jsonl",
  "default_branch": "main",
  "commits": "...",
  "diffstat": "...",
  "session_excerpt": "..."
}
```

Tamanho típico: ~50–60 KB (dominado pelo tail da sessão, limitado por
`max_session_kb` no config).

### Pipeline do hook

```
SessionEnd (Claude Code)
    → loops snapshot (stdin JSON)
    → git: repo, branch, sha, open loop?
    → evidence: log + diffstat + transcript tail
    → write snapshots/<repo>/<branch>.json (overwrite)
```

### Pipeline híbrido do resume

```
loops resume <query>
    → resolve loop (scanner / query engine)
    → cache.get(lp)                         → hit? print
    → snapshot.get(repo, branch)
         → head_sha == lp.head_sha?         → distill(evidence) → cache.put → print
         → else                             → gather_resume_evidence (pull) → …
```

### Skill

```
/loops:resume [query]
    → !`loops resume $ARGUMENTS`  (ou branch git atual se vazio)
    → Claude recebe markdown destilado no prompt
    → instrução: executar Next step sem re-perguntar
```

## Por quê

1. **Evidence-only no MVP** — entrega valor (fast path, confiança alta, skill)
   com menos peças que async distill; evita LLM por sessão encerrada e falhas
   silenciosas em background.
2. **Um slot por branch** — bounds de disco previsíveis; sha na leitura, não no
   nome do arquivo; evita acúmulo igual ao cache `@sha` sem prune.
3. **CLI dona do lifecycle** — semântica de open loop, merge e ignore só o
   `loops` conhece; o OS não pode distinguir snapshot órfão de arquivo útil.
4. **Híbrido** — branches pré-hook e hook não instalado continuam funcionando
   (pull); zero regressão.
5. **Snapshot sobrevive ao cache** — se o usuário apagar `cache/`, evidence
   ainda permite fast path no próximo resume; ~50 KB por branch é aceitável.
6. **Skill fecha o último metro** — o USP é retomada de contexto; injetar no
   harness elimina fricção terminal → cola → Claude.

## Consequências

**Positivas**

- Hook rápido e confiável dentro do timeout SessionEnd.
- Sessão do hook = sessão relevante (confiança high sem heurística).
- Disco limitado por branches com snapshot, não por sessões.
- Skill utilizável antes ou sem async distill.
- Política de retenção documentada; resolve débito do cache sem GC.

**Negativas / riscos**

- Primeiro resume após pausa continua ~30s (LLM no resume) — não < 1s até
  cache existir ou async distill entrar.
- Hook só captura dali em diante (como ADR 0001 previu para push).
- Depende de `loops` no PATH na instalação do hook.
- Prune de órfãos no scan adiciona I/O leve a cada `loops` / `resume`.
- Formato `.jsonl` do Claude Code continua não-API-pública (risco 1 da spec).

## Fases de implementação

| Fase | Entrega | Dependências |
|---|---|---|
| 2a | `src/snapshot.rs`, `loops snapshot`, testes stdin + tempdir | — |
| 2b | Modo híbrido em `run_resume` + indicador Source/Confidence | 2a |
| 2c | `loops setup hooks` (merge settings.json) | 2a |
| 2d | Skill `loops-resume` + `loops setup skill` | 2b |
| 2e | Prune de órfãos no scan; `loops ignore` limpa snapshot+cache | 2a |
| 2f | `loops prune` (+ `--apply`); docs `configuration.md` | 2e |
| 2g (futuro) | `snapshot_mode = "distill"` + hook `async: true` | dogfood 2a–2f |

Fase 2a–2b entregam o núcleo (captura + resume híbrido); 2c–2d entregam
integração Claude Code; 2e–2f entregam higiene de disco.

## Fora de escopo (MVP Fase 2)

- Destilação assíncrona no hook (subfase 2g).
- Hook `Stop` (checkpoint por turno).
- Daemon ou cron de manutenção; prune lazy no scan + `loops prune` manual.
- Adapters de sessão além do Claude Code (Fase 3).
- `loops clean` que remove worktrees ou altera repos git.
- TTL automático por idade (opcional pós-MVP; configurável se necessário).

## Relação com ADRs anteriores

- **0001** — pull-only permanece como fallback; híbrido estende, não substitui.
- **0002** — git e LLM continuam via shell-out; hook chama binário `loops`.
- **0003** — resume continua exigindo match único; inventory cache acelera
  resolve, independente de snapshot/cache de destilação.
