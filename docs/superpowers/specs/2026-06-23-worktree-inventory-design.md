# open-loops — Spec: inventário de worktrees + EN-first + autocomplete

- **Data:** 2026-06-23
- **Status:** validado em brainstorming; aguardando revisão final do autor
- **Produto:** CLI `loops` — novo comando `worktrees`, comando `completions`, migração de output para inglês

## 1. Problema

O autor usa fluxo **worktree-por-branch**: cada branch ativa vira uma worktree no
disco. Com o tempo o ambiente acumula worktrees e ele perde o controle:

- Quantas worktrees existem abertas?
- Quais já foram mergeadas na branch default (logo, são lixo no disco)?
- Quais estão paradas frias (sem mudança, sem commit recente) e merecem revisão?
- O que pode ser apagado com segurança?

Hoje `loops` lista **branches não mergeadas**, mas não modela o eixo físico
(worktree no disco). O ambiente fica sujo e o custo mental cresce.

**Encaixe no produto:** uma worktree mergeada parada no disco É um "open loop"
com custo físico. Listá-la e oferecer o comando de limpeza fecha o loop. Reusa
todo o scanner existente (merged-vs-default, idade, ahead/behind). Não dilui a
tese — reforça.

## 2. Escopo (decisões do brainstorming)

| Decisão | Valor travado |
|---|---|
| Nível de ação | **Inventário + comando pronto.** Lista com veredito; pras worktrees apagáveis imprime o comando exato pra copiar. **Não executa nada.** Tool segue advisor. |
| Idioma | **CLI inteira migra para inglês** — outputs e mensagens de erro. Atualiza `features.md`, `configuration.md`, `setup.md` e a regra PT no `CLAUDE.md`. |
| ASCII | **Sem emoji.** Veredito é palavra (`deletable`/`cold`/`active`/`prunable`). Toda saída ASCII-only. |
| Autocomplete | Comando `loops completions <shell>` via `clap_complete` (bash/zsh/fish). |

**Fora de escopo (explícito):** `loops clean` que deleta worktrees. Fica pra
feature futura deliberada com guardas — não entra aqui.

## 3. Comando `loops worktrees` (alias `wt`)

### Fonte de dados

Pra cada repo achado nas raízes configuradas (mesmo `find_repos` atual), roda
`git worktree list --porcelain`. Worktree tem `.git` **arquivo** (não dir), então
o `walk()` atual não a confunde com repo — enumeração vem do repo, não da varredura
de disco. Sem dupla contagem.

Campos do porcelain por entrada: `worktree <path>`, `HEAD <sha>`,
`branch refs/heads/<nome>` (ou `detached`), e flags `bare` / `locked` / `prunable`.

### Sinais computados por worktree

- **branch** — nome da branch (ou `(detached)`).
- **idle** — idade do último commit (reusa lógica de `OpenLoop.last_commit`).
- **merged** — branch está mergeada na default? (reusa set `--merged` do scanner).
- **state** — `clean` ou `dirty`: `git -C <worktree> status --porcelain` vazio = clean.
- **verdict** — derivado dos sinais (tabela abaixo).

### Regra de veredito

| Verdict | Condição | Significado |
|---|---|---|
| `prunable` | git marca `prunable` (dir sumiu / órfã) | `git worktree prune` resolve. Precede os demais. |
| `active` | state=dirty | Mudança não commitada. Trabalho vivo. Nunca sugerir apagar. |
| `deletable` | merged na default **e** clean | Fechou. Lixo no disco. Comando de limpeza oferecido. |
| `cold` | não-merged **e** clean | Sem mudança, ainda não mergeada. Candidata a revisar. Sem comando automático. |

Ordem de avaliação é de cima pra baixo (primeira que casa vence). **Sem threshold
de tempo no veredito** — `idle` é só coluna e chave de ordenação, não entra na regra.
Evita magic number.

Worktree principal (checkout da default) e bare repos: marcadas como `home` /
omitidas, nunca `deletable`. Detached HEAD: sem branch → sem merged → `verdict`
fica `active` por segurança (nunca sugere apagar algo sem branch claro).

### Saída

Tabela ASCII, ordenada por: `deletable`/`prunable` no topo (ação óbvia primeiro),
depois por idle decrescente.

```
WORKTREE            BRANCH       IDLE  MERGED  STATE  VERDICT
my-app/fix-bug      fix/bug       8d   yes     clean  deletable
api/spike-redis     spike/redis   40d  no      clean  cold
my-app/feat-login   feat/login    12d  no      dirty  active
```

`WORKTREE` é `repo_name/<basename-da-worktree>` (curto, legível). Caminho completo
aparece só no bloco de comandos.

Depois da tabela, bloco copiável só pras `deletable` / `prunable`:

```
# 2 worktrees deletable. Copy to clean up:
git -C /home/me/repo/my-app worktree remove fix-bug && git -C /home/me/repo/my-app branch -d fix/bug
git -C /home/me/repo/api worktree prune
```

Ordem do comando importa: `worktree remove` **antes** de `branch -d` (não dá pra
deletar branch enquanto checked out numa worktree).

Sem worktrees apagáveis: imprime linha tipo `# nothing to clean up.`.

`--json`: (se a flag global já existir no momento) emite a lista estruturada;
caso contrário, fica fora de escopo desta feature.

## 4. Comando `loops completions <shell>`

`clap_complete::generate` imprime o script de completion no stdout pro shell
pedido (`bash` / `zsh` / `fish`). Sem efeito colateral — usuário redireciona pra
onde o shell carrega.

Doc em `features.md` mostra o install por shell, ex:
```bash
loops completions zsh > ~/.zfunc/_loops
```

## 5. Migração para inglês

Todos os 10 arquivos em `src/` têm strings PT voltadas ao usuário (outputs e
erros). Migração é mecânica mas ampla:

- **Outputs** (`output.rs`, `cli.rs`, cabeçalhos de tabela, `features.md` examples).
- **Mensagens de erro** (`anyhow` contexts, `bail!`, `eprintln!` de warning) em todos
  os módulos (`scanner`, `config`, `distill`, `cache`, `ignores`, `sessions/*`, `main`).
- **Docs:** `features.md`, `configuration.md`, `setup.md` reescritos com exemplos EN.
- **`CLAUDE.md`:** trocar a regra "mensagens de erro em PT" por "mensagens de erro em
  EN, acionáveis". Manter o resto do arquivo em PT (é guia interno do autor).
- **Comentários de código e nomes de teste:** ficam em PT (não são voltados ao usuário;
  fora de escopo pra evitar churn desnecessário).

Princípio de tolerância de parsing e mensagens acionáveis seguem iguais — só muda o
idioma da string.

## 6. Arquitetura

- **Novo `src/worktrees.rs`** — não polui `scanner.rs`. Exporta algo como
  `struct Worktree { repo_name, repo_path, worktree_path, branch: Option<String>,
  head_sha, last_commit, merged, dirty, prunable, is_main }` e
  `fn worktrees(repo: &Path) -> Result<Vec<Worktree>>` + `fn scan_worktrees(roots) ->
  (Vec<Worktree>, Vec<String>)` no mesmo molde de `scanner::scan`.
- Reusa `scanner::git`, `scanner::default_branch`, `scanner::find_repos` e a lógica
  do set `--merged`.
- **Veredito** numa função pura testável (`fn verdict(&Worktree) -> Verdict`).
- **CLI** (`cli.rs`): adiciona subcomandos `Worktrees`/`Wt` e `Completions { shell }`.
  Camada fina; orquestração coberta por `tests/cli.rs`.
- **Output** (`output.rs`): função de formatação da tabela de worktrees + bloco de
  comandos, ASCII-only.

## 7. Casos de borda

- Worktree principal / bare → `home`, nunca apagável.
- Detached HEAD → sem branch → `active` (seguro).
- `prunable` (dir sumiu) → veredito `prunable`, comando `git worktree prune`.
- Branch checked out não deletável → ordem `remove` antes de `branch -d` resolve.
- Repo sem worktree extra (só o principal) → tabela com 1 linha `home` ou vazia.
- Falha de git em repo individual → warning, nunca aborta (igual `scan`).

## 8. Testes

Repos git reais em tempdir (`testutil`), agora com helper pra criar worktree real
(`git worktree add`). Casos cobertos:

- `deletable`: branch mergeada + worktree clean.
- `active` por dirty: worktree com arquivo não commitado.
- `cold`: branch não-merged, clean, antiga.
- `detached`: worktree em detached HEAD → `active`.
- principal não vira `deletable`.
- `verdict()` puro: tabela de combinações de sinais.
- `tests/cli.rs`: `loops worktrees` imprime tabela; `loops completions zsh` imprime
  script não-vazio.

Gate de cobertura 70% (core 85%) mantido.

## 9. Definition of Done

- [ ] `loops worktrees` / `loops wt` funcional, output ASCII EN, ordenado, bloco de comandos.
- [ ] `loops completions <bash|zsh|fish>` gera script.
- [ ] CLI inteira (outputs + erros) em inglês.
- [ ] `features.md`, `configuration.md`, `setup.md` atualizados em EN.
- [ ] Regra de idioma de erro no `CLAUDE.md` atualizada.
- [ ] Testes novos passando; cobertura no gate.
- [ ] `just lint` (clippy -D warnings) e `just fmt` limpos.
- [ ] CHANGELOG atualizado.
