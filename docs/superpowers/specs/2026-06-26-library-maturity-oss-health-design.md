# open-loops — Spec: maturidade da lib + saúde OSS (WAVE 4)

- **Data:** 2026-06-26
- **Status:** rascunho — derivado de brainstorm; aguardando revisão do autor
- **Produto:** API pública da lib, testes, observabilidade, arquivos de comunidade
- **Depende de:**
  - `docs/architecture/09-build-ci-release.md` (WAVE 1, ex-ci-hardening-design) — matriz cross-OS, `--locked`,
    cargo-deny; erros tipados e proptest são desenhados para falhar cedo nessa
    matriz, não só no E2E.
  - `docs/architecture/09-build-ci-release.md` (WAVE 2/3, ex-release-completeness-automation-design) —
    release-plz landado; badge crates.io assume versão publicada automaticamente.

> **Regra dura:** migração de erros (§4.1) **termina** antes de considerar WAVE 4
> fechada — sem alias `anyhow::Result` deprecated na API pública. PRs incrementais
> por módulo são permitidos; estado misto na superfície pública não.

---

## 1. Problema

Quatro lacunas de maturidade, ortogonais ao CI e ao release, mas **reforçadas**
por eles:

**A. API pública com `anyhow` (maior ponto).** O crate define `[lib]` e publica no
crates.io, mas funções `pub` retornam `anyhow::Result`. Consumidores não conseguem
`match` em erros específicos; `anyhow` vaza na API. O audit de higiene marcou
"Tipagem: 0" porque não avalia design de erro — o gap é real.

**B. Testes: cobertura alta, profundidade baixa.** ~95% de linhas (audit
2026-06-18), E2E excelente (`tests/cli.rs`, git real), mas:
- asserts frágeis (`err.to_string().contains(...)`) — **quebram entre SOs** quando
  o CI cross-OS (WAVE 1) expõe diferenças de path/mensagem do git;
- `query.rs` sem propriedades formais (candidato natural a proptest);
- `distill.rs`: `build_prompt` com múltiplos excerpts e erros de IO não injetados.

**C. Observabilidade.** `main.rs` e módulos usam só `eprintln!`. Para ferramenta
que varre repos, faz shell-out ao git e invoca LLM, depurar com `--verbose` ou
`RUST_LOG` é essencial. ADR 0002 registrou shell-out; o audit de logging (2026-06-18)
anotou tracing como fora do MVP — hora de endereçar sem mudar o contrato
stdout/stderr do CLI.

**D. Saúde da comunidade.** Faltam os três arquivos que o GitHub destaca para OSS:
`CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`. Existem `AGENTS.md` e
templates de issue/PR, mas não sinalizam "aceito contribuição humana". README sem
badges (CI hardening já prevê alguns — consolidar aqui sem duplicar trabalho).

**Objetivo:** lib com erros tipados (`thiserror`), testes que sobrevivem à matriz
CI cross-OS, tracing atrás de `--verbose`/`RUST_LOG`, e arquivos de comunidade
mínimos — sem alterar comportamento funcional do CLI.

## 2. Escopo / decisões travadas

| Decisão | Valor travado | Justificativa |
|---|---|---|
| Estratégia de erro | **Opção A — migração completa**, PRs por módulo, **sem** alias `anyhow::Result` deprecated | Sem consumidores externos conhecidos; momento pré-2.0; alias deprecated estagna (rejeitado na brainstorm) |
| Forma dos erros | **Por domínio** (`QueryError`, `GitError`, `ConfigError`, …) + `OpenLoopsError` com `#[from]` na borda CLI | `match` granular para lib; CLI achata com `anyhow` |
| `anyhow` no crate | **`[dependencies]` do binário apenas** (via `cli`/`main`); removido das deps da lib ao fechar 4.1 | Contrato crates.io correto |
| Semver | **Minor bump** + nota explícita no CHANGELOG: *breaking: library error types* | Rigorous major é opcional; sem devedores, minor com nota basta |
| Proptest | Só **`query.rs`** nesta wave | Maior ROI; scanner parsers já têm tabela de casos |
| Gate cobertura core | **85% linhas** em `query.rs`, parsers puros de `scanner.rs` (`parse_worktree_porcelain`, `repo_name_from_common_dir`), `build_prompt` em `distill.rs` | Alinha com alvo 85% do core no `CLAUDE.md`; gate global 70% inalterado |
| Tracing | **`tracing` na lib**; **`tracing-subscriber` só no binário** | Lib emite events; binário configura filtro |
| `--verbose` | Seta `RUST_LOG=open_loops=debug` se usuário não definiu `RUST_LOG` | Composição com ecossistema Rust |
| Progresso humano | `info!` para fases (`scan`, `distill`); visível com `--verbose` ou `RUST_LOG=info` | stderr continua canal de progresso; stdout intacto para pipe |
| Community files | Contributor Covenant 2.1; CONTRIBUTING aponta para `AGENTS.md` | Humanos vs agentes sem duplicar arquitetura |
| Badges README | CI, crates.io, licença (MSRV badge fica com WAVE 1 se já landado) | Não duplicar se CI spec já adicionou — conferir ao implementar |

**Fora de escopo:** esconder módulos (`pub(crate)` em massa — Opção C rejeitada;
split de `scanner.rs` em crate interna de git; OIDC crates.io; mudança de
comportamento do CLI; novos subcomandos.

**Motivação explícita testes + CI:** erros tipados e proptest existem para que a
matriz `ubuntu/macos/windows` do WAVE 1 falhe em **segundos** em funções puras,
com asserts estáveis (`matches!`), em vez de depender só do E2E (~30s) com
string-matching de mensagens de erro dependentes de SO.

---

## 3. Design — 4.1 Error typing (`thiserror`)

### `src/error.rs` (novo)

Enums por domínio, todos com `thiserror::Error`:

```rust
// Esqueleto — detalhes ao implementar

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("idle needs a comparator, e.g. idle:>7d")]
    IdleMissingComparator,
    #[error("invalid duration '{0}' (expected e.g. 7d)")]
    InvalidDuration(String),
    #[error("reserved token '{0}' is not supported yet")]
    ReservedToken(String),
    // ...
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git {command} failed in {repo}: {stderr}")]
    CommandFailed {
        repo: PathBuf,
        command: String,
        stderr: String,
    },
    #[error("couldn't find the default branch in {repo}")]
    NoDefaultBranch { repo: PathBuf },
    // ...
}

#[derive(Debug, Error)]
pub enum OpenLoopsError {
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    // Io, Cache, Distill, Session, Ignore — conforme módulo
}
```

### Regras de migração

| Camada | `Result` |
|---|---|
| `query`, `scanner`, `config`, `cache`, `distill`, `sessions`, `ignores`, `worktrees` | `Result<T, DomainError>` ou `Result<T, OpenLoopsError>` |
| `cli.rs` | `anyhow::Result<()>` — `.map_err(Into::into)` ou `.context()` na borda |
| `main.rs` | `eprintln!("error: {e:#}")` inalterado em UX |

### PRs incrementais (todos verdes antes de merge)

| PR | Escopo |
|---|---|
| **4.1a** | `error.rs` + `query.rs`; `thiserror` em `[dependencies]` |
| **4.1b** | `scanner.rs` (`GitError` com `repo` + `command` + `stderr`) |
| **4.1c** | `config`, `cache`, `distill`, `sessions`, `ignores`, `worktrees` |
| **4.1d** | `cli.rs` achata; remover `anyhow` das deps da lib no `Cargo.toml` |

### Impacto em testes existentes

Substituir padrão frágil:

```rust
// Antes
assert!(err.to_string().contains("idle"));

// Depois
assert!(matches!(err, QueryError::IdleMissingComparator));
```

Para `GitError`, assertar variant + campos (`repo`, `command`), não substring de
stderr do git (varia por SO/locale).

---

## 4. Design — 4.2 Unit tests + proptest

### `query.rs` — proptest

Dev-dep: `proptest = "1"`.

Propriedades candidatas (implementar as que forem estáveis):

1. **`parse` nunca panics** para `String` arbitrária.
2. **Tokens desconhecidos viram bare terms** — `foo:bar` → term `"foo:bar"`, não erro.
3. **`parse_duration`** só aceita sufixo `m|h|d|w`; qualquer outro →
   `QueryError::InvalidDuration`.
4. **Monotonicidade de filtros** — `matches(c)` com `idle:>7d` implica
   `matches(c)` com `idle:>3d` para o mesmo candidato com `days_idle=10`.

Manter os 12 testes manuais existentes; proptest complementa, não substitui.

### `scanner.rs` — parsers puros

Já cobertos por tabela (`parse_worktree_porcelain`, `repo_name_from_common_dir`).
Adicionar só gaps de valor:
- linhas malformadas entre entries de worktree;
- paths Windows-style em input de parser (string pura — **roda igual em todos os SOs do CI**).

Funções que shell-out ao git mantêm testes com `testutil` + tempdir (padrão atual).

### `distill.rs`

- `build_prompt` com **dois+ excerpts** — assert de separadores entre sessões.
- `run_llm` com comando que falha (exit ≠ 0) → variant tipada, não string match.

IO injection para `write_all` no prompt: aceitar como **NICE** se exigir trait
mock pesado; documentar como lacuna residual se pular.

### CI

- `cargo test --locked` na matriz WAVE 1 executa unit + proptest em todos os SOs.
- Opcional: job `proptest` com `PROPTEST_CASES=10000` no Ubuntu apenas (NICE, não
  bloqueante) — regressões de propriedade sem inflar tempo da matriz.

---

## 5. Design — 4.3 Observabilidade (`tracing`)

### Dependências

```toml
[dependencies]
tracing = "0.1"

# apenas o binário precisa do subscriber — se cli/main forem o único consumidor,
# tracing-subscriber fica como dep direta do bin ou via feature no crate root
```

Abordagem: `tracing` em `[dependencies]` (lib emite); inicialização do subscriber
em `main.rs` antes de `cli::run_*`.

### Mapeamento de `eprintln!`

| Hoje | Depois |
|---|---|
| `eprintln!("scanning…")` | `tracing::info!(phase = "scan", "scanning git repositories")` |
| `eprintln!("warning: {w}")` | `tracing::warn!(%w)` |
| `eprintln!("distilling…")` | `tracing::info!(phase = "distill", "distilling")` |
| `eprintln!("error: {e:#}")` em `main` | manter — é saída user-facing final |

Subscriber default sem flags: nível `WARN` (só warnings). Com `--verbose` ou
`RUST_LOG=open_loops=debug`: spans e debug visíveis em stderr.

### CLI

Adicionar flag global `--verbose` no `struct Cli` (clap). Não quebra subcomandos.

### ADR

Atualizar nota em ADR 0002 ou criar ADR 0009: shell-out permanece; **logging
estruturado** passa a ser suportado (supersedes a decisão "só eprintln" do MVP).

---

## 6. Design — 4.4 Community health

### Arquivos novos

| Arquivo | Conteúdo mínimo |
|---|---|
| `CONTRIBUTING.md` | `just setup/test/lint/fmt`; Conventional Commits; abrir issue antes de PR grande; link `AGENTS.md` para mapa de arquitetura; código e mensagens de erro em EN |
| `CODE_OF_CONDUCT.md` | [Contributor Covenant 2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/) |
| `SECURITY.md` | Escopo: CLI local, sem servidor; como reportar (GitHub Security Advisory ou email); política de disclosure responsável; o que **não** é vulnerabilidade (conteúdo de sessão LLM no disco do usuário) |

### README

Badges no topo (se WAVE 1 ainda não landou os badges, adicionar aqui):

```markdown
[![CI](https://github.com/carvalhosauro/open-loops/actions/workflows/ci.yml/badge.svg)](...)
[![crates.io](https://img.shields.io/crates/v/open-loops.svg)](https://crates.io/crates/open-loops)
[![license](https://img.shields.io/crates/l/open-loops.svg)](...)
```

Link para `CONTRIBUTING.md` na seção Docs.

### GitHub

Settings → Community Standards passa a verde nos três itens.

---

## 7. Arquitetura / arquivos

- **`src/error.rs`** — **novo**. Enums `*Error` + `OpenLoopsError`.
- **`src/query.rs`** — `Result<_, QueryError>`.
- **`src/scanner.rs`** — `Result<_, GitError>` para shell-out; parsers puros sem mudança de assinatura onde já retornam tipos concretos.
- **`src/config.rs`**, **`src/cache.rs`**, **`src/distill.rs`**, **`src/sessions/`**, **`src/ignores.rs`**, **`src/worktrees.rs`** — erros de domínio.
- **`src/cli.rs`** — `anyhow::Result`; `--verbose`; migração de `eprintln!` → `tracing`.
- **`src/main.rs`** — init `tracing-subscriber`; UX de erro inalterada.
- **`Cargo.toml`** — `thiserror`, `tracing` em deps; `proptest` em dev-deps; `anyhow` removido de deps da lib (permanece para bin).
- **`CONTRIBUTING.md`**, **`CODE_OF_CONDUCT.md`**, **`SECURITY.md`** — **novos**.
- **`README.md`** — badges + link contributing.
- **`CLAUDE.md`** — seção desenvolvimento: erros tipados na lib, `--verbose`, gate 85% core.
- **Inalterados:** lógica de negócio do scan/distill/query, `release.yml`, `tests/cli.rs` (exceto asserts se necessário).

### No mapa (registro da decisão)

- **Decisão typed-errors + tracing** — **nova**, EN. A registrar na seção *Decisions*
  da camada `docs/architecture/` (o sistema ADR em `docs/decisions/` foi consolidado
  nessa camada). Registra: (a) lib expõe `thiserror`, binário usa `anyhow`;
  (b) motivação testes + CI cross-OS; (c) `tracing` com `--verbose`/`RUST_LOG`. Liga a este spec.

---

## 8. Casos de borda / riscos

- **PR 4.1 grande se não fatiado.** Mitigação: tabela de PRs §3; cada um verde na
  matriz CI.
- **`GitError::stderr` ainda varia por SO** — não assertar conteúdo de stderr em
  testes; assertar `command`, `repo`, variant. stderr só para display humano.
- **Proptest flake** — limitar inputs (`prop_filter_map`); `PROPTEST_CASES` default
  modesto no CI.
- **`tracing-subscriber` e testes** — init subscriber só em `main`; testes não
  devem precisar de subscriber (events no-op sem subscriber é OK no tracing).
- **Badges duplicados** — se WAVE 1 já adicionou badges ao README, §6 vira verificação
  não duplicação.
- **Breaking change na lib sem consumidores** — documentar no CHANGELOG; se alguém
  depender de `open_loops::scanner::open_loops` com `anyhow`, quebra na 4.1d.
- **Windows + proptest + paths** — testes de parser usam strings literais, não
  `PathBuf::display()`, para SO-agnosticismo.

---

## 9. Validação

- **4.1:** `cargo test --locked` verde na matriz CI; `cargo doc --no-deps` mostra
  `Result<T, QueryError>` etc., não `anyhow::Error`.
- **4.2:** `cargo test query::` inclui proptest; `cargo llvm-cov` ≥ 85% nas funções
  core listadas em §2.
- **4.3:** `loops --verbose` mostra spans em stderr; `RUST_LOG=open_loops=debug loops`
  sem `--verbose` funciona; pipe de stdout (`loops | wc -l`) inalterado.
- **4.4:** GitHub Community Standards verde; links em CONTRIBUTING/SECURITY resolvem.

---

## 10. Definition of Done

### 4.1 — Error typing
- [ ] `src/error.rs` com enums por domínio + `OpenLoopsError`.
- [ ] Todos os módulos `pub` retornam erros tipados (não `anyhow::Result`).
- [ ] `anyhow` removido de `[dependencies]` da lib; permanece na borda CLI.
- [ ] Testes migrados para `matches!` / variant asserts (não string de stderr).
- [ ] CHANGELOG: nota breaking na API da lib.

### 4.2 — Unit tests + proptest
- [ ] `proptest` em `query.rs` (≥ 3 propriedades estáveis).
- [ ] `build_prompt` com múltiplos excerpts testado.
- [ ] Gate ≥ 85% linhas em `query`, parsers puros de `scanner`, `build_prompt`.
- [ ] Testes de parser com paths Windows-style como strings puras.

### 4.3 — Tracing
- [ ] `tracing` na lib; subscriber em `main.rs`.
- [ ] Flag global `--verbose` no CLI.
- [ ] `eprintln!` de progresso/warning migrados para `tracing`.
- [ ] ADR 0009 criado.

### 4.4 — Community health
- [ ] `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md` na raiz.
- [ ] README: badges (CI, crates.io, license) + link para CONTRIBUTING.
- [ ] `CLAUDE.md` atualizado (erros, verbose, cobertura core).

### Geral
- [ ] WAVE 1 e WAVE 2/3 landados (pré-requisito).
- [ ] CI verde em ubuntu + macos + windows após cada PR da série 4.1a–d.
