# open-loops — Spec (Fase A): descoberta de repos agnóstica de layout (bare + worktree)

- **Data:** 2026-06-25
- **Status:** rascunho — derivado de brainstorm de design; aguardando revisão do autor
- **Produto:** CLI `loops` — `src/scanner.rs` (descoberta de repositórios)
- **Sequência:** **Fase A de 2.** A [Fase B — atribuição de sessão por worktree](2026-06-25-worktree-session-attribution.md) só começa **depois** desta mergeada. A Fase A entrega *descoberta* (achar as branches); a Fase B entrega *atribuição de sessão* (casar o que a IA fez em cada worktree). Ver §9.

## 1. Problema

O scanner não enxerga o layout **git bare + worktree** usado pelo autor. Hoje
`find_repos` (`src/scanner.rs`) só reconhece um repo quando `dir/.git` é um
**diretório**:

```rust
fn walk(dir: &Path, depth: usize, repos: &mut Vec<PathBuf>) {
    if dir.join(".git").is_dir() { repos.push(dir.to_path_buf()); return; }
    ...
}
```

No layout do autor `.git` é **sempre arquivo** (e o store fica em `.bare/`):

```
~/repo/pigz/back/pigz-api/
├── .bare/          # store real (bare); SEM subdir .git
├── .git            # arquivo:  gitdir: ./.bare
├── main/           # worktree;  main/.git é arquivo → gitdir: .../.bare/worktrees/main
├── dev/            # worktree
└── feat-*/         # worktrees
```

Consequência verificada: nenhum `.git`-diretório existe na árvore, logo
`find_repos` retorna **zero repos**. `loops`, `loops resume` e o futuro
`loops worktrees` (que itera sobre `find_repos`) não encontram nada no ambiente
real do autor. Repos *bare* puros (`git init --bare`, store no próprio dir, sem
`.git`) também já não são detectados hoje.

**Princípio de design (a regra desta spec):** não dar match em layout de pasta.
Nenhuma heurística de path específica (`.bare`, nomes de worktree, profundidade
fixa) — isso viraria hardcode do caso do autor. Em vez disso, **perguntar ao
git**: normal, bare, worktree e submodule são arranjos físicos da mesma coisa
lógica — um *branch store* (o `--git-common-dir`). Classificar interrogando o
git e **deduplicar pelo common-dir** torna a descoberta agnóstica de layout.

## 2. Escopo

| Decisão | Valor travado |
|---|---|
| Detecção | Aceitar `.git` **arquivo OU diretório**, e repos **bare** (probe estrutural barato + confirmação via git). |
| Canonicalização | Resolver cada candidato a `--git-common-dir` (absoluto) e **deduplicar** por ele. N worktrees do mesmo repo = 1 conjunto de loops. |
| `repo_name` | Derivado do common-dir, agnóstico de layout (§5). Nunca mais `.bare`/`main`/`dev`. |
| Profundidade | `scan_depth` configurável (`config.toml`), default 4. Substitui `MAX_DEPTH` fixo. |
| Enumeração de branch | Inalterada — `open_loops` já roda contra bare (verificado: `for-each-ref`, `branch --merged`, `rev-list` funcionam sem working tree). |

**Fora de escopo (explícito):**
- Atribuição de sessão por worktree → **Fase B**. Nesta fase, `repo_path` aponta
  para o container do repo; `loops resume` de uma branch em worktree pode destilar
  **sem** excerpts de sessão da IA (degradação consciente — log/diffstat/LLM
  seguem funcionando). Ver §8 e a Fase B.
- Submodules como repos independentes (o common-dir aponta pro pai; ficam
  deduplicados sob o repo dono — comportamento aceitável, não objetivo).

## 3. Detecção (varredura de FS)

`walk` passa a marcar `dir` como **candidato a repo** se qualquer:

1. `dir.join(".git")` **existe** — arquivo (worktree/submodule/ponteiro bare) ou
   diretório (repo normal). Troca `is_dir()` por `exists()`.
2. `dir` é **bare**: probe estrutural barato — existe `dir/HEAD` **e**
   `dir/objects/` **e** `dir/refs/`. Cobre `git init --bare` e o caso de a root
   apontar direto pro store.

Mantém o **early-return ao detectar** (não desce dentro de um repo). No layout do
autor a detecção dispara em `pigz-api/` (tem o `.git` ponteiro) — **nunca** desce
nos worktrees, então não há dupla contagem na varredura.

- O skip de dirs ocultos e `SKIP_DIRS` (`node_modules`, `target`) continua
  valendo **para a descida**; a checagem de `dir/.git` é explícita e não é afetada
  por ele.
- **Borda conhecida e aceita:** um repo bare nomeado `.bare` *isolado* (sem o
  arquivo `.git` ponteiro no container e não apontado direto por uma root) é
  oculto → pulado na descida → não detectado. O caso comum (worktree) sempre tem
  o ponteiro `.git` no container, que é detectado no nível do container.
  Apontar uma root direto pro bare também funciona. Documentar em
  `configuration.md`.

## 4. Canonicalização + dedup (o coração)

Para cada candidato, uma chamada git:

```
git -C <candidato> rev-parse --path-format=absolute --git-common-dir
```

Resolve, verificado nos repos do autor:

| Candidato | `--git-common-dir` |
|---|---|
| `.../pigz-api` (ponteiro → `.bare`) | `.../pigz-api/.bare` |
| `.../pigz-api/main` (worktree) | `.../pigz-api/.bare` |
| `.../pigz-api/dev` (worktree) | `.../pigz-api/.bare` |
| `.../open-loops` (normal) | `.../open-loops/.git` |

Agrupar candidatos por common-dir; manter **um representante por grupo**. Isso é
o que garante agnosticismo de layout: qualquer combinação de worktrees colapsa
para um único repo lógico, escaneado uma vez. O representante (path onde rodar o
git downstream) é o **candidato detectado** (o container, p.ex. `pigz-api`), não
o common-dir cru — mantém `repo_path` legível e `git -C` resolve igual.

Falha do `rev-parse` em um candidato (não é repo de verdade) → warning, segue
(mesmo princípio de tolerância de `scan`).

## 5. `repo_name` agnóstico de layout

Hoje: `repo.file_name()` — daria `.bare`, `main` ou `dev`. Regra nova, derivada do
**common-dir**:

- se basename do common-dir ∈ {`.git`, `.bare`} → `repo_name` = basename do
  **pai** do common-dir (`.../pigz-api/.bare` → `pigz-api`;
  `.../app/.git` → `app`);
- senão → basename do common-dir **sem** sufixo `.git` (bare `foo.git` → `foo`).

Função pura, testável isoladamente.

## 6. Profundidade configurável

`MAX_DEPTH = 3` (fixo) vira `scan_depth` em `Config` (`src/config.rs`),
**default 4**. Motivo: `pigz-api` está em profundidade 3 a partir de `~/repo`
(no limite atual); árvores org/categoria/repo mais fundas precisam de mais. Como
`walk` faz early-return na fronteira do repo, aumentar o default é barato (só
custa em subárvores sem repo). `SKIP_DIRS` segue fixo nesta fase.

## 7. Arquitetura / arquivos

- **`src/scanner.rs`**
  - `walk`: novo predicado de detecção (§3); `find_repos` recebe `scan_depth`.
  - **Novo** `fn git_common_dir(path) -> Result<PathBuf>` e dedup em `find_repos`
    (ou em `scan`) por common-dir.
  - **Novo** `fn repo_name_from_common_dir(common_dir) -> String` (§5) — pura.
  - `open_loops` / `scan`: assinatura recebe o representante + common-dir; usa
    `repo_name_from_common_dir` no lugar de `file_name()`. Resto inalterado.
- **`src/config.rs`**: campo `scan_depth: usize` (default 4) no `Config`; lido do
  `config.toml`. `label_for_repo` (longest-prefix) inalterado — opera sobre o
  `repo_path` representante.
- **`src/cli.rs`**: passa `cfg.scan_depth` adiante; nenhuma mudança de orquestração
  além disso.
- **`docs/decisions/`**: novo **ADR 0005 — descoberta de repos por interrogação ao
  git** registra a decisão transversal (perguntar ao git + dedup por common-dir
  em vez de heurística de path). Referencia ADR 0002 (shell-out) e 0003 (key
  canônica `root_label/repo/branch`, que continua válida — `repo_name` muda de
  *fonte*, não de *formato*).

## 8. Casos de borda

- Repo normal (`.git` dir) → common-dir `.../.git`, `repo_name` = pai. Inalterado.
- Worktree + container apontam pro mesmo common-dir → 1 repo (dedup).
- Bare direto (root → `.bare` ou `foo.git`) → detectado pelo probe; nome correto.
- Branch **sem** worktree (não checada out) → entra no inventário normalmente
  (enumerada do common-dir). Sessão da IA fica vazia até a Fase B — esperado.
- Colisão de hash de path no inventory (ADR 0003) → o common-dir absoluto é a
  identidade estável; usar ele como base do hash.
- Falha de git em um repo → warning, nunca aborta (igual `scan` hoje).

## 9. Relação com Fase B e com `loops worktrees`

- A **Fase B** depende desta: só faz sentido mapear branch→worktree depois que a
  descoberta acha os repos. Fluem em sequência, nunca em paralelo.
- O comando `loops worktrees` (spec 2026-06-23) também itera sobre `find_repos` —
  esta fase é **pré-requisito** para ele funcionar em ambiente bare. A coleta
  `git worktree list --porcelain` é compartilhada com a Fase B (mesma plumbing).

## 10. Testes

Repos git reais em tempdir (`src/testutil.rs`), com **novos helpers**:

- `init_bare_repo(path)` — `git init --bare`.
- `add_worktree(repo, name, branch)` — cria layout `.bare` + ponteiro `.git` +
  `git worktree add` (espelha o ambiente do autor).

Casos:

- `find_repos` acha repo normal (`.git` dir) — regressão.
- `find_repos` acha repo via ponteiro `.git` arquivo (layout bare+worktree).
- `find_repos` acha repo bare puro (probe estrutural).
- **dedup**: container + 2 worktrees do mesmo repo → 1 entrada.
- `repo_name_from_common_dir`: tabela (`.bare`, `.git`, `foo.git`).
- `open_loops` num repo bare+worktree enumera as branches não-mergeadas (sem
  working tree).
- `scan_depth` configurável: repo em profundidade 4 achado com default; não
  achado com `scan_depth=2`.
- `tests/cli.rs`: `loops` lista branches num fixture bare+worktree.

Gate de cobertura 70% (core 85%) mantido.

## 11. Definition of Done

- [ ] `find_repos` detecta `.git` arquivo/dir e repos bare; early-return e skips preservados.
- [ ] Dedup por `--git-common-dir`; N worktrees → 1 repo.
- [ ] `repo_name` derivado do common-dir (função pura testada).
- [ ] `scan_depth` configurável em `config.toml` (default 4).
- [ ] `loops` lista branches no ambiente bare+worktree do autor (validação manual em `~/repo/pigz`).
- [ ] ADR 0005 escrito; `docs/features.md` + `docs/configuration.md` documentam descoberta e `scan_depth`.
- [ ] Helpers `init_bare_repo`/`add_worktree` em `testutil`; casos novos passando.
- [ ] `just lint` (clippy -D warnings) e `just fmt` limpos; cobertura no gate.
- [ ] CHANGELOG atualizado (git-cliff).
