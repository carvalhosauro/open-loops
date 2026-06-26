# open-loops — Spec: CI hardening (5 buracos)

- **Data:** 2026-06-25
- **Status:** rascunho — derivado de brainstorm de design; aguardando revisão do autor
- **Produto:** CI/CD — `.github/workflows/ci.yml` + novos arquivos de raiz/`.github/`
- **Escopo travado em brainstorm:** apenas os 5 buracos do `ci.yml`. Health files
  (CONTRIBUTING/CODE_OF_CONDUCT/SECURITY), release-plz, testes unit/proptest e
  `tracing` ficam **fora** — tratados depois, sobre esta base.

## 1. Problema

O `ci.yml` atual tem um job `check` (Ubuntu) e um `coverage` (Ubuntu). Cinco
lacunas, em ordem de impacto:

1. **Só testa em Linux.** `loops` faz shell-out ao git, varre filesystem e
   resolve paths de worktree — comportamento que diverge entre SOs (separadores,
   CRLF, git). O release **entrega** 4 alvos (`aarch64-apple-darwin`,
   `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
   via `dist-workspace.toml`), mas o CI valida só Ubuntu.
2. **Sem cache de build.** Nenhum job usa `Swatinem/rust-cache`; cada run
   recompila do zero.
3. **Sem `--locked`.** `clippy`/`test` rodam sem `--locked`; o CI pode passar com
   um `Cargo.lock` diferente do que o usuário recebe. (O `publish-crate.yml` já
   usa `--locked` — divergência entre CI e publish.)
4. **Sem checagem de supply-chain.** Para algo publicado em crates.io + Homebrew,
   não há varredura de advisories (RUSTSEC) nem de licenças.
5. **Sem automação de dependência e sem `concurrency`.** Deps frouxamente pinadas
   (`"1"`, `"0.4"`) entram sem PR de update; runs obsoletos de PR não são
   cancelados.

Lacuna correlata: `rust-toolchain.toml` pina `1.89`, mas o CI roda `@stable` —
um contrato de MSRV que nada verifica (falsa sensação).

**Objetivo:** fechar os 5 buracos + resolver a ambiguidade de MSRV, sem alterar a
lógica do CLI nem o pipeline de release (`release.yml`/cargo-dist) ou o
`publish-crate.yml`.

## 2. Escopo / decisões travadas

| Decisão | Valor travado | Justificativa |
|---|---|---|
| MSRV | **1.89 como contrato real.** Job `msrv` dedicado em `1.89`; `rust-toolchain.toml` fica. | Pin sem verificação é falsa promessa; users em distros antigas merecem garantia. |
| Matriz de teste | **`[ubuntu-latest, macos-latest, windows-latest]` × stable.** | Cobre Linux + macOS (arm) + Windows — os SOs onde path/git divergem. |
| mac-intel (macos-13) | **Fora da matriz.** | Único alvo entregue não testado: `x86_64-apple-darwin`. Mesma lib do mac-arm; arch raramente diverge em CLI de git. Continua **entregue** pelo release. Trade-off aceito (ver §5). |
| Supply-chain | **`cargo-deny`** (advisories + licenças + bans) via `deny.toml`. | Mais completo que `cargo-audit`; valida a dupla licença MIT/Apache dos deps. |
| `--locked` | Em `clippy`, `test`, `msrv` (check) e coverage. | Alinha CI ao que publish/usuário recebem. |
| Cache | `Swatinem/rust-cache@v2` em todo job que compila. | Ganho gratuito de tempo. |
| `concurrency` | `cancel-in-progress` por workflow+ref. | Mata runs de PR obsoletos. |
| Pin de actions | **SHA-pin** todas as `uses:` (`@<sha> # vX`). | CI carrega tokens de publish (crates.io/Homebrew); tag móvel (`@v4`) é vetor de supply-chain — maintainer comprometido repointa a tag. Dependabot (github-actions) mantém os SHAs. |
| Env global de CI | Bloco `env:` no topo do workflow: `CARGO_INCREMENTAL: 0`, `CARGO_NET_RETRY: 10`, `RUSTUP_MAX_RETRIES: 10`, `RUST_BACKTRACE: short`, `RUSTFLAGS: "-D warnings"`. | Mata flakes de rede em download de crate/toolchain, encolhe o cache (artefatos incrementais são inúteis em CI limpo), backtrace legível nas falhas de teste git-tempdir. `RUSTFLAGS: "-D warnings"` é **global** (mais amplo que só clippy — vale para `check`/`test` também): adotado deliberadamente. |
| cargo-deny split | Matriz `advisories` (não-bloqueante) vs `bans licenses sources` (bloqueante). | Advisory RUSTSEC recém-publicado não pode barrar um PR alheio; política de licença/ban/source continua obrigatória. |

As três últimas linhas (pin de actions, env global, split do cargo-deny) e as
correções de schema do `deny.toml` (§4) vêm da análise dos repos de referência
(`starship/starship`, `EmbarkStudios/cargo-deny`) — **WAVE 1**. O ponto correlato
de **publish via OIDC** em crates.io (eliminar `CARGO_REGISTRY_TOKEN`) fica **fora**
deste spec por decisão. WAVE 2/3 (completude do artefato de release + `release-plz`)
têm spec própria: `2026-06-26-release-completeness-automation-design.md`.

**Fora de escopo (depois, sobre esta base):** health files, README de
contribuição, `release-plz`, testes unit/proptest em `scanner`/`query`/`distill`,
`tracing`/`--verbose`, publish OIDC em crates.io.

## 3. Design — `ci.yml`

Topo do workflow:

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_INCREMENTAL: 0
  CARGO_NET_RETRY: 10
  RUSTUP_MAX_RETRIES: 10
  RUST_BACKTRACE: short
  RUSTFLAGS: "-D warnings"
```

> **SHA-pin:** todos os `uses:` abaixo são pinados ao commit SHA completo
> (`@<sha> # vX.Y.Z`); as versões aparecem como tag no spec só por legibilidade.
> Dependabot (`github-actions`, §4) mantém os SHAs atualizados.

Jobs:

**`fmt`** (ubuntu-latest) — sem cache (não compila):
- `dtolnay/rust-toolchain@stable` com `rustfmt`
- `cargo fmt --check`

**`test`** (matriz) — o coração da mudança:
- `strategy.matrix.os: [ubuntu-latest, macos-latest, windows-latest]`
- `runs-on: ${{ matrix.os }}`
- `fail-fast: false` (um SO quebrar não cancela os outros — quero ver todos)
- `dtolnay/rust-toolchain@stable` com `clippy`
- `Swatinem/rust-cache@v2`
- `cargo clippy --all-targets --locked -- -D warnings` (na matriz: pega código
  `#[cfg(...)]`/path específico de SO)
- `cargo test --locked`

**`msrv`** (ubuntu-latest) — honra o contrato 1.89:
- `dtolnay/rust-toolchain@1.89`
- `Swatinem/rust-cache@v2`
- `cargo check --locked --all-targets` — *check*, não *test*: dev-deps podem
  exigir Rust mais novo; o contrato é que **o crate** compila em 1.89.

**`coverage`** (ubuntu-latest) — mantém o gate, ganha cache:
- `dtolnay/rust-toolchain@stable`
- `Swatinem/rust-cache@v2`
- `taiki-e/install-action@cargo-llvm-cov`
- `cargo llvm-cov --fail-under-lines 70`

**`audit`** (ubuntu-latest) — supply-chain, **matriz dividida**:
- `strategy.matrix.checks: [advisories, "bans licenses sources"]`; `fail-fast: false`
- `continue-on-error: ${{ matrix.checks == 'advisories' }}` — advisory RUSTSEC novo
  reporta sem bloquear merge; licença/ban/source seguem **obrigatórios** (resolve §5).
- `EmbarkStudios/cargo-deny-action@v2` com:
  - `rust-version: stable` — o default da action é um `1.71.0` obsoleto; sobrescrever.
  - `command: check ${{ matrix.checks }}`
  - `arguments: --all-features --locked` — audita o `Cargo.lock` versionado.
- (gerencia cache próprio)

## 4. Arquitetura / arquivos

- **`.github/workflows/ci.yml`** — reescrito conforme §3. `fmt` separado de
  `test` porque formatação é OS-independente (rodar 4× é desperdício); `clippy`
  fica na matriz porque lints `cfg`-gated divergem por SO. Todos os `uses:`
  **SHA-pinados** (`@<sha> # vX`); bloco `env:` global no topo (§3).
- **`deny.toml`** (raiz) — **novo**. Schema **cargo-deny 0.16+** (o 0.16 removeu
  as keys `vulnerability`/`unsound`/`notice`/`severity-threshold`):
  - `[graph]`: `targets` = só os 4 alvos entregues (do `dist-workspace.toml`);
    `all-features = true`.
  - `[advisories]`: vulnerabilidades/unsound/notice são **sempre** erro (não há
    key no 0.16+); `unmaintained = "workspace"` (só falha em unmaintained de dep
    direto); `yanked = "deny"`; `ignore = []` (cada entrada futura com comentário
    justificando).
  - `[licenses]`: allow-list **explícita** alinhada à árvore real — `MIT`,
    `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `Unicode-3.0`, `ISC`,
    `BSD-3-Clause`, `Zlib`. **Achado verificado: `dirs → option-ext` é MPL-2.0**
    — único license fora do par MIT/Apache. Tratar com `exceptions` escopada
    (`{ allow = ["MPL-2.0"], crate = "option-ext" }`) em vez de afrouxar o
    `allow` global. `confidence-threshold = 0.93`.
  - `[bans]`: `multiple-versions = "warn"`; `wildcards = "deny"`.
  - `[sources]`: `unknown-registry = "deny"`, `unknown-git = "deny"` (só crates.io).
- **`.github/dependabot.yml`** — **novo**. Dois ecossistemas, semanal:
  - `cargo` (`/`) e `github-actions` (`/`).
  - Agrupamento de minor/patch num PR só, para cortar ruído.
- **`rust-toolchain.toml`** — **inalterado** (`1.89`); agora lastreado pelo job
  `msrv`.
- **`README.md`** — badges: status do CI, versão crates.io, MSRV 1.89, licença.
- **Inalterados (neste spec):** `release.yml`, lógica do CLI. `publish-crate.yml` e fluxo
  manual de changelog foram substituídos por release-plz em WAVE 2/3 (ADR 0007).

### No mapa (registro da decisão)

- **ADR `docs/decisions/0006-ci-msrv-cross-os.md`** — **novo**, EN (estilo dos
  ADRs existentes). Registra: (a) MSRV = 1.89 como contrato verificado; (b)
  política de matriz = Linux + macOS-arm + Windows testados no CI; mac-intel
  (`x86_64-apple-darwin`) entregue mas não testado (com a justificativa). Liga a
  este spec.
- **`CLAUDE.md`** — nota curta na seção de desenvolvimento: matriz de CI + MSRV
  1.89, para orientar agentes.

## 5. Casos de borda / riscos

- **`--locked` exige `Cargo.lock` atual e versionado.** Provável já estar (o
  `publish --locked` depende dele). Se houver drift, o primeiro run falha com
  `Cargo.lock needs updating` — desejável, expõe o problema. Validar antes de
  mergear.
- **Windows na matriz pode expor falhas reais (heads-up de escopo).** É o ponto
  da matriz, mas `tests/cli.rs` cria repos git em tempdir e o adapter de sessão
  codifica o path do cwd (`~/.claude/projects/<cwd-encodado>/`) — separadores,
  CRLF e drive-letter divergem no Windows. Habilitar Windows **pode revelar
  falhas de teste que precisam ser corrigidas no código**, não só infra de CI.
  Corrigir essas falhas faz parte deste trabalho; se forem grandes/profundas, o
  spec é re-decomposto. Não tratar `windows-latest` vermelho como "esperado".
- **mac-intel não testado.** Único alvo entregue sem cobertura no CI:
  `x86_64-apple-darwin`. `macos-latest` é arm64 nos runners GitHub (cobre
  `aarch64-apple-darwin`); o Intel fica descoberto por decisão. Risco residual:
  bug arch-específico só apareceria no binário entregue. O ADR 0006 documenta a
  escolha.
- **`deny.toml` licenças** — a allow-list precisa bater com a árvore real; um dep
  com licença fora da lista quebra o job `audit`. **Offender já mapeado:**
  `dirs → option-ext` é MPL-2.0 — tratado por `exceptions` escopada (§4), não
  afrouxando o `allow` global. Validar rodando `cargo deny check licenses` local
  ao montar o arquivo; se aparecer outro (ex.: `linux-raw-sys` via `rustix`),
  adicionar ao `allow` apenas após revisar.
- **Advisory RUSTSEC novo — agora mitigado pelo split (§3).** O job `audit` roda
  em matriz: `advisories` com `continue-on-error: true` (reporta sem bloquear
  merge) e `bans licenses sources` bloqueante. Assim um RUSTSEC recém-publicado
  sinaliza sem barrar um PR não relacionado; ainda assim, um `[advisories].ignore`
  pontual e documentado segue disponível para casos sem fix. Opcional: rodar
  `advisories` também num `schedule:` cron diário, para CVE novo aparecer mesmo
  sem commits.

## 6. Validação

- Após reescrever, abrir PR e confirmar **todos** os jobs verdes:
  `fmt`, `test (ubuntu-latest)`, `test (macos-latest)`, `test (windows-latest)`,
  `msrv`, `coverage`, `audit`.
- Confirmar que o cache popula no segundo run (tempo de `test` cai).
- Confirmar que `concurrency` cancela um run anterior ao dar push novo no mesmo
  PR.
- Confirmar que o job `audit` aparece como dois checks — `audit (advisories)` e
  `audit (bans licenses sources)` — e que só o segundo é bloqueante.
- Conferir que todos os `uses:` estão SHA-pinados (grep por `uses:.*@v` não deve
  achar tag móvel sem SHA).
- `dependabot.yml`: validar sintaxe (GitHub valida ao mergear; opcionalmente
  conferir na aba Insights → Dependency graph → Dependabot).
- Sem suíte de teste nova: a mudança é de infra de CI; a verificação é o próprio
  CI rodando verde nos dois SOs.

## 7. Definition of Done

- [ ] `ci.yml` reescrito: `concurrency` + bloco `env:` global + jobs `fmt`,
      `test` (matriz ubuntu+macos+windows, `fail-fast: false`), `msrv` (1.89,
      check), `coverage` (com cache), `audit` (cargo-deny, matriz dividida).
- [ ] **Todos os `uses:` SHA-pinados** (`@<sha> # vX`). _(WAVE 1.1)_
- [ ] **Bloco `env:` global**: `CARGO_INCREMENTAL`, `CARGO_NET_RETRY`,
      `RUSTUP_MAX_RETRIES`, `RUST_BACKTRACE`, `RUSTFLAGS: "-D warnings"`. _(WAVE 1.2)_
- [ ] **`audit` em matriz dividida**: `advisories` (`continue-on-error`) vs
      `bans licenses sources` (bloqueante); `rust-version: stable` na action;
      `arguments: --all-features --locked`. _(WAVE 1.3)_
- [ ] **`deny.toml` schema 0.16+**: `[graph].targets`, advisories sempre-erro,
      `unmaintained="workspace"`, exceção MPL-2.0 escopada para `option-ext`. _(WAVE 1.4)_
- [ ] `test (windows-latest)` verde — falhas de path/CRLF/session expostas pelo
      Windows corrigidas no código.
- [ ] `--locked` em clippy/test/msrv/coverage.
- [ ] `Swatinem/rust-cache@v2` em test/msrv/coverage.
- [ ] `deny.toml` na raiz; `cargo deny check` passa localmente.
- [ ] `.github/dependabot.yml` (cargo + github-actions, semanal, agrupado).
- [ ] README com badges (CI, crates.io, MSRV, license).
- [ ] ADR `0006-ci-msrv-cross-os.md` criado.
- [ ] Nota de CI/MSRV em `CLAUDE.md`.
- [ ] PR com todos os jobs verdes nos dois SOs; cache populando.
- [ ] ~~CHANGELOG atualizado (git-cliff).~~ **Obsolete** — release-plz updates
      `CHANGELOG.md` on Release PR merge (see ADR
      [0007](docs/decisions/0007-release-plz-cargo-dist-split.md); WAVE 2/3).
