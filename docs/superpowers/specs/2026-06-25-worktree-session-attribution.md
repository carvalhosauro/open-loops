# open-loops — Spec (Fase B): atribuição de sessão por worktree

- **Data:** 2026-06-25
- **Status:** rascunho — derivado de brainstorm de design; aguardando revisão do autor
- **Produto:** CLI `loops` — `src/scanner.rs` + `src/sessions/claude_code.rs`
- **Sequência:** **Fase B de 2.** Depende da [Fase A — descoberta bare+worktree](2026-06-25-scanner-bare-worktree-discovery.md), que **precisa estar mergeada antes** desta começar. A Fase A faz `loops` *achar* as branches; a Fase B faz as sessões da IA *casarem* com cada branch no layout worktree.

## 1. Problema

Depois da Fase A, a descoberta funciona, mas a **atribuição de sessão** ainda
não. `OpenLoop.repo_path` aponta para o **container** do repo (p.ex.
`.../pigz-api`), enquanto o adapter de sessão (`src/sessions/claude_code.rs`,
`ClaudeCode::excerpts`) localiza sessões **codificando o path do cwd** em
`~/.claude/projects/<cwd-encodado>/`.

No fluxo worktree-por-branch, o cwd onde a IA rodou é a **worktree da branch**
(`.../pigz-api/dev`), **não** o container nem o `.bare`. Verificado:

| Path | `rev-parse --is-bare-repository` |
|---|---|
| `.../pigz-api` (container, `.git`→`.bare`) | `true` |
| `.../pigz-api/main` (worktree) | `false` |
| `.../pigz-api/dev` (worktree) | `false` |

Logo, com `repo_path = container`, o encode não bate com nenhum diretório de
projeto, e `loops resume <branch-em-worktree>` destila **sem os excerpts da
sessão** — perde justamente o sinal de "o que a IA estava fazendo".

**Objetivo:** mapear cada branch à sua worktree (quando checada out) e usar esse
path na busca de sessão — sem hardcode de nome de worktree, sempre via git.

## 2. Escopo

| Decisão | Valor travado |
|---|---|
| Fonte da verdade | `git worktree list --porcelain` no representante do repo (uma chamada por repo). |
| Mapa | `branch → worktree_path` (1:1; git proíbe a mesma branch checada out em duas worktrees). |
| `repo_path` por loop | = worktree da branch **se checada out**; senão o container/common-dir (fallback da Fase A). |
| Busca de sessão | `ClaudeCode::excerpts` recebe o path da worktree quando existe. |
| git ops | `git_log`/`diffstat`/`commit_window` rodam no mesmo path (worktree vê todas as branches; bare também). Sem regressão. |

**Fora de escopo:**
- Inventário/limpeza de worktrees (`loops worktrees`) — feature à parte
  (spec 2026-06-23). Compartilha a coleta `worktree list`, mas o veredito de
  limpeza não entra aqui.
- Sessões de branches **sem** worktree: continuam vazias (correto — a IA nunca
  rodou ali). Não é falha.

## 3. Design

Em `open_loops` (ou um helper chamado por ele), depois de enumerar as branches:

```
git -C <repo> worktree list --porcelain
```

Parse do porcelain → entradas `worktree <path>` / `branch refs/heads/<nome>`
(ignorar `bare`, `detached`, `locked` para fins de mapa). Constrói
`HashMap<String, PathBuf>` (branch → path).

Para cada `OpenLoop`:

- `repo_path = mapa.get(&branch).cloned().unwrap_or(<container/common-dir>)`.

Isso serve **os dois** consumidores com um campo só:

- **git ops** — funcionam de qualquer worktree do repo (ou do bare). Sem mudança
  de assinatura.
- **busca de sessão** — `excerpts(repo_path, …)` agora recebe o cwd real onde a
  IA rodou, então o encode bate com `~/.claude/projects/`.

**Backward compatible:** num repo normal, a branch default está checada out no
próprio dir do repo → `repo_path` = dir do repo = comportamento de hoje. Branch
feature comum (sem worktree dedicada) → fallback = dir do repo, igual hoje.

### Decisão: campo único vs. campo novo

Manter **um** `repo_path` (worktree-quando-existe, container-senão) em vez de
adicionar `session_path`. Justificativa: git ops são corretas de qualquer um dos
dois paths, então não há ganho em separar — e um campo só evita divergência de
estado e mantém a `key()` / cache (ADR 0003) intocados (`repo_path` nunca entra
na chave). O common-dir (identidade pro dedup/inventory da Fase A) permanece a
âncora estável; `repo_path` é só "onde rodar git / achar sessão".

## 4. Arquitetura / arquivos

- **`src/scanner.rs`**
  - **Novo** `fn worktree_map(repo: &Path) -> Result<HashMap<String, PathBuf>>`
    — parse de `worktree list --porcelain`. Pura sobre a saída do git (testável
    com fixture de string).
  - `open_loops`: chama `worktree_map` uma vez; resolve `repo_path` por branch.
    Falha do `worktree list` → mapa vazio + warning (degrada pro fallback, nunca
    aborta).
- **`src/sessions/claude_code.rs`**: nenhuma mudança de assinatura — passa a
  receber o path correto. Confirmar que `excerpts` não assume nada além do cwid
  encodado.
- **`src/cli.rs`**: nenhuma mudança (já passa `lp.repo_path` para
  `excerpts`/`git_log`/`diffstat`).
- Reuso possível com `loops worktrees`: extrair o parse de `--porcelain` para um
  helper compartilhado se as duas features landarem próximas.

## 5. Casos de borda

- Branch em detached HEAD numa worktree → sem `branch refs/heads/...` → não entra
  no mapa → fallback. Correto.
- Branch sem worktree → fallback container; sessão vazia. Correto.
- Repo normal sem worktree extra → mapa tem só a default no dir do repo;
  comportamento idêntico ao atual.
- `worktree list` lista worktree `prunable` (dir sumiu) → path inválido; a busca
  de sessão simplesmente não acha nada (sem crash). Aceitável.
- Mesma branch em duas worktrees → impossível por construção do git.

## 6. Testes

Reusa helpers da Fase A (`init_bare_repo`, `add_worktree`):

- `worktree_map` parseia `--porcelain` (fixture string): branches → paths;
  ignora `detached`/`bare`.
- `open_loops` em fixture bare+worktree: branch com worktree → `repo_path` = path
  da worktree; branch sem worktree → `repo_path` = container.
- Integração de sessão: criar `~/.claude/projects/<encode-da-worktree>/` fake com
  uma sessão; `loops resume <branch>` inclui o excerpt. Branch sem worktree →
  resume destila sem excerpt (sem erro).
- Regressão: repo normal → `repo_path` inalterado; sessão casa como antes.

Gate de cobertura 70% (core 85%) mantido.

## 7. Definition of Done

- [ ] `worktree_map` parseia `worktree list --porcelain` (helper testado).
- [ ] `open_loops` resolve `repo_path` por branch (worktree quando existe, fallback senão).
- [ ] `loops resume <branch-em-worktree>` traz excerpts da sessão no ambiente do autor (validação manual em `~/repo/pigz`).
- [ ] Repo normal sem regressão (sessão casa como antes).
- [ ] `docs/features.md` documenta o casamento de sessão por worktree; ADR 0005 (Fase A) atualizado com a nota de atribuição, se necessário.
- [ ] Testes novos passando; `just lint` e `just fmt` limpos; cobertura no gate.
- [ ] CHANGELOG atualizado (git-cliff).
