# Revisão de Higiene de Código — 2026-06-18

**Escopo:** 14 arquivos fonte (`src/**/*.rs`, `tests/cli.rs`, `Cargo.toml`) — branch `feat/mvp` vs `main`
**Categorias:** Valores mágicos, Código morto, Duplicação, N+1, Tipagem, Legibilidade geral
**Total de achados:** 12 (0 críticos, 7 médios, 5 baixos)

---

## src/output.rs — 6 pts (3 médios)

### [medium] Magic number 60 — limiar de minutos (linha 14)
**Por quê:** Literal `60` para troca de min→h não deixa claro o domínio de tempo.
**Direção:** Extrair constante `MINUTES_PER_HOUR = 60`.

### [medium] Magic number 48 — limiar de horas (linha 14)
**Por quê:** `48` para troca de h→d não documenta que representa "2 dias em horas".
**Direção:** Extrair `HOURS_THRESHOLD_FOR_DAYS = 48` com comentário explicando a escolha.

### [medium] Magic numbers 60 e 24 na conversão de tempo (linha 17)
**Por quê:** `mins / (60 * 24)` recalcula minutos-por-dia com literais repetidos.
**Direção:** Extrair `MINUTES_PER_HOUR` e `HOURS_PER_DAY` e compor na expressão.

---

## src/cli.rs — 2 pts (1 médio)

### [medium] Sequência config-load-scan-warnings duplicada em run_list e run_resume (linha 32)
**Por quê:** `Store::new`, `cfg.load()`, verificação de `roots`, `scanner::scan()` e loop de warnings aparecem duas vezes. Mudança nessa lógica exige edição em dois lugares.
**Direção:** Extrair helper `load_and_scan(base: &Path) -> Result<(Config, Vec<OpenLoop>, Vec<String>)>`.

---

## tests/cli.rs — 2 pts (1 médio)

### [medium] Função git() duplicada de src/testutil.rs (linha 6)
**Por quê:** Lógica de invocação de git em testes existe nos dois arquivos com mínima diferença.
**Direção:** Tornar `testutil::git` pública e usá-la em `tests/cli.rs`.

---

## src/scanner.rs — 2 pts (1 médio)

### [medium] MAX_DEPTH = 3 sem documentação da razão (linha 81)
**Por quê:** O valor 3 é nomeado mas sem explicação de por que esse limite é suficiente.
**Direção:** Adicionar comentário `///` explicando que 3 nivels cobre a maioria dos layouts de monorepo.

---

## src/sessions/claude_code.rs — 2 pts (1 médio)

### [medium] Magic number 7 — padding de janela temporal (linha 79)
**Por quê:** `Duration::days(7)` sem nome não comunica que é margem de tolerância.
**Direção:** Extrair constante `WINDOW_PADDING_DAYS: i64 = 7` com comentário.

---

## src/config.rs — 2 pts (2 baixos)

### [low] Magic number 3 — DEFAULT_MAX_SESSIONS (linha 38)
**Direção:** Extrair constante `DEFAULT_MAX_SESSIONS: usize = 3`.

### [low] Magic number 50 — DEFAULT_MAX_SESSION_KB (linha 42)
**Direção:** Extrair constante `DEFAULT_MAX_SESSION_KB: u64 = 50`.

---

## src/distill.rs — 1 pt (1 baixo)

### [low] Magic number 7 — comprimento de SHA curto (linha 99)
**Por quê:** `[..7]` é convenção conhecida mas não nomeada.
**Direção:** Extrair `SHA_SHORT_LEN: usize = 7`.

---

## src/cache.rs — 2 pts (2 baixos)

### [low] Magic strings '/' e '__' na codificação de branch (linha 22)
**Direção:** Extrair constantes `BRANCH_SEP` e `BRANCH_SEP_REPLACEMENT`.

### [low] Padrão parent().ok_or_else duplicado com ignores.rs (linha 40)
**Direção:** Aceitar como idiomático em Rust, ou extrair helper `ensure_parent_exists`.

---

## Resumo por categoria

| Categoria | Achados |
|---|---|
| Valores mágicos | 9 |
| Duplicação | 3 |
| Código morto | 0 |
| N+1 | 0 |
| Tipagem | 0 |
| Legibilidade geral | 0 |

Nenhum achado de alta severidade. Os médios são todos oportunidades de extração de constantes e eliminação de duplicação — não bloqueiam merge.
