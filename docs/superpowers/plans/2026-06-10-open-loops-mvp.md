# open-loops MVP — Plano de Implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** CLI `loops` em Rust que lista trabalhos iniciados e não finalizados (branches não mergeadas, cross-repo) e destila o contexto de retomada (por quê / feito / falta / próximo passo) a partir de sessões de IA + git, via LLM sob demanda.

**Architecture:** Binário único + lib (`open_loops`). Módulos pequenos com responsabilidade única: `config`, `ignores`, `scanner` (git via shell-out), `sessions` (trait adapter + adapter Claude Code), `distill` (prompt + LLM via comando configurável), `cache` (por branch@HEAD), `output`, `cli`. Estado todo em `~/.open-loops/` (override por `OPEN_LOOPS_HOME` para testes). Spec: `docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md`.

**Tech Stack:** Rust (edition 2021), clap 4 (derive), serde + toml, serde_json, anyhow, chrono, dirs. Dev: tempfile, assert_cmd, predicates. DX: lefthook, cargo-llvm-cov, justfile, cargo-dist, git-cliff.

---

## Estrutura de arquivos (visão final)

```
Cargo.toml                  # crate open-loops, bin "loops", lib open_loops
rust-toolchain.toml         # toolchain pinada
.editorconfig
LICENSE-MIT / LICENSE-APACHE
justfile                    # setup/test/lint/fmt/cov/changelog
lefthook.yml                # pre-commit (fmt+clippy) e commit-msg (conventional)
cliff.toml                  # changelog por conventional commits
.github/workflows/ci.yml    # fmt+clippy+test+coverage gate 70%
.github/workflows/release.yml  # gerado pelo cargo-dist
.github/PULL_REQUEST_TEMPLATE.md
.github/ISSUE_TEMPLATE/{bug_report.md,feature_request.md}
src/main.rs                 # entry: parse + dispatch
src/lib.rs                  # declaração dos módulos
src/cli.rs                  # clap structs + run_list/run_init/run_ignore/run_resume
src/config.rs               # Config + Store (config.toml)
src/ignores.rs              # Ignores (ignores.toml)
src/scanner.rs              # OpenLoop, find_repos, open_loops, scan, git helpers
src/sessions/mod.rs         # SessionExcerpt, trait SessionSource
src/sessions/claude_code.rs # adapter Claude Code (~/.claude/projects/*.jsonl)
src/distill.rs              # build_prompt, run_llm, with_sources
src/cache.rs                # Cache (cache/<repo>/<branch>@<sha>.md)
src/output.rs               # human_age, render_table
src/testutil.rs             # #[cfg(test)] helpers de repo git temporário
tests/cli.rs                # teste E2E via assert_cmd
README.md, AGENTS.md, docs/{setup,features,configuration}.md, docs/decisions/
```

Convenção de erro: mensagens em português, sempre acionáveis, nunca abortar operação inteira por falha parcial (warnings em stderr).

---

### Task 1: Scaffold do crate + toolchain + licenças

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.editorconfig`, `.gitignore`, `src/main.rs`, `src/lib.rs`, `LICENSE-MIT`, `LICENSE-APACHE`

- [ ] **Step 1: Criar o crate e arquivos base**

```bash
cd /home/gustavo/repo/me/open-loops
cargo init --name open-loops
```

Substituir `Cargo.toml` por:

```toml
[package]
name = "open-loops"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
description = "Recupere o contexto de trabalhos pausados: o que começou, onde parou, qual o próximo passo"
repository = "https://github.com/carvalhosauro/open-loops"

[lib]
name = "open_loops"
path = "src/lib.rs"

[[bin]]
name = "loops"
path = "src/main.rs"

[dependencies]
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive"] }
dirs = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

`src/lib.rs`:

```rust
//! open-loops: recupera contexto de trabalhos pausados.
//! Spec: docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md
```

`src/main.rs`:

```rust
fn main() {}
```

`rust-toolchain.toml` (pinar a stable corrente; rode `rustc --version` e registre a exata):

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`.editorconfig`:

```ini
root = true

[*]
charset = utf-8
end_of_line = lf
insert_final_newline = true
indent_style = space
indent_size = 4

[*.{yml,yaml,toml,md,json}]
indent_size = 2
```

`.gitignore`:

```
/target
```

- [ ] **Step 2: Baixar licenças (dual MIT OR Apache-2.0)**

```bash
curl -fsSL https://www.apache.org/licenses/LICENSE-2.0.txt -o LICENSE-APACHE
cat > LICENSE-MIT <<'EOF'
MIT License

Copyright (c) 2026 Gustavo

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
EOF
```

- [ ] **Step 3: Verificar que compila**

Run: `cargo build`
Expected: `Finished` sem erros.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .editorconfig .gitignore src LICENSE-MIT LICENSE-APACHE
git commit -m "build: scaffold rust crate with dual MIT/Apache-2.0 license"
```

---

### Task 2: DX — justfile, lefthook, CI com gate de cobertura

**Files:**
- Create: `justfile`, `lefthook.yml`, `.github/workflows/ci.yml`

- [ ] **Step 1: Criar `justfile`**

```just
default: test

# instala hooks e valida toolchain
setup:
    @command -v lefthook >/dev/null || (echo "instale lefthook: https://lefthook.dev/installation/" && exit 1)
    lefthook install
    rustup show

test:
    cargo test

lint:
    cargo clippy --all-targets -- -D warnings

fmt:
    cargo fmt

# requer: cargo install cargo-llvm-cov
cov:
    cargo llvm-cov --fail-under-lines 70

# requer: cargo install git-cliff
changelog:
    git cliff -o CHANGELOG.md
```

- [ ] **Step 2: Criar `lefthook.yml`**

```yaml
pre-commit:
  parallel: true
  commands:
    fmt:
      run: cargo fmt --check
    clippy:
      run: cargo clippy --all-targets -- -D warnings

commit-msg:
  commands:
    conventional:
      run: >
        grep -qE '^(feat|fix|refactor|perf|test|docs|build|ci|chore|style|revert)(\(.+\))?!?: .+'
        {1} || (echo "commit fora do padrão Conventional Commits: type(scope): resumo" && exit 1)
```

- [ ] **Step 3: Criar `.github/workflows/ci.yml`**

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@cargo-llvm-cov
      # gate da spec: >=70% global de linhas
      - run: cargo llvm-cov --fail-under-lines 70
```

- [ ] **Step 4: Instalar hooks e validar**

Run: `just setup && cargo fmt --check`
Expected: hooks instalados, fmt sem diffs. (Se lefthook não estiver instalado, instale antes — ex.: `npm install -g lefthook` ou `brew install lefthook`.)

- [ ] **Step 5: Commit**

```bash
git add justfile lefthook.yml .github/workflows/ci.yml
git commit -m "ci: add justfile, lefthook hooks and ci with 70% coverage gate"
```

---

### Task 3: Módulo `config` — Config + Store

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Escrever testes que falham**

`src/config.rs` (testes primeiro, no fim do arquivo; o módulo inteiro vai abaixo):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_sem_arquivo_retorna_default() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let cfg = store.load().unwrap();
        assert!(cfg.roots.is_empty());
        assert_eq!(cfg.llm_command, "claude -p");
        assert_eq!(cfg.max_sessions, 3);
        assert_eq!(cfg.max_session_kb, 50);
    }

    #[test]
    fn save_e_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let mut cfg = Config::default();
        cfg.llm_command = "cat".into();
        store.save(&cfg).unwrap();
        assert_eq!(store.load().unwrap().llm_command, "cat");
    }

    #[test]
    fn add_roots_canonicaliza_e_deduplica() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let root = tmp.path().join("projetos");
        std::fs::create_dir_all(&root).unwrap();
        store.add_roots(&[root.clone()]).unwrap();
        let cfg = store.add_roots(&[root.clone()]).unwrap();
        assert_eq!(cfg.roots.len(), 1);
        assert!(cfg.roots[0].is_absolute());
    }

    #[test]
    fn add_roots_falha_para_dir_inexistente() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("state"));
        let err = store.add_roots(&[tmp.path().join("nao-existe")]).unwrap_err();
        assert!(err.to_string().contains("raiz inexistente"));
    }
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod config;
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test config`
Expected: erro de compilação — `Store`/`Config` não existem.

- [ ] **Step 3: Implementar**

Topo de `src/config.rs` (acima do `mod tests`):

```rust
//! Config persistida em <base>/config.toml.
//! O caminho base vem de fora (main resolve OPEN_LOOPS_HOME ou ~/.open-loops)
//! para que testes injetem um tempdir — nada aqui lê variáveis de ambiente.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Diretórios onde os repositórios git são procurados.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Comando que recebe o prompt em stdin e devolve a resposta em stdout.
    #[serde(default = "default_llm_command")]
    pub llm_command: String,
    /// Diretório de sessões do Claude Code.
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: PathBuf,
    /// Máximo de sessões usadas na destilação.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    /// KB lidos do fim de cada sessão.
    #[serde(default = "default_max_session_kb")]
    pub max_session_kb: u64,
}

fn default_llm_command() -> String {
    "claude -p".into()
}

fn default_sessions_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude/projects")
}

fn default_max_sessions() -> usize {
    3
}

fn default_max_session_kb() -> u64 {
    50
}

impl Default for Config {
    fn default() -> Self {
        Self {
            roots: vec![],
            llm_command: default_llm_command(),
            sessions_dir: default_sessions_dir(),
            max_sessions: default_max_sessions(),
            max_session_kb: default_max_session_kb(),
        }
    }
}

pub struct Store {
    base: PathBuf,
}

impl Store {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    pub fn config_path(&self) -> PathBuf {
        self.base.join("config.toml")
    }

    pub fn load(&self) -> Result<Config> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("lendo {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("config.toml inválido em {}", path.display()))
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        std::fs::create_dir_all(&self.base)
            .with_context(|| format!("criando {}", self.base.display()))?;
        std::fs::write(self.config_path(), toml::to_string_pretty(config)?)?;
        Ok(())
    }

    pub fn add_roots(&self, paths: &[PathBuf]) -> Result<Config> {
        let mut config = self.load()?;
        for p in paths {
            let abs = std::fs::canonicalize(p)
                .with_context(|| format!("raiz inexistente: {}", p.display()))?;
            if !config.roots.contains(&abs) {
                config.roots.push(abs);
            }
        }
        self.save(&config)?;
        Ok(config)
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test config`
Expected: `4 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/lib.rs
git commit -m "feat(config): add config store with toml persistence"
```

---

### Task 4: Módulo `ignores`

**Files:**
- Create: `src/ignores.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Escrever testes que falham**

`src/ignores.rs` (testes no fim do arquivo):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vazio_quando_arquivo_nao_existe() {
        let tmp = tempfile::tempdir().unwrap();
        let ig = Ignores::load(tmp.path()).unwrap();
        assert!(!ig.contains("repo/branch"));
    }

    #[test]
    fn add_persiste_e_contains_acha() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ig = Ignores::load(tmp.path()).unwrap();
        ig.add("app/feat/x").unwrap();
        let recarregado = Ignores::load(tmp.path()).unwrap();
        assert!(recarregado.contains("app/feat/x"));
        assert!(!recarregado.contains("app/feat/y"));
    }
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod ignores;
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test ignores`
Expected: erro de compilação — `Ignores` não existe.

- [ ] **Step 3: Implementar**

Topo de `src/ignores.rs`:

```rust
//! Loops descartados pelo usuário ("não vale continuar").
//! Persistido em <base>/ignores.toml, chaves no formato "repo/branch".
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct IgnoreFile {
    #[serde(default)]
    ignored: BTreeSet<String>,
}

pub struct Ignores {
    path: PathBuf,
    set: BTreeSet<String>,
}

impl Ignores {
    pub fn load(base: &Path) -> Result<Self> {
        let path = base.join("ignores.toml");
        let set = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            toml::from_str::<IgnoreFile>(&raw)
                .with_context(|| format!("ignores.toml inválido em {}", path.display()))?
                .ignored
        } else {
            BTreeSet::new()
        };
        Ok(Self { path, set })
    }

    pub fn add(&mut self, key: &str) -> Result<()> {
        self.set.insert(key.to_string());
        std::fs::create_dir_all(self.path.parent().expect("path tem parent"))?;
        let file = IgnoreFile { ignored: self.set.clone() };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?)?;
        Ok(())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test ignores`
Expected: `2 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/ignores.rs src/lib.rs
git commit -m "feat(ignores): add persistent ignore list for dead loops"
```

---

### Task 5: Helper de teste `testutil` + scanner: `git()` e `default_branch()`

**Files:**
- Create: `src/testutil.rs`, `src/scanner.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Criar `src/testutil.rs`**

```rust
//! Helpers de teste: repositórios git temporários.
use std::path::Path;
use std::process::Command;

pub fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git instalado")
        .status;
    assert!(status.success(), "git {args:?} falhou em {}", repo.display());
}

/// Cria repo com branch main e 1 commit.
pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

/// Cria branch a partir do estado atual com 1 commit próprio.
pub fn add_branch_with_commit(dir: &Path, branch: &str, file: &str) {
    git(dir, &["checkout", "-b", branch]);
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", &format!("wip {branch}")]);
    git(dir, &["checkout", "main"]);
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod scanner;
#[cfg(test)]
pub mod testutil;
```

- [ ] **Step 2: Escrever testes que falham**

`src/scanner.rs` começa só com os testes:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn default_branch_detecta_main() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        assert_eq!(default_branch(&repo).unwrap(), "main");
    }

    #[test]
    fn git_falha_com_mensagem_contextual() {
        let tmp = tempfile::tempdir().unwrap();
        // diretório não é repo git
        let err = git(tmp.path(), &["status"]).unwrap_err();
        assert!(err.to_string().contains("git"));
    }
}
```

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test scanner`
Expected: erro de compilação — `default_branch`/`git` não existem.

- [ ] **Step 4: Implementar**

Topo de `src/scanner.rs`:

```rust
//! Descoberta de repositórios e branches não mergeadas via shell-out ao git.
//! Decisão de design: shell-out (não git2/gix) — simples e debugável;
//! o gargalo de performance do produto é o LLM, não o git.
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("git não encontrado no PATH — instale o git")?;
    if !out.status.success() {
        bail!(
            "git {:?} falhou em {}: {}",
            args,
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Branch default: origin/HEAD se existir; senão main; senão master.
pub fn default_branch(repo: &Path) -> Result<String> {
    if let Ok(sym) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        return Ok(sym.trim_start_matches("origin/").to_string());
    }
    for candidate in ["main", "master"] {
        if git(repo, &["rev-parse", "--verify", &format!("refs/heads/{candidate}")]).is_ok() {
            return Ok(candidate.to_string());
        }
    }
    bail!("não achei a branch default em {} (esperava origin/HEAD, main ou master)", repo.display())
}
```

- [ ] **Step 5: Rodar e ver passar**

Run: `cargo test scanner`
Expected: `2 passed`.

- [ ] **Step 6: Commit**

```bash
git add src/scanner.rs src/testutil.rs src/lib.rs
git commit -m "feat(scanner): add git shell-out helper and default branch detection"
```

---

### Task 6: Scanner — `OpenLoop`, `find_repos`, `open_loops`, `scan` e helpers de contexto

**Files:**
- Modify: `src/scanner.rs`

- [ ] **Step 1: Adicionar testes que falham** (dentro do `mod tests` existente)

```rust
    #[test]
    fn find_repos_acha_repos_ate_profundidade_3_e_pula_ocultos() {
        let tmp = tempfile::tempdir().unwrap();
        testutil::init_repo(&tmp.path().join("a/b/repo1"));
        testutil::init_repo(&tmp.path().join("repo2"));
        testutil::init_repo(&tmp.path().join(".oculto/repo3"));
        let repos = find_repos(&[tmp.path().to_path_buf()]);
        let names: Vec<_> = repos
            .iter()
            .map(|r| r.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"repo1".to_string()));
        assert!(names.contains(&"repo2".to_string()));
        assert!(!names.contains(&"repo3".to_string()));
    }

    #[test]
    fn open_loops_acha_nao_mergeada_ignora_mergeada_e_default() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");
        testutil::git(&repo, &["branch", "mergeada"]); // aponta para main => mergeada

        let loops = open_loops(&repo).unwrap();
        assert_eq!(loops.len(), 1);
        let l = &loops[0];
        assert_eq!(l.branch, "feat/x");
        assert_eq!(l.repo_name, "app");
        assert_eq!(l.key(), "app/feat/x");
        assert_eq!(l.ahead, 1);
        assert_eq!(l.behind, 0);
        assert_eq!(l.head_sha.len(), 40);
    }

    #[test]
    fn scan_agrega_repos_e_reporta_warning_sem_abortar() {
        let tmp = tempfile::tempdir().unwrap();
        let bom = tmp.path().join("bom");
        testutil::init_repo(&bom);
        testutil::add_branch_with_commit(&bom, "feat/ok", "ok.txt");
        // repo quebrado: .git é arquivo vazio => find_repos não acha; quebra de
        // verdade: repo sem nenhum commit (default_branch falha)
        let vazio = tmp.path().join("vazio");
        std::fs::create_dir_all(&vazio).unwrap();
        testutil::git(&vazio, &["init", "-b", "main"]);

        let (loops, warnings) = scan(&[tmp.path().to_path_buf()]);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].key(), "bom/feat/ok");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("vazio"));
    }

    #[test]
    fn helpers_de_contexto_retornam_commits_e_janela() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("app");
        testutil::init_repo(&repo);
        testutil::add_branch_with_commit(&repo, "feat/x", "x.txt");

        let log = git_log(&repo, "main", "feat/x").unwrap();
        assert!(log.contains("wip feat/x"));
        let stat = diffstat(&repo, "main", "feat/x").unwrap();
        assert!(stat.contains("x.txt"));
        let (ini, fim) = commit_window(&repo, "main", "feat/x").unwrap();
        assert!(ini <= fim);
    }
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test scanner`
Expected: erro de compilação — `OpenLoop`, `find_repos`, `open_loops`, `scan`, `git_log`, `diffstat`, `commit_window` não existem.

- [ ] **Step 3: Implementar** (adicionar ao `src/scanner.rs`, abaixo de `default_branch`)

```rust
#[derive(Debug, Clone)]
pub struct OpenLoop {
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub head_sha: String,
    pub last_commit: DateTime<Utc>,
    pub ahead: u32,
    pub behind: u32,
}

impl OpenLoop {
    /// Chave canônica usada em resume/ignore: "repo/branch".
    pub fn key(&self) -> String {
        format!("{}/{}", self.repo_name, self.branch)
    }
}

const MAX_DEPTH: usize = 3;
const SKIP_DIRS: [&str; 2] = ["node_modules", "target"];

/// Varre as raízes até MAX_DEPTH procurando diretórios com .git.
pub fn find_repos(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for root in roots {
        walk(root, 0, &mut repos);
    }
    repos.sort();
    repos
}

fn walk(dir: &Path, depth: usize, repos: &mut Vec<PathBuf>) {
    if dir.join(".git").is_dir() {
        repos.push(dir.to_path_buf());
        return;
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !path.is_dir() || name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        walk(&path, depth + 1, repos);
    }
}

/// Branches não mergeadas (exceto a default) de um repo.
pub fn open_loops(repo: &Path) -> Result<Vec<OpenLoop>> {
    let default = default_branch(repo)?;
    let merged: std::collections::HashSet<String> =
        git(repo, &["branch", "--merged", &default, "--format=%(refname:short)"])?
            .lines()
            .map(|s| s.trim().to_string())
            .collect();
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string());
    let raw = git(repo, &[
        "for-each-ref",
        "refs/heads",
        "--format=%(refname:short)%09%(objectname)%09%(committerdate:iso8601-strict)",
    ])?;
    let mut result = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split('\t');
        let (Some(branch), Some(sha), Some(date)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if branch == default || merged.contains(branch) {
            continue;
        }
        let counts = git(repo, &["rev-list", "--left-right", "--count", &format!("{default}...{branch}")])?;
        let mut c = counts.split_whitespace();
        let behind: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
        let ahead: u32 = c.next().unwrap_or("0").parse().unwrap_or(0);
        let last_commit = DateTime::parse_from_rfc3339(date)
            .with_context(|| format!("data inválida vinda do git: {date}"))?
            .with_timezone(&Utc);
        result.push(OpenLoop {
            repo_name: repo_name.clone(),
            repo_path: repo.to_path_buf(),
            branch: branch.to_string(),
            head_sha: sha.to_string(),
            last_commit,
            ahead,
            behind,
        });
    }
    Ok(result)
}

/// Varre todos os repos em paralelo. Falha em um repo vira warning, nunca aborta.
pub fn scan(roots: &[PathBuf]) -> (Vec<OpenLoop>, Vec<String>) {
    let repos = find_repos(roots);
    let results: Vec<Result<Vec<OpenLoop>>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| s.spawn(move || open_loops(repo)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| bail_panic()))
            .collect()
    });
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for (repo, res) in repos.iter().zip(results) {
        match res {
            Ok(mut loops) => all.append(&mut loops),
            Err(e) => warnings.push(format!("{}: {e:#}", repo.display())),
        }
    }
    (all, warnings)
}

fn bail_panic() -> Result<Vec<OpenLoop>> {
    Err(anyhow::anyhow!("panic ao escanear o repositório"))
}

/// Commits exclusivos da branch (para o prompt de destilação).
pub fn git_log(repo: &Path, default: &str, branch: &str) -> Result<String> {
    git(repo, &["log", "--oneline", &format!("{default}..{branch}")])
}

/// Diffstat da branch contra a base (para o prompt de destilação).
pub fn diffstat(repo: &Path, default: &str, branch: &str) -> Result<String> {
    git(repo, &["diff", "--stat", &format!("{default}...{branch}")])
}

/// Janela temporal dos commits exclusivos da branch (filtra sessões de IA).
pub fn commit_window(repo: &Path, default: &str, branch: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let raw = git(repo, &["log", "--format=%cI", &format!("{default}..{branch}")])?;
    let mut dates: Vec<DateTime<Utc>> = raw
        .lines()
        .filter_map(|l| DateTime::parse_from_rfc3339(l.trim()).ok())
        .map(|d| d.with_timezone(&Utc))
        .collect();
    if dates.is_empty() {
        // branch sem commit próprio: usa o último commit dela
        let head = git(repo, &["log", "-1", "--format=%cI", branch])?;
        dates.push(DateTime::parse_from_rfc3339(head.trim())?.with_timezone(&Utc));
    }
    Ok((
        *dates.iter().min().expect("dates não vazio"),
        *dates.iter().max().expect("dates não vazio"),
    ))
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test scanner`
Expected: `6 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/scanner.rs
git commit -m "feat(scanner): discover repos and unmerged branches with context helpers"
```

---

### Task 7: Sessions — trait `SessionSource` + adapter Claude Code

**Files:**
- Create: `src/sessions/mod.rs`, `src/sessions/claude_code.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Criar `src/sessions/mod.rs`** (trait + tipos; é a camada de adapters da Fase 3 da spec)

```rust
//! Fontes de sessão de IA. Cada harness (Claude Code, futuramente Codex,
//! OpenCode) vira um adapter deste trait — o resto do código não conhece
//! formato nem localização de sessão.
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

pub mod claude_code;

#[derive(Debug)]
pub struct SessionExcerpt {
    /// Nome do arquivo de sessão (exibido na seção Fontes).
    pub source: String,
    pub modified: DateTime<Utc>,
    /// Texto extraído (mensagens user/assistant), já truncado.
    pub text: String,
}

pub trait SessionSource {
    /// Trechos das sessões mais relevantes para a branch.
    /// `window`: intervalo dos commits da branch (sessões fora dele e que não
    /// mencionam a branch são descartadas).
    fn excerpts(
        &self,
        repo_path: &Path,
        branch: &str,
        window: (DateTime<Utc>, DateTime<Utc>),
        max_sessions: usize,
        max_kb: u64,
    ) -> Result<Vec<SessionExcerpt>>;
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod sessions;
```

- [ ] **Step 2: Escrever testes que falham**

`src/sessions/claude_code.rs` começa só com os testes:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionSource;
    use chrono::{Duration, Utc};
    use std::path::Path;

    #[test]
    fn encode_project_path_igual_ao_claude_code() {
        assert_eq!(
            encode_project_path(Path::new("/home/g/repo/me/open-loops")),
            "-home-g-repo-me-open-loops"
        );
        assert_eq!(encode_project_path(Path::new("/home/g/my.app")), "-home-g-my-app");
    }

    #[test]
    fn extract_text_pega_user_assistant_e_ignora_resto() {
        let user = r#"{"type":"user","message":{"content":"quero implementar login"}}"#;
        let asst = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"vou criar feat/login"}]}}"#;
        let meta = r#"{"type":"summary","summary":"x"}"#;
        assert_eq!(extract_text(user).unwrap(), "[user] quero implementar login");
        assert_eq!(extract_text(asst).unwrap(), "[assistant] vou criar feat/login");
        assert!(extract_text(meta).is_none());
        assert!(extract_text("linha corrompida não-json").is_none());
    }

    #[test]
    fn excerpts_seleciona_por_janela_tolera_lixo_e_limita_quantidade() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let repo = Path::new("/home/g/app");
        let dir = projects.join(encode_project_path(repo));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sessao1.jsonl"),
            concat!(
                r#"{"type":"user","message":{"content":"quero implementar login"}}"#, "\n",
                "lixo nao-json\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"proximo passo: validar token"}]}}"#, "\n",
            ),
        )
        .unwrap();
        // arquivo de outro formato é ignorado
        std::fs::write(dir.join("nota.txt"), "nada").unwrap();

        let src = ClaudeCode { projects_dir: projects };
        let now = Utc::now();
        let window = (now - Duration::days(1), now + Duration::days(1));
        let ex = src.excerpts(repo, "feat/login", window, 3, 50).unwrap();
        assert_eq!(ex.len(), 1);
        assert!(ex[0].text.contains("[user] quero implementar login"));
        assert!(ex[0].text.contains("proximo passo: validar token"));
        assert_eq!(ex[0].source, "sessao1.jsonl");
    }

    #[test]
    fn excerpts_vazio_quando_dir_do_projeto_nao_existe() {
        let tmp = tempfile::tempdir().unwrap();
        let src = ClaudeCode { projects_dir: tmp.path().to_path_buf() };
        let now = Utc::now();
        let ex = src
            .excerpts(Path::new("/nao/existe"), "b", (now, now), 3, 50)
            .unwrap();
        assert!(ex.is_empty());
    }
}
```

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test claude_code`
Expected: erro de compilação — `ClaudeCode`, `encode_project_path`, `extract_text` não existem.

- [ ] **Step 4: Implementar**

Topo de `src/sessions/claude_code.rs`:

```rust
//! Adapter para sessões do Claude Code (~/.claude/projects/<path-encoded>/*.jsonl).
//! ATENÇÃO: formato interno do Claude Code, não é API pública — pode mudar.
//! Por isso o parsing é tolerante: linha ruim é pulada, nunca aborta (risco 1 da spec).
use super::{SessionExcerpt, SessionSource};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

pub struct ClaudeCode {
    pub projects_dir: PathBuf,
}

/// Claude Code codifica o caminho do projeto substituindo '/' e '.' por '-'.
/// Ex.: /home/g/repo/x -> -home-g-repo-x
pub fn encode_project_path(p: &Path) -> String {
    p.to_string_lossy().replace(['/', '.'], "-")
}

/// Extrai o texto de uma linha jsonl de sessão. None para linhas
/// não-mensagem, corrompidas ou vazias (parsing tolerante).
pub fn extract_text(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let role = v.get("type")?.as_str()?;
    if role != "user" && role != "assistant" {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(format!("[{role}] {text}"))
    }
}

/// Lê os últimos `max_bytes` do arquivo e extrai o texto das mensagens.
/// O fim da conversa concentra o "onde parei" (decisão da spec).
fn read_tail_text(path: &Path, max_bytes: u64) -> Result<String> {
    let raw = std::fs::read(path)?;
    let start = raw.len().saturating_sub(max_bytes as usize);
    let tail = String::from_utf8_lossy(&raw[start..]);
    let mut lines = tail.lines();
    if start > 0 {
        lines.next(); // primeira linha pode estar cortada no meio
    }
    Ok(lines.filter_map(extract_text).collect::<Vec<_>>().join("\n"))
}

impl SessionSource for ClaudeCode {
    fn excerpts(
        &self,
        repo_path: &Path,
        branch: &str,
        window: (DateTime<Utc>, DateTime<Utc>),
        max_sessions: usize,
        max_kb: u64,
    ) -> Result<Vec<SessionExcerpt>> {
        let dir = self.projects_dir.join(encode_project_path(repo_path));
        if !dir.is_dir() {
            return Ok(vec![]);
        }
        let pad = Duration::days(7);
        let (start, end) = (window.0 - pad, window.1 + pad);
        let mut candidates: Vec<(DateTime<Utc>, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(modified) = meta.modified() else { continue };
            let modified: DateTime<Utc> = modified.into();
            let in_window = modified >= start && modified <= end;
            // heurística da spec: janela temporal OU menção à branch
            let relevant = in_window
                || std::fs::read_to_string(&path)
                    .map(|c| c.contains(branch))
                    .unwrap_or(false);
            if relevant {
                candidates.push((modified, path));
            }
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0)); // mais recente primeiro
        candidates.truncate(max_sessions);
        let mut out = Vec::new();
        for (modified, path) in candidates {
            let text = read_tail_text(&path, max_kb * 1024)?;
            if text.is_empty() {
                continue;
            }
            let source = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(SessionExcerpt { source, modified, text });
        }
        Ok(out)
    }
}
```

- [ ] **Step 5: Rodar e ver passar**

Run: `cargo test sessions`
Expected: `4 passed`.

- [ ] **Step 6: Commit**

```bash
git add src/sessions src/lib.rs
git commit -m "feat(sessions): add session source trait and claude code adapter"
```

---

### Task 8: Módulo `cache`

**Files:**
- Create: `src/cache.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Escrever testes que falham**

`src/cache.rs` (testes no fim):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fake_loop(sha: &str) -> OpenLoop {
        OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: sha.into(),
            last_commit: Utc::now(),
            ahead: 1,
            behind: 0,
        }
    }

    #[test]
    fn miss_depois_put_depois_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let lp = fake_loop("abc123");
        assert!(cache.get(&lp).is_none());
        cache.put(&lp, "contexto destilado").unwrap();
        assert_eq!(cache.get(&lp).unwrap(), "contexto destilado");
    }

    #[test]
    fn head_novo_invalida_sozinho() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        cache.put(&fake_loop("sha-velho"), "velho").unwrap();
        assert!(cache.get(&fake_loop("sha-novo")).is_none());
    }
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod cache;
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test cache`
Expected: erro de compilação — `Cache` não existe.

- [ ] **Step 3: Implementar**

Topo de `src/cache.rs`:

```rust
//! Cache de destilações em <base>/cache/<repo>/<branch>@<head-sha>.md.
//! Chavear pelo SHA do HEAD faz o cache invalidar sozinho quando a branch anda.
use crate::scanner::OpenLoop;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(base: &Path) -> Self {
        Self { dir: base.join("cache") }
    }

    fn path(&self, lp: &OpenLoop) -> PathBuf {
        // branches têm '/', que não pode virar subdiretório no nome do arquivo
        let branch = lp.branch.replace('/', "__");
        self.dir
            .join(&lp.repo_name)
            .join(format!("{branch}@{}.md", lp.head_sha))
    }

    pub fn get(&self, lp: &OpenLoop) -> Option<String> {
        std::fs::read_to_string(self.path(lp)).ok()
    }

    pub fn put(&self, lp: &OpenLoop, content: &str) -> Result<()> {
        let path = self.path(lp);
        std::fs::create_dir_all(path.parent().expect("path tem parent"))?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test cache`
Expected: `2 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs src/lib.rs
git commit -m "feat(cache): add distillation cache keyed by branch and head sha"
```

---

### Task 9: Módulo `distill` — prompt, LLM e fontes

**Files:**
- Create: `src/distill.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Escrever testes que falham**

`src/distill.rs` (testes no fim):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use crate::sessions::SessionExcerpt;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fake_loop() -> OpenLoop {
        OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: "abcdef1234567890".into(),
            last_commit: Utc::now(),
            ahead: 2,
            behind: 1,
        }
    }

    fn fake_excerpt() -> SessionExcerpt {
        SessionExcerpt {
            source: "sessao1.jsonl".into(),
            modified: Utc::now(),
            text: "[user] implementa login".into(),
        }
    }

    #[test]
    fn build_prompt_inclui_evidencias_e_secoes() {
        let p = build_prompt(&fake_loop(), "main", "abc feat: wip", "x.txt | 2 +", &[fake_excerpt()]);
        assert!(p.contains("## Por quê"));
        assert!(p.contains("## Próximo passo"));
        assert!(p.contains("app/feat/login"));
        assert!(p.contains("abc feat: wip"));
        assert!(p.contains("[user] implementa login"));
    }

    #[test]
    fn build_prompt_sem_sessoes_declara_ausencia() {
        let p = build_prompt(&fake_loop(), "main", "", "", &[]);
        assert!(p.contains("nenhuma encontrada"));
    }

    #[test]
    fn run_llm_passa_prompt_via_stdin() {
        // `cat` ecoa o stdin: valida o contrato sem LLM real
        let out = run_llm("cat", "prompt de teste").unwrap();
        assert_eq!(out.trim(), "prompt de teste");
    }

    #[test]
    fn run_llm_erro_contextual_quando_comando_falha() {
        let err = run_llm("false", "x").unwrap_err();
        assert!(err.to_string().contains("comando LLM falhou"));
    }

    #[test]
    fn with_sources_anexa_git_e_sessoes() {
        let doc = with_sources("## Por quê\nlogin", &fake_loop(), &[fake_excerpt()]);
        assert!(doc.contains("## Fontes"));
        assert!(doc.contains("abcdef1")); // sha curto
        assert!(doc.contains("sessao1.jsonl"));
    }
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod distill;
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test distill`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

Topo de `src/distill.rs`:

```rust
//! Destilação: monta o prompt com as evidências (git + sessões) e chama o
//! LLM via comando configurável (default "claude -p"). Comando injetável =
//! testes usam `cat` e usuários podem trocar de LLM sem mudar código.
use crate::scanner::OpenLoop;
use crate::sessions::SessionExcerpt;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

pub fn build_prompt(
    lp: &OpenLoop,
    default_branch: &str,
    commits: &str,
    diffstat: &str,
    excerpts: &[SessionExcerpt],
) -> String {
    let mut p = format!(
        "Você reconstrói o contexto de uma branch de trabalho pausada.\n\
         Responda em markdown, em português, com exatamente estas seções:\n\n\
         ## Por quê\n## Feito\n## Falta\n## Próximo passo\n\n\
         Seja concreto e direto. Baseie-se APENAS nas evidências abaixo.\n\
         Se a evidência for insuficiente para uma seção, escreva \"evidência insuficiente\".\n\n\
         # Branch\n{key} (base: {default_branch})\n\n\
         # Commits (base..branch)\n{commits}\n\n\
         # Diffstat\n{diffstat}\n",
        key = lp.key(),
    );
    if excerpts.is_empty() {
        p.push_str("\n# Sessões de IA\nnenhuma encontrada\n");
    } else {
        for e in excerpts {
            p.push_str(&format!(
                "\n# Sessão {} (modificada {})\n{}\n",
                e.source,
                e.modified.format("%Y-%m-%d"),
                e.text
            ));
        }
    }
    p
}

/// Executa o comando LLM com o prompt em stdin e devolve o stdout.
pub fn run_llm(llm_command: &str, prompt: &str) -> Result<String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(llm_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!("falha ao executar o comando LLM `{llm_command}` — está instalado? Ajuste llm_command no config.toml")
        })?;
    child
        .stdin
        .take()
        .expect("stdin pipeado")
        .write_all(prompt.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "comando LLM falhou (`{llm_command}`): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Anexa a seção Fontes (auditoria do usuário — mitigação do risco 2 da spec).
pub fn with_sources(answer: &str, lp: &OpenLoop, excerpts: &[SessionExcerpt]) -> String {
    let sha_curto = &lp.head_sha[..7.min(lp.head_sha.len())];
    let mut doc = format!(
        "# {}\n\n{}\n\n## Fontes\n\n- git: branch {} (HEAD {})\n",
        lp.key(),
        answer.trim(),
        lp.branch,
        sha_curto
    );
    for e in excerpts {
        doc.push_str(&format!(
            "- sessão: {} (modificada {})\n",
            e.source,
            e.modified.format("%Y-%m-%d")
        ));
    }
    doc
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test distill`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/distill.rs src/lib.rs
git commit -m "feat(distill): build evidence prompt and run configurable llm command"
```

---

### Task 10: Módulo `output` — idade humana e tabela

**Files:**
- Create: `src/output.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Escrever testes que falham**

`src/output.rs` (testes no fim):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    fn lp(branch: &str, idade_dias: i64) -> OpenLoop {
        OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: branch.into(),
            head_sha: "abc".into(),
            last_commit: Utc::now() - Duration::days(idade_dias),
            ahead: 1,
            behind: 0,
        }
    }

    #[test]
    fn human_age_minutos_horas_dias() {
        let now = Utc::now();
        assert_eq!(human_age(now, now - Duration::minutes(5)), "5min");
        assert_eq!(human_age(now, now - Duration::hours(3)), "3h");
        assert_eq!(human_age(now, now - Duration::days(12)), "12d");
    }

    #[test]
    fn render_table_ordena_mais_parado_primeiro() {
        let t = render_table(&[lp("recente", 1), lp("antiga", 30)], Utc::now());
        let pos_antiga = t.find("antiga").unwrap();
        let pos_recente = t.find("recente").unwrap();
        assert!(pos_antiga < pos_recente);
        assert!(t.contains("LOOP"));
        assert!(t.contains("30d"));
    }

    #[test]
    fn render_table_vazia_celebra() {
        assert!(render_table(&[], Utc::now()).contains("Nenhum loop aberto"));
    }
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod output;
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test output`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

Topo de `src/output.rs`:

```rust
//! Renderização para o terminal: tabela do inventário e idades humanas.
use crate::scanner::OpenLoop;
use chrono::{DateTime, Utc};

pub fn human_age(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let mins = (now - then).num_minutes().max(0);
    if mins < 60 {
        format!("{mins}min")
    } else if mins < 48 * 60 {
        format!("{}h", mins / 60)
    } else {
        format!("{}d", mins / (60 * 24))
    }
}

/// Tabela ordenada do mais parado para o mais recente (decisão da spec:
/// staleness é o critério de atenção).
pub fn render_table(loops: &[OpenLoop], now: DateTime<Utc>) -> String {
    if loops.is_empty() {
        return "Nenhum loop aberto. Tudo finalizado ou ignorado.\n".into();
    }
    let mut sorted: Vec<&OpenLoop> = loops.iter().collect();
    sorted.sort_by_key(|l| l.last_commit);
    let key_w = sorted.iter().map(|l| l.key().len()).max().unwrap_or(4).max(4);
    let mut out = format!("{:<key_w$}  {:>9}  {:>5}  {:>6}\n", "LOOP", "PARADO HÁ", "AHEAD", "BEHIND");
    for l in sorted {
        out.push_str(&format!(
            "{:<key_w$}  {:>9}  {:>5}  {:>6}\n",
            l.key(),
            human_age(now, l.last_commit),
            l.ahead,
            l.behind
        ));
    }
    out
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test output`
Expected: `3 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/output.rs src/lib.rs
git commit -m "feat(output): render inventory table sorted by staleness"
```

---

### Task 11: CLI — clap, `run_list`, `run_init`, `run_ignore` e main

**Files:**
- Create: `src/cli.rs`
- Modify: `src/lib.rs`, `src/main.rs`

- [ ] **Step 1: Criar `src/cli.rs`** (camada fina — coberta pelo teste E2E da Task 13)

```rust
//! Definição dos comandos e orquestração dos módulos.
use crate::config::Store;
use crate::ignores::Ignores;
use crate::scanner::{self, OpenLoop};
use crate::{cache, distill, output, sessions};
use anyhow::{bail, ensure, Result};
use clap::{Parser, Subcommand};
use sessions::SessionSource;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "loops", version, about = "Recupere o contexto de trabalhos pausados")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Registra raízes de repositórios (ex.: loops init ~/repo)
    Init { paths: Vec<PathBuf> },
    /// Destila o contexto de um loop: por quê, feito, falta, próximo passo
    Resume { query: String },
    /// Descarta um loop morto da lista (formato repo/branch)
    Ignore { key: String },
}

pub fn run_list(base: &Path) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "nenhuma raiz configurada. Rode: loops init <dir-com-seus-repos>"
    );
    let (found, warnings) = scanner::scan(&cfg.roots);
    for w in &warnings {
        eprintln!("aviso: {w}");
    }
    let ignores = Ignores::load(base)?;
    let visible: Vec<OpenLoop> = found.into_iter().filter(|l| !ignores.contains(&l.key())).collect();
    print!("{}", output::render_table(&visible, chrono::Utc::now()));
    Ok(())
}

pub fn run_init(base: &Path, paths: &[PathBuf]) -> Result<()> {
    ensure!(!paths.is_empty(), "uso: loops init <dir> [<dir>...]");
    let store = Store::new(base.to_path_buf());
    let cfg = store.add_roots(paths)?;
    println!("raízes registradas:");
    for r in &cfg.roots {
        println!("  {}", r.display());
    }
    println!("\nconfig em {}", store.config_path().display());
    Ok(())
}

pub fn run_ignore(base: &Path, key: &str) -> Result<()> {
    ensure!(
        key.contains('/'),
        "formato esperado: repo/branch (rode `loops` para ver as chaves)"
    );
    let mut ignores = Ignores::load(base)?;
    ignores.add(key)?;
    println!("ignorado: {key}");
    Ok(())
}

pub fn run_resume(base: &Path, query: &str) -> Result<()> {
    let store = Store::new(base.to_path_buf());
    let cfg = store.load()?;
    ensure!(
        !cfg.roots.is_empty(),
        "nenhuma raiz configurada. Rode: loops init <dir-com-seus-repos>"
    );
    let (found, warnings) = scanner::scan(&cfg.roots);
    for w in &warnings {
        eprintln!("aviso: {w}");
    }
    // resolução fuzzy: substring case-insensitive sobre a chave repo/branch
    let q = query.to_lowercase();
    let matches: Vec<&OpenLoop> = found.iter().filter(|l| l.key().to_lowercase().contains(&q)).collect();
    let lp = match matches.len() {
        0 => bail!("nenhum loop bate com '{query}'. Rode `loops` para ver os abertos."),
        1 => matches[0],
        _ => bail!(
            "query ambígua, candidatos:\n{}",
            matches.iter().map(|l| format!("  {}", l.key())).collect::<Vec<_>>().join("\n")
        ),
    };

    let cache = cache::Cache::new(base);
    if let Some(hit) = cache.get(lp) {
        println!("{hit}");
        return Ok(());
    }

    let default = scanner::default_branch(&lp.repo_path)?;
    let commits = scanner::git_log(&lp.repo_path, &default, &lp.branch)?;
    let diffstat = scanner::diffstat(&lp.repo_path, &default, &lp.branch)?;
    let window = scanner::commit_window(&lp.repo_path, &default, &lp.branch)?;
    let source = sessions::claude_code::ClaudeCode { projects_dir: cfg.sessions_dir.clone() };
    let excerpts = source.excerpts(&lp.repo_path, &lp.branch, window, cfg.max_sessions, cfg.max_session_kb)?;
    if excerpts.is_empty() {
        eprintln!("aviso: nenhuma sessão de IA encontrada — confiança baixa, contexto só do git");
    }
    let prompt = distill::build_prompt(lp, &default, &commits, &diffstat, &excerpts);
    let answer = distill::run_llm(&cfg.llm_command, &prompt)?;
    let doc = distill::with_sources(&answer, lp, &excerpts);
    cache.put(lp, &doc)?;
    println!("{doc}");
    Ok(())
}
```

Em `src/lib.rs` adicionar:

```rust
pub mod cli;
```

- [ ] **Step 2: Substituir `src/main.rs`**

```rust
use clap::Parser;
use open_loops::cli::{self, Cli, Command};
use std::path::PathBuf;

fn main() {
    let cli = Cli::parse();
    let base = base_dir();
    let result = match cli.command {
        None => cli::run_list(&base),
        Some(Command::Init { paths }) => cli::run_init(&base, &paths),
        Some(Command::Resume { query }) => cli::run_resume(&base, &query),
        Some(Command::Ignore { key }) => cli::run_ignore(&base, &key),
    };
    if let Err(e) = result {
        eprintln!("erro: {e:#}");
        std::process::exit(1);
    }
}

/// OPEN_LOOPS_HOME serve para testes e instalações não-padrão.
fn base_dir() -> PathBuf {
    std::env::var_os("OPEN_LOOPS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().expect("HOME não definido").join(".open-loops"))
}
```

- [ ] **Step 3: Compilar e smoke test manual**

Run: `cargo build && ./target/debug/loops --help`
Expected: help com `init`, `resume`, `ignore`.

Run: `OPEN_LOOPS_HOME=/tmp/ol-test ./target/debug/loops`
Expected: `erro: nenhuma raiz configurada. Rode: loops init <dir-com-seus-repos>` e exit code 1.

- [ ] **Step 4: Lint completo**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: limpo.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs src/lib.rs
git commit -m "feat(cli): wire list, init, ignore and resume commands"
```

---

### Task 12: Teste E2E de fluxo completo

**Files:**
- Create: `tests/cli.rs`

- [ ] **Step 1: Escrever o teste E2E**

```rust
//! E2E: binário real, repos git reais, LLM substituído por `cat`.
use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn git(repo: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?} falhou");
}

fn loops(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("loops").unwrap();
    cmd.env("OPEN_LOOPS_HOME", home);
    cmd
}

#[test]
fn fluxo_completo_init_list_resume_cache_ignore() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("projetos/meu-app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    git(&repo, &["checkout", "-b", "feat/login"]);
    std::fs::write(repo.join("b.txt"), "b").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "feat: login wip"]);

    // init registra a raiz
    loops(&home)
        .arg("init")
        .arg(tmp.path().join("projetos"))
        .assert()
        .success()
        .stdout(predicate::str::contains("raízes registradas"));

    // list mostra o loop aberto
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("meu-app/feat/login"));

    // troca o LLM por `cat`: resume imprime o prompt (que contém os commits)
    let cfg_path = home.join("config.toml");
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(&cfg_path, cfg.replace("llm_command = \"claude -p\"", "llm_command = \"cat\"")).unwrap();

    loops(&home)
        .args(["resume", "feat/login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("feat: login wip"))
        .stdout(predicate::str::contains("## Fontes"))
        .stderr(predicate::str::contains("confiança baixa")); // sem sessões de IA no fixture

    // segunda chamada vem do cache: funciona mesmo com LLM quebrado
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(&cfg_path, cfg.replace("llm_command = \"cat\"", "llm_command = \"false\"")).unwrap();
    loops(&home)
        .args(["resume", "feat/login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Fontes"));

    // ignore remove da lista
    loops(&home)
        .args(["ignore", "meu-app/feat/login"])
        .assert()
        .success();
    loops(&home)
        .assert()
        .success()
        .stdout(predicate::str::contains("feat/login").not());
}

#[test]
fn resume_sem_match_orienta_usuario() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let raiz = tmp.path().join("projetos");
    std::fs::create_dir_all(&raiz).unwrap();
    loops(&home).arg("init").arg(&raiz).assert().success();
    loops(&home)
        .args(["resume", "nao-existe"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nenhum loop bate"));
}
```

- [ ] **Step 2: Rodar e ver passar**

Run: `cargo test --test cli`
Expected: `2 passed`. (Se falhar, é bug real de integração — debugar antes de seguir.)

- [ ] **Step 3: Suite completa + cobertura local**

Run: `cargo test && cargo llvm-cov --fail-under-lines 70`
Expected: tudo verde, cobertura ≥70%. (Requer `cargo install cargo-llvm-cov`.)

- [ ] **Step 4: Commit**

```bash
git add tests/cli.rs
git commit -m "test: add e2e flow covering init, list, resume, cache and ignore"
```

---

### Task 13: Documentação — README, docs/, AGENTS.md, templates

**Files:**
- Create: `README.md`, `AGENTS.md`, `docs/setup.md`, `docs/features.md`, `docs/configuration.md`, `docs/decisions/0001-mvp-pull-only.md`, `docs/decisions/0002-git-e-llm-via-shell-out.md`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/ISSUE_TEMPLATE/bug_report.md`, `.github/ISSUE_TEMPLATE/feature_request.md`

Regra da spec: todo exemplo deve ser colável no terminal; nada de descrição abstrata quando um comando resolve.

- [ ] **Step 1: `README.md`**

```markdown
# open-loops

> O que eu comecei e não terminei? Onde parei? Qual o próximo passo?

`loops` lista seus trabalhos pausados (branches não mergeadas, em todos os seus
repositórios) e reconstrói o contexto de retomada a partir das suas sessões de
IA e do git — sem você documentar nada.

## Instalação

```bash
# via cargo
cargo install open-loops

# via script (Linux/macOS)
curl -fsSL https://github.com/carvalhosauro/open-loops/releases/latest/download/open-loops-installer.sh | sh
```

## Quickstart

```bash
# 1. registre onde ficam seus repositórios
loops init ~/repo

# 2. veja tudo que está aberto, do mais parado para o mais recente
loops
# LOOP                    PARADO HÁ  AHEAD  BEHIND
# meu-app/feat/login            12d      3       1
# api/fix/timeout                2d      1       0

# 3. retome um trabalho: por quê, feito, falta, próximo passo
loops resume feat/login
```

Estado fica em `~/.open-loops/` — nenhum arquivo é criado nos seus repositórios.

Docs completas em [`docs/`](docs/): [setup](docs/setup.md) ·
[funcionalidades](docs/features.md) · [configuração](docs/configuration.md).

## Licença

MIT OR Apache-2.0.
```

- [ ] **Step 2: `AGENTS.md`** (e symlink `CLAUDE.md`)

```markdown
# open-loops — mapa para agentes

CLI Rust. Binário: `loops`. Spec: `docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md`.

## Comandos de desenvolvimento

```bash
just setup   # instala hooks (lefthook)
just test    # cargo test
just lint    # clippy -D warnings
just fmt     # cargo fmt
just cov     # cobertura, gate 70% (core: alvo 85%)
```

## Estrutura

- `src/scanner.rs` — repos e branches via shell-out ao git
- `src/sessions/` — trait SessionSource + adapter claude_code (NUNCA acople
  formato de sessão fora deste diretório; Fase 3 adiciona outros harnesses)
- `src/distill.rs` — prompt + LLM via comando configurável (testes usam `cat`)
- `src/cache.rs` — cache por `branch@head-sha`
- `src/config.rs`, `src/ignores.rs` — estado em `~/.open-loops/`
- `src/cli.rs` — orquestração; camada fina, coberta por `tests/cli.rs`

## Convenções

- Conventional Commits (hook valida); mensagens de erro em PT, acionáveis
- Parsing de sessão é tolerante: linha ruim = skip + warning, nunca abort
- Testes criam repos git reais em tempdir (`src/testutil.rs`)
- Docs fazem parte do Definition of Done (checklist do PR)
- Fonte de verdade dos comandos/config: `docs/features.md` e `docs/configuration.md`
```

```bash
ln -s AGENTS.md CLAUDE.md
```

- [ ] **Step 3: `docs/setup.md`**

```markdown
# Setup

## Instalar

```bash
cargo install open-loops
# ou
curl -fsSL https://github.com/carvalhosauro/open-loops/releases/latest/download/open-loops-installer.sh | sh
```

## Configurar

```bash
loops init ~/repo ~/trabalho
```

Isso cria `~/.open-loops/config.toml`:

```toml
# diretórios varridos em busca de repositórios git (até 3 níveis)
roots = ["/home/voce/repo", "/home/voce/trabalho"]
# comando que recebe o prompt em stdin e devolve a resposta em stdout
llm_command = "claude -p"
# onde estão as sessões do Claude Code
sessions_dir = "/home/voce/.claude/projects"
# máximo de sessões usadas por destilação
max_sessions = 3
# KB lidos do fim de cada sessão
max_session_kb = 50
```

## Verificar

```bash
loops   # deve listar suas branches não mergeadas
```
```

- [ ] **Step 4: `docs/features.md`**

```markdown
# Funcionalidades

## `loops` — inventário

```bash
loops
# LOOP                    PARADO HÁ  AHEAD  BEHIND
# meu-app/feat/login            12d      3       1
```

Branches não mergeadas de todos os repos das raízes configuradas, do mais
parado para o mais recente. Sem LLM — sempre rápido.

## `loops resume <query>` — retomada

```bash
loops resume feat/login
```

A query casa por substring com `repo/branch`. Saída: `## Por quê`, `## Feito`,
`## Falta`, `## Próximo passo` + `## Fontes` (commits e sessões usados — audite
se desconfiar). Primeira chamada usa o LLM (~30-60s); repetir é instantâneo
(cache por commit). Sem sessões de IA, o contexto vem só do git e o aviso
"confiança baixa" aparece.

## `loops ignore <repo/branch>` — descartar

```bash
loops ignore meu-app/feat/experimento-velho
```

Tira o loop da lista (a branch não é tocada). Para reverter, edite
`~/.open-loops/ignores.toml`.

## `loops init <dir>...` — registrar raízes

```bash
loops init ~/repo
```
```

- [ ] **Step 5: `docs/configuration.md`**

```markdown
# Configuração

Arquivo: `~/.open-loops/config.toml` (criado pelo `loops init`).
Override do diretório base: variável `OPEN_LOOPS_HOME`.

| Chave | Tipo | Default | Descrição |
|---|---|---|---|
| `roots` | lista de paths | `[]` | Diretórios varridos (3 níveis) em busca de repos |
| `llm_command` | string | `"claude -p"` | Comando LLM: prompt via stdin, resposta via stdout |
| `sessions_dir` | path | `~/.claude/projects` | Sessões do Claude Code |
| `max_sessions` | inteiro | `3` | Sessões usadas por destilação |
| `max_session_kb` | inteiro | `50` | KB lidos do fim de cada sessão |

## Trocar o LLM

Qualquer comando que leia stdin e escreva stdout serve:

```toml
llm_command = "ollama run llama3"
```

## Arquivos de estado

```
~/.open-loops/
├── config.toml    # esta configuração
├── ignores.toml   # loops descartados via `loops ignore`
└── cache/         # destilações por repo/branch@sha (pode apagar à vontade)
```
```

- [ ] **Step 6: ADRs**

`docs/decisions/0001-mvp-pull-only.md`:

```markdown
# ADR 0001: MVP pull-only (scan sob demanda)

Data: 2026-06-10 · Status: aceito

## Contexto
O contexto de retomada existe em sessões de IA + git. Capturar via hook
(push) é mais rápido na leitura, mas só funciona dali em diante e exige
infraestrutura por máquina.

## Decisão
MVP destila sob demanda (pull): zero captura, funciona retroativamente nas
branches já existentes. Push/híbrido ficam para a Fase 2, depois de validar
a hipótese central (retomada <60s sem documentação manual).

## Consequências
Retomada fria custa chamada de LLM (~30-60s); mitigado por cache por
branch@HEAD-sha. Mapeamento sessão→branch é heurístico (janela temporal +
menção); a seção Fontes permite auditoria.
```

`docs/decisions/0002-git-e-llm-via-shell-out.md`:

```markdown
# ADR 0002: git e LLM via shell-out

Data: 2026-06-10 · Status: aceito

## Decisão
git via subprocesso (não git2/gix) e LLM via comando configurável em
`llm_command` (default `claude -p`), recebendo prompt em stdin.

## Racional
Simplicidade e debugabilidade; o gargalo é o LLM, não o git. Comando
injetável permite testar com `cat` e trocar de provedor sem mudar código.

## Consequências
Requer git e um CLI de LLM no PATH; erros orientam a instalação/configuração.
```

- [ ] **Step 7: Templates do GitHub**

`.github/PULL_REQUEST_TEMPLATE.md`:

```markdown
## O que muda

## Por quê

## Checklist

- [ ] `just test` verde
- [ ] `just lint` limpo
- [ ] Cobertura não caiu (`just cov`)
- [ ] Docs atualizadas (features/configuration/ADR) se comportamento ou config mudou
- [ ] Commits seguem Conventional Commits
```

`.github/ISSUE_TEMPLATE/bug_report.md`:

```markdown
---
name: Bug report
about: Algo não funciona como documentado
---

**O que aconteceu**

**O que era esperado**

**Como reproduzir** (comandos colaveis)

```bash
```

**Ambiente:** SO, `loops --version`, `git --version`
```

`.github/ISSUE_TEMPLATE/feature_request.md`:

```markdown
---
name: Feature request
about: Proposta de melhoria
---

**Problema que resolve** (não a solução — o problema)

**Solução proposta**

**Alternativas consideradas**
```

- [ ] **Step 8: Commit**

```bash
git add README.md AGENTS.md CLAUDE.md docs/setup.md docs/features.md docs/configuration.md docs/decisions .github/PULL_REQUEST_TEMPLATE.md .github/ISSUE_TEMPLATE
git commit -m "docs: add readme, agent map, user docs, adrs and github templates"
```

---

### Task 14: Distribuição — cargo-dist + git-cliff

**Files:**
- Create: `dist-workspace.toml`, `cliff.toml`, `.github/workflows/release.yml` (gerado)
- Modify: `Cargo.toml` (se o `dist init` pedir)

- [ ] **Step 1: Instalar e inicializar cargo-dist**

```bash
cargo install cargo-dist --locked
dist init --yes
```

Ajustar o `dist-workspace.toml` gerado para conter:

```toml
[dist]
ci = "github"
installers = ["shell", "homebrew"]
targets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
]
```

(Manter `cargo-dist-version` na versão que o `dist init` gravar.)

- [ ] **Step 2: Criar `cliff.toml`** (changelog por Conventional Commits)

```toml
[changelog]
header = "# Changelog\n"
body = """
## {{ version | default(value="unreleased") }}{% if timestamp %} - {{ timestamp | date(format="%Y-%m-%d") }}{% endif %}
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in commits %}- {{ commit.message | upper_first }}
{% endfor %}{% endfor %}
"""

[git]
conventional_commits = true
commit_parsers = [
  { message = "^feat", group = "Features" },
  { message = "^fix", group = "Fixes" },
  { message = "^perf", group = "Performance" },
  { message = "^docs", group = "Docs" },
  { message = "^refactor|^test|^build|^ci|^chore|^style", group = "Internals" },
]
```

- [ ] **Step 3: Validar plano de release localmente**

Run: `dist plan`
Expected: lista artefatos para os 4 targets + installers shell e homebrew, sem erros.

Run: `cargo install git-cliff --locked && git cliff`
Expected: changelog gerado no stdout a partir dos commits existentes.

- [ ] **Step 4: Commit**

```bash
git add dist-workspace.toml cliff.toml .github/workflows/release.yml Cargo.toml Cargo.lock
git commit -m "build: add cargo-dist release pipeline and git-cliff changelog"
```

- [ ] **Step 5: Documentar o fluxo de release no AGENTS.md** (adicionar ao fim)

```markdown
## Release

```bash
just changelog                      # atualiza CHANGELOG.md
git add CHANGELOG.md && git commit -m "docs: update changelog"
git tag v0.1.0 && git push --tags   # CI builda binários + installers + release notes
```
```

```bash
git add AGENTS.md
git commit -m "docs: document release flow"
```

---

### Task 15: Verificação final contra a spec

- [ ] **Step 1: Suite completa**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check && cargo llvm-cov --fail-under-lines 70`
Expected: tudo verde.

- [ ] **Step 2: Dogfood real**

```bash
cargo install --path .
loops init ~/repo
loops
loops resume <alguma-branch-real-parada>
```

Expected: inventário <5s; resume frio <60s com as 4 seções + Fontes; segunda chamada instantânea. **Cronometrar e anotar** — são os critérios de sucesso 1 e 2 da spec.

- [ ] **Step 3: Checklist da spec**

- Inventário cross-repo automático (branch = unidade) ✓ Task 6/11
- Retomada com POR QUÊ/FEITO/FALTA/PRÓXIMO PASSO + fontes ✓ Task 9/11
- Estado 100% fora dos repos ✓ `~/.open-loops/` (Tasks 3/4/8)
- LLM configurável, default `claude -p` ✓ Task 9
- Erros tolerantes com contexto ✓ Tasks 6/7/9
- Gate de cobertura 70% + lefthook + justfile ✓ Task 2
- Distribuição multi-canal + changelog por tag ✓ Task 14
- Docs com exemplos colaveis + ADRs + templates ✓ Task 13
- Licença dual ✓ Task 1

- [ ] **Step 4: Commit final (se houver ajustes do dogfood)**

```bash
git add -A && git commit -m "chore: final adjustments from spec verification"
```

---

## Fora deste plano

Fase 2 (hook de checkpoint, skill `/loops:resume`, modo híbrido) e Fase 3 (adapters Codex/OpenCode) — novas specs/planos. Protocolo de experimentos da spec só é acionado se a heurística sessão→branch ou o truncamento se mostrarem insuficientes no dogfood; nesse caso criar `experiments/<tema>/` conforme a spec antes de mudar `src/`.
