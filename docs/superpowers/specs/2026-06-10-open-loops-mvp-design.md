# open-loops — Spec do MVP

- **Data:** 2026-06-10
- **Status:** validado em brainstorming; aguardando revisão final do autor
- **Produto:** CLI `loops` — recuperação de contexto de trabalhos pausados

## 1. Definição precisa do problema

Desenvolvedor solo trabalha em múltiplos projetos, branches e sessões de IA em paralelo. Trabalhos iniciados pausam por dias ou semanas. Ao retomar, o custo dominante **não é entender o código** — é recuperar o contexto:

- Por que essa branch existe?
- O que já foi feito e o que falta?
- Qual era o próximo passo?
- Vale a pena continuar?

Esse contexto **já existe**, espalhado em sessões de IA (`~/.claude/projects/`) e no git. O problema é o custo de garimpo: 10+ minutos de busca manual por sessões antigas, sem garantia de achar. Esse custo causa retomadas lentas e **abandono de trabalho válido**.

**Evidência (episódio real, 2026-06-10):** issue "Em progresso" no Linear → checkout na branch → código compreensível, mas sem certeza do que faltava → reabriu sessão antiga de IA manualmente → 10+ min até recontextualizar. O autor relata trabalhos abandonados exclusivamente pelo custo de recuperação de contexto.

**Hipótese principal revisada durante a descoberta:** a dor central é **retomada de contexto**, não visibilidade. O inventário de trabalhos abertos é o gatilho de uso; a retomada é o valor.

## 2. Segmentação de usuários

- **MVP:** o próprio autor (dogfood). Perfil: dev que usa Claude Code intensamente, multi-repo, sem disciplina de issues (Linear apenas no trabalho, com cobertura fraca — maioria das branches não tem chamado).
- **Pós-validação:** devs solo com fluxo IA-intensivo e múltiplos contextos paralelos. Não especificar antes de validar com o usuário 1.

## 3. Jobs To Be Done

- **Principal:** "Quando volto a uma branch pausada, quero recuperar estado e próximo passo em menos de 60 segundos, sem ter documentado nada previamente."
- **Secundário:** "Quando decido no que trabalhar, quero ver tudo que iniciei e não terminei, em todos os projetos, ordenado por tempo parado."

## 4. Fluxo atual do usuário (sem a ferramenta)

1. Lembra (ou tropeça via Linear/git) de um trabalho parado.
2. Faz checkout da branch; lê diff e log; entende o código, mas não o estado do trabalho.
3. Garimpa sessões antigas de IA manualmente procurando "onde parei".
4. Após ~10+ min recontextualiza — ou desiste e abandona o trabalho.

## 5. Principais dores e momentos de fricção

| Momento | Dor |
|---|---|
| Descoberta | Nenhuma visão cross-repo do que está aberto; depende de memória ou sorte |
| Retomada | Estado e próximo passo não estão estruturados em lugar nenhum |
| Decisão | Sem saber o custo de retomar, abandona trabalho válido |

## 6. Hipóteses de solução debatidas

| Abordagem | Mecânica | Trade-off | Decisão |
|---|---|---|---|
| **Pull** (scan sob demanda) | Varre git + sessões de IA na hora da retomada; LLM destila | Zero captura, funciona retroativamente; retomada fria ~30-60s | **MVP** |
| **Push** (checkpoint via hook) | Hook no fim da sessão grava snapshot | Contexto fresco, leitura instantânea; só funciona dali em diante | Fase 2 |
| **Híbrido** | Snapshot quando existe; fallback no pull | Melhor dos dois; mais peças | Fase 2 |

Racional do faseamento: pull-only é shippável em dias e valida a hipótese central ("retomada <60s sem escrever nada") antes de construir infraestrutura de captura.

## 7. Menor MVP possível

**Unidade de trabalho:** branch não mergeada com commits (esqueleto, detecção 100% automática) + sessões de IA e git da branch (carne, fontes de contexto). Branch é proxy de indexação, não a entidade conceitual.

**CLI `loops` em Rust**, binário único, estado 100% fora dos repositórios (`~/.open-loops/`).

### Comandos

```
loops                      # inventário: branches não mergeadas em todos os repos
                           # configurados, com idade ("parado há 12d"), ordenado
                           # por staleness. Sem LLM. Meta: <5s.

loops resume <repo/branch> # retomada (resolução fuzzy de nome): destila contexto e
                           # responde POR QUÊ / FEITO / FALTA / PRÓXIMO PASSO.
                           # Meta: <60s frio, <1s cacheado.

loops init                 # registra raízes de repos (ex: ~/repo) no config

loops ignore <repo/branch> # descarta loop morto da lista (decisão "não vale continuar")
```

### Arquitetura

| Módulo | Responsabilidade |
|---|---|
| `cli` | clap; parsing dos 4 comandos |
| `config` | lê/grava `~/.open-loops/config.toml` (raízes, comando LLM, path das sessões) |
| `scanner` | varre raízes, acha repos git, lista branches não mergeadas + idade + ahead/behind |
| `sessions` | localiza sessões do Claude Code (`~/.claude/projects/<path-encoded>/*.jsonl`), filtra relevância, extrai trechos |
| `distill` | monta prompt (git log + diffstat + trechos de sessão), chama LLM, estrutura resposta |
| `cache` | `~/.open-loops/cache/<repo>/<branch>@<head-sha>.md`; HEAD novo invalida sozinho |
| `output` | render: tabela (inventário) e markdown (retomada) |

### Decisões técnicas

- **Git via shell-out** ao binário `git` (não `git2`/`gix`): simples, debugável, perf irrelevante (gargalo é o LLM).
- **LLM via comando configurável**, default `claude -p` (headless; usa assinatura existente, zero API key nova). Injeção de dependência: testes substituem por script fake; usuários de outros CLIs de LLM trocam via config.
- **Relevância sessão→branch (heurística MVP):** janela de tempo sobrepondo os commits da branch + grep do nome da branch no conteúdo da sessão. Sabidamente imprecisa — risco mitigado em §10 e candidata a experimento (ver Protocolo de Experimentos).
- **Truncamento:** últimos 50 KB de cada uma das até 3 sessões mais relevantes (defaults configuráveis) — o fim da conversa concentra o "onde parei".

### Fluxos

- **`loops`:** config → scanner em paralelo por repo → remove ignorados → ordena por staleness → tabela.
- **`loops resume`:** resolve branch (fuzzy) → cache hit? imprime → miss: scanner + sessions → distill via LLM → grava cache → imprime as 4 seções + fontes utilizadas.

### Tratamento de erros

Princípio: erro sempre com contexto acionável; nunca abortar a operação inteira por falha parcial.

- Repo sem sessões de IA → destila só do git; avisa "confiança baixa: sem sessões de IA".
- `.jsonl` corrompido/formato inesperado → pula o arquivo, emite warning, continua.
- `claude` CLI ausente → mensagem com instrução de instalação + como configurar comando alternativo.
- Raiz configurada inexistente → warning e segue com as demais.

### Testes e CI

- Unit tests por módulo: repos git temporários criados no próprio teste; `.jsonl` sintéticos como fixtures.
- `distill` testado com LLM fake injetado via config.
- CI (GitHub Actions): `fmt` + `clippy` + `test` + cobertura (gate em §DX) em todo push.
- Release por tag com **cargo-dist**: GitHub Releases (Linux/macOS/Windows), brew tap, install.sh, `cargo install`. Instalação em 1 comando.

### DX e qualidade

- **Pre-commit hooks** via [lefthook](https://github.com/evilmartians/lefthook) (binário único, config YAML versionada no repo, instalado no `just setup`):
  - `pre-commit`: `cargo fmt --check` + `cargo clippy -- -D warnings` nos arquivos staged.
  - `commit-msg`: valida formato Conventional Commits.
  - Testes completos ficam no CI, não no hook — commit deve continuar rápido (<5s).
- **Cobertura de testes** com `cargo-llvm-cov`, medida no CI:
  - Gate global: ≥70% de linhas (CI falha abaixo disso).
  - Alvo para módulos core (`scanner`, `sessions`, `distill`, `cache`): ≥85%.
  - `cli` e `output` podem ficar abaixo do alvo (camada fina, testada via testes de integração).
  - PRs não podem reduzir a cobertura global.
- **Task runner**: `justfile` com `just setup` (instala hooks + toolchain), `just test`, `just lint`, `just fmt`, `just cov`. Um comando por intenção — agente ou humano não precisa decorar flags.
- **Reprodutibilidade**: `rust-toolchain.toml` pinando a versão do Rust; `.editorconfig`.
- **Repo agente-friendly**: `AGENTS.md`/`CLAUDE.md` na raiz com mapa do projeto, comandos do justfile e convenções — primeiro arquivo que um agente lê.

### Postura open-source

Nasce como ferramenta pessoal, mas estruturada desde o dia 1 para virar projeto público — o custo de nascer certo é baixo; o de migrar depois, alto.

- **Alta superfície de distribuição é essencial:** os N usuários precisam instalar **e atualizar** com comodidade. cargo-dist publica em todos os canais (GitHub Releases multi-OS, brew tap, install.sh, `cargo install`); atualização acontece pelo mesmo canal da instalação (`brew upgrade`, re-rodar install.sh, `cargo install --force`). Nenhum canal exige passo manual além de 1 comando.
- **Release disciplinado:** gatilho por tag (`vX.Y.Z`, SemVer) → CI gera binários, changelog automático (a partir dos Conventional Commits) e release notes na GitHub Release. Sem release manual.
- **Padrões de contribuição:** templates de PR e issue em `.github/` (bug report, feature request, template de PR com checklist de testes/cobertura); PRs seguem Conventional Commits — mesmo enquanto o único contribuidor for o autor, os templates documentam o padrão para quem chegar.
- **Licença open-source:** dual **MIT OR Apache-2.0** (convenção do ecossistema Rust — máxima compatibilidade para adoção e contribuição). Arquivos `LICENSE-MIT` e `LICENSE-APACHE` na raiz desde o primeiro commit de código.

### Documentação

Essencial, com dupla audiência: humanos **e** agentes de IA. Regra de ouro: sempre que possível, exemplo concreto com bloco bash/zsh **colável no terminal** — nunca descrição abstrata quando um comando executável resolve.

- **README.md:** instalação (1 comando por canal: brew, install.sh, cargo), quickstart (`loops init` → `loops` → `loops resume`, com output real de exemplo), e link para `docs/`.
- **`docs/setup.md`:** setup inicial completo — instalação, `loops init`, estrutura do `config.toml` comentada campo a campo.
- **`docs/features.md`:** cada comando com exemplo de invocação + output esperado.
- **`docs/configuration.md`:** todas as chaves do config com default, tipo e exemplo (incl. troca do comando LLM).
- **`AGENTS.md`/`CLAUDE.md`:** mapa do repo para agentes (já em §DX) — aponta para os docs acima como fonte de verdade; sem duplicar conteúdo.
- **Docs são parte do Definition of Done:** funcionalidade nova ou mudança de config sem doc atualizada não mergeia (checklist do template de PR).
- Decisões arquiteturais relevantes registradas em `docs/decisions/` (formato ADR curto) — exigência do post de boas práticas open-source na era LLM.

### Princípios de engenharia

O repositório nasce otimizado para agentes de IA trabalharem nele, com base em:

- [Boas práticas de projetos open-source na era LLM — Akita](https://akitaonrails.com/2026/05/30/boas-praticas-projetos-codigo-aberto-llm-o-minimo): instalação em 1 comando; compile once, repackage many; CI desde o dia 1; release por tag com changelog; README + `docs/` com decisões arquiteturais.
- [Clean code para agentes de IA — Akita](https://akitaonrails.com/2026/04/20/clean-code-para-agentes-de-ia/): arquivos e funções pequenos; responsabilidade única; nomes únicos e descritivos; comentários de "porquê"; tipos explícitos; nesting raso; erros com contexto; estrutura de diretórios previsível; injeção de dependência.

### Protocolo de experimentos

Quando uma decisão técnica tiver incerteza relevante (candidatas já conhecidas: heurística sessão→branch, estratégia de truncamento, prompt de destilação):

1. Implementar abordagens alternativas em `experiments/<tema>/<abordagem>/`.
2. Medir contra critério objetivo definido antes do experimento.
3. Registrar comparativo + trade-offs em `docs/experiments/<tema>.md`.
4. Promover a vencedora para `src/`; `experiments/` fica fora do build de release.

## 8. Fora de escopo do MVP

Hooks de checkpoint, skills de Claude Code, snapshots push, TUI, integração Linear, memória técnica detalhada, decisões arquiteturais do usuário, documentação automática, gestão completa de tarefas, gestão de conhecimento, multi-usuário.

## 9. Critérios de sucesso (30 dias de dogfood)

1. Retomada fria em <60s; cacheada em <1s.
2. Inventário cross-repo em <5s.
3. Autor usa `loops` ≥3x/semana sem se forçar.
4. ≥1 trabalho retomado que teria sido abandonado no fluxo antigo.
5. Zero arquivos criados dentro dos repositórios dos projetos.

## 10. Riscos e hipóteses críticas

**Riscos:**

1. **Formato `.jsonl` das sessões muda sem aviso** (formato interno do Claude Code, não é API pública) → parser tolerante + fixtures versionadas + degradação para git-only.
2. **Heurística sessão→branch erra e destila contexto errado** — pior que não ter contexto → output sempre lista as fontes utilizadas (sessões e commits) para o usuário auditar; tema prioritário do protocolo de experimentos.
3. **Destilação fria estoura 60s** → cache por HEAD + truncamento agressivo; medir desde o primeiro build.

**Hipóteses a validar no dogfood:**

- H1: retomada via destilação automática é confiável o suficiente para substituir o garimpo manual.
- H2: o inventário por branch cobre a maioria dos trabalhos reais (trabalho que nunca vira branch é minoria aceitável).
- H3: <60s frio é alcançável com `claude -p`.

## 11. Evoluções futuras

Apenas o que já foi debatido e aprovado como direção:

- **Fase 2 — Push + Híbrido:** hook (SessionEnd/Stop) grava snapshot no fim da sessão; `resume` usa snapshot quando existe, fallback no pull. Skill `/loops:resume` injetando o contexto destilado diretamente na nova sessão de IA.

Qualquer evolução além da Fase 2 será debatida em novo ciclo de brainstorming — intencionalmente não especulada aqui.
