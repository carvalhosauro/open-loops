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
