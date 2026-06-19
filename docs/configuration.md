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
