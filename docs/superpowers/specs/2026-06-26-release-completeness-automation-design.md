# open-loops — Spec: completude do artefato de release + automação (WAVE 2/3)

- **Data:** 2026-06-26
- **Status:** rascunho — derivado de análise de repos de referência
  (`BurntSushi/ripgrep`, `sharkdp/bat`, `sharkdp/fd`, `MarcoIeni/release-plz`);
  aguardando revisão do autor
- **Produto:** pipeline de release — `dist-workspace.toml`, `Cargo.toml`,
  `build.rs` (novo), `release-plz.toml` (novo), `.github/workflows/`
- **Depende de:** `2026-06-25-ci-hardening-design.md` (WAVE 1). Fazer **depois**
  daquela base — release-plz assume o CI endurecido e o `concurrency` como
  precedente.

> **Regra dura:** `release.yml` é **100% gerado pelo cargo-dist** — nunca editar à
> mão (é sobrescrito em `dist generate`). Toda mudança de empacotamento abaixo vai
> em `dist-workspace.toml` / `Cargo.toml` / `build.rs`, nunca no `release.yml`.

---

## 1. Problema

Dois buracos no pipeline de release, ortogonais ao CI:

**A. Artefato incompleto (WAVE 2).** O archive entregue pelo cargo-dist contém
**só o binário `loops`**. Comparado a ripgrep — que entrega binário + man page +
completions de 4 shells + README + todos os arquivos de licença — faltam:
1. **Shell completions no archive.** O CLI já gera (`loops completions <shell>`,
   `src/cli.rs:237`, via `clap_complete::generate`), mas a saída não vai no
   tarball. Usuário `brew install loops` hoje recebe **zero** tab-completion.
2. **Man page.** Não existe (`clap_mangen`/`roff`/`man` = zero hits no repo).
3. **README/LICENSE/CHANGELOG no archive** — projeto é dual-license MIT/Apache;
   se os `LICENSE-*` não vão no redistribuível, é gap de compliance. cargo-dist
   tem `auto-includes` ligado por default, mas **precisa ser verificado**.

**B. Metadados e release manual (WAVE 3).**
4. **`Cargo.toml` incompleto** (vs. bat/fd): faltam `rust-version`, `categories`,
   `keywords`, `readme`, `authors`. E **não há `[profile.release]`** — quem faz
   `cargo install loops` não pega LTO/strip (só o perfil `dist` tem `lto="thin"`).
5. **Release 100% manual:** bump de versão à mão + `just changelog` (git-cliff) +
   `git tag && push` + `publish-crate.yml`. 5 passos, fácil esquecer um.

**Objetivo:** archive completo e auto-suficiente; `Cargo.toml` publicável de
primeira; release de "5 passos manuais" para "merge de um PR" — sem tocar na
lógica do CLI e mantendo o cargo-dist como dono dos binários.

## 2. Escopo / decisões travadas

| Decisão | Valor travado | Justificativa |
|---|---|---|
| Geração de completions/man | **`build.rs` emite no `OUT_DIR`**, não subcomando-em-runtime para o release. Subcomando `loops completions` **fica** (uso local). | `build.rs` roda no **host** em compile-time → imune a cross-compile. A alternativa (rodar o binário recém-buildado p/ gerar) força a dança de qemu do ripgrep em alvos não-nativos. |
| Man page | **`clap_mangen`** (dev-dep), gerado pelo mesmo `build.rs`. | Padrão do ecossistema clap; pareia com as completions. |
| Bundling no archive | cargo-dist `include` aponta para os arquivos gerados + verificar `auto-includes` de LICENSE/README/CHANGELOG. | cargo-dist empacota o que `include` listar; não gera nada. |
| Metadados `Cargo.toml` | Adicionar `rust-version="1.89"`, `categories`, `keywords`, `readme`, `authors`. | Descoberta em crates.io + contrato MSRV explícito no manifesto. |
| `[profile.release]` | `lto="thin"`, `strip=true`. | `cargo install loops` deve render binário otimizado, não só o caminho `dist`. |
| Automação de release | **Adotar `release-plz` (parcial).** Ele assume bump + changelog + crates.io publish + tag. cargo-dist **continua** dono de binários/Homebrew/GitHub Release. | release-plz é feito p/ crate solo com Conventional Commits e **compõe** com cargo-dist; não compete. |
| Handoff release-plz → cargo-dist | **PAT fine-grained** (`RELEASE_PLZ_TOKEN`, Contents+PR RW) para empurrar a tag. | Tag empurrada com `GITHUB_TOKEN` **não** dispara outros workflows (anti-loop do GitHub) → `release.yml` nunca rodaria. |

**Fora de escopo:** WAVE 4 (lib de git / split de `scanner.rs` / sandbox de
config em teste / CONTRIBUTING) — tratada separadamente. Publish OIDC em crates.io
(WAVE 1.5) também fora. Code-signing, notarização, alvos cross-compile novos.

---

## 3. Design — WAVE 2 (artefato completo)

### `build.rs` (novo, raiz)

Gera completions e man page em compile-time, no `OUT_DIR`, reusando a definição
`clap` do CLI (exigir o `Command` num módulo compartilhável entre `build.rs` e
`src/`, ou reconstruir via `clap::CommandFactory`):

```rust
// build.rs (esqueleto)
use clap::CommandFactory;
use clap_complete::{generate_to, Shell};
use std::{env, path::PathBuf};

include!("src/cli_command.rs"); // o `Command`/derive compartilhado

fn main() {
    let outdir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut cmd = Cli::command();
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
        generate_to(shell, &mut cmd, "loops", &outdir).unwrap();
    }
    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf).unwrap();
    std::fs::write(outdir.join("loops.1"), buf).unwrap();
}
```

Dev-deps: `clap_complete` (já é dep), `clap_mangen` (novo, em `[build-dependencies]`).

> Refator pré-requisito: a definição `clap` precisa estar acessível ao `build.rs`.
> Hoje o derive vive em `src/cli.rs`; extrair o `struct Cli` para um arquivo
> `include!`-ável (ou crate interna) é parte deste trabalho. **Risco:** acoplar
> `build.rs` ao `src/` exige cuidado — `build.rs` não vê o crate compilado, só
> arquivos-fonte. Validar que o `include!` compila isolado.

### `dist-workspace.toml`

Listar os artefatos gerados no `include` para o cargo-dist empacotá-los. O `OUT_DIR`
é volátil; a abordagem robusta é o `build.rs` copiar (ou o cargo-dist apontar via
glob estável) para um diretório versionado de saída — definir o caminho exato ao
implementar e confirmar contra a doc do cargo-dist em uso (versão pinada `0.32.0`).

### Verificação de `auto-includes`

Baixar um tarball de release passado (`gh release download <tag>`) e inspecionar:
se `README.md`/`LICENSE-*`/`CHANGELOG.md` **não** estiverem lá, o `auto-includes`
do cargo-dist está off ou os nomes não batem — corrigir via `include` explícito.

---

## 4. Design — WAVE 3 (metadados + automação)

### `Cargo.toml`

```toml
[package]
# ...existentes...
rust-version = "1.89"          # contrato MSRV no manifesto (casa com rust-toolchain.toml)
authors = ["Gustavo Carvalho <...>"]
readme = "README.md"
categories = ["command-line-utilities", "development-tools"]
keywords = ["git", "worktree", "context", "ai", "agents"]

[profile.release]
lto = "thin"
strip = true
```

### `release-plz.toml` (novo, raiz)

```toml
[workspace]
publish = true                  # crates.io publish (substitui publish-crate.yml)
git_release_enable = false      # cargo-dist é dono do GitHub Release; não duplicar
git_tag_name = "v{{ version }}" # casa com o trigger do release.yml e do publish
changelog_update = true
# semver_check = false          # opcional: pular o custo de semver-check em CLI solo
```

O `cliff.toml` existente é **reusado as-is** — release-plz embute git-cliff e o
detecta automaticamente. `just changelog` vira preview local (não mais o caminho
de release).

### `.github/workflows/release-plz.yml` (novo)

```yaml
name: release-plz
on:
  push:
    branches: [main]
permissions:
  contents: write
  pull-requests: write
concurrency:
  group: release-plz-${{ github.ref }}
  cancel-in-progress: false       # release-plz exige; não cancelar release em curso
jobs:
  release-plz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<sha>   # v6 — SHA-pin (padrão WAVE 1)
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@<sha> # stable
      - name: Run release-plz
        uses: release-plz/action@<sha>   # v0.3
        env:
          GITHUB_TOKEN: ${{ secrets.RELEASE_PLZ_TOKEN }}     # PAT, NÃO o GITHUB_TOKEN
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

### Fluxo após migração

```
merge do Release PR (na main)
  → release-plz: publica crates.io + empurra tag vX.Y.Z (com PAT)
  → tag dispara release.yml → cargo-dist builda binários/Homebrew + GitHub Release
```

### Quem é dono de quê

| Concern | Hoje | Depois |
|---|---|---|
| Bump de versão | edição manual | **release-plz** |
| `CHANGELOG.md` | `just changelog` manual | **release-plz** (reusa `cliff.toml`) |
| crates.io publish | `publish-crate.yml` | **release-plz** → **deletar `publish-crate.yml`** |
| tag `vX.Y.Z` | `git tag && push` manual | **release-plz** |
| GitHub Release + notas | cargo-dist | **cargo-dist** (`git_release_enable=false` no release-plz) |
| binários + Homebrew | cargo-dist | **cargo-dist** (inalterado) |

---

## 5. Arquitetura / arquivos

- **`build.rs`** (raiz) — **novo**. Gera completions (4 shells) + `loops.1`.
- **`src/cli_command.rs`** (ou equivalente) — **novo/refator**. Definição `clap`
  compartilhada entre `src/` e `build.rs` via `include!`.
- **`Cargo.toml`** — metadados + `[build-dependencies] clap_mangen`,
  `[profile.release]`.
- **`dist-workspace.toml`** — `include` dos artefatos gerados.
- **`release-plz.toml`** (raiz) — **novo**.
- **`.github/workflows/release-plz.yml`** — **novo**.
- **`.github/workflows/publish-crate.yml`** — **deletar** (subsumido).
- **`cliff.toml`** — **reusado** (release-plz o detecta).
- **`justfile`** — recipe `changelog` vira preview local; documentar.
- **Inalterados:** `release.yml` (gerado), lógica do CLI.

### No mapa (registro da decisão)

- **ADR `docs/decisions/0007-release-plz-cargo-dist-split.md`** — **novo**, EN.
  Registra: release-plz é dono de versão/changelog/crates.io+tag; cargo-dist é
  dono de binários/Homebrew; handoff via tag empurrada com PAT. Liga a este spec.
- **`Cargo.toml`/`deny.toml`** — manter `[graph].targets` (do `deny.toml`,
  WAVE 1) em sincronia com os alvos do `dist-workspace.toml`.
- **`CLAUDE.md`** — atualizar a seção Release: novo fluxo (merge de PR), e que o
  `just changelog` é só preview.

---

## 6. Casos de borda / riscos

- **Handoff da tag é o ponto único de falha.** Se release-plz usar `GITHUB_TOKEN`
  em vez do PAT, a tag é empurrada mas `release.yml` **não dispara** e nenhum
  binário sai — silenciosamente. Validar com um release patch ponta-a-ponta antes
  de confiar.
- **`build.rs` + `include!` do `src/`.** `build.rs` não enxerga o crate compilado;
  o arquivo incluído precisa compilar isolado (sem depender de outros módulos do
  crate). Se o `struct Cli` puxar tipos do resto do `src/`, o refator cresce.
- **`OUT_DIR` é efêmero** entre `build.rs` e o empacotamento do cargo-dist.
  Confirmar o mecanismo exato (glob de `include`, ou copiar para path versionado)
  contra a versão pinada do cargo-dist (`0.32.0`); não assumir.
- **Atribuição com PAT** — tag/commit saem com a conta do autor. GitHub App
  (`release-plz[bot]`) é atribuição mais limpa, porém mais setup → **NICE**, não
  bloqueante para solo.
- **Colisão com o DoD do CI-hardening spec.** Aquele spec lista "CHANGELOG
  atualizado (git-cliff)" como passo manual de PR. Quando release-plz assumir o
  changelog, esse item fica obsoleto — atualizar lá ao landar este.
- **`auto-includes` pode já cobrir LICENSE/README.** Verificar antes de adicionar
  `include` redundante (item de S esforço que pode virar no-op).

## 7. Validação

- **WAVE 2:** buildar, inspecionar o `OUT_DIR`/archive — confirmar 4 completions +
  `loops.1` + LICENSE/README/CHANGELOG presentes no tarball. `man ./loops.1`
  renderiza. `cargo install --path .` produz binário com strip/LTO (conferir
  tamanho menor).
- **WAVE 3:** `cargo publish --dry-run --locked` passa com os metadados novos.
  Um **release patch ponta-a-ponta**: merge do Release PR → crates.io publicado →
  tag empurrada → `release.yml` dispara → binários + Homebrew + GitHub Release
  saem. Este é o teste que prova o handoff do PAT.

## 8. Definition of Done

### WAVE 2 — artefato completo
- [ ] `build.rs` gera completions (bash/zsh/fish/powershell) + `loops.1`.
- [ ] `clap_mangen` em `[build-dependencies]`; definição `clap` compartilhável.
- [ ] `dist-workspace.toml` empacota os artefatos gerados no archive.
- [ ] Verificado que LICENSE-*/README/CHANGELOG estão no tarball (auto-includes
      ou `include` explícito).
- [ ] Subcomando `loops completions` preservado (uso local).

### WAVE 3 — metadados + automação
- [ ] `Cargo.toml`: `rust-version`, `categories`, `keywords`, `readme`, `authors`.
- [ ] `[profile.release]` com `lto`/`strip`.
- [ ] `release-plz.toml` + `.github/workflows/release-plz.yml` (SHA-pin, PAT).
- [ ] `RELEASE_PLZ_TOKEN` (PAT fine-grained) configurado nos secrets do repo.
- [ ] `publish-crate.yml` deletado.
- [ ] Release patch ponta-a-ponta valida o handoff release-plz → tag → cargo-dist.
- [ ] ADR `0007-release-plz-cargo-dist-split.md` criado.
- [ ] `CLAUDE.md` (seção Release) e DoD do CI-hardening spec atualizados.
