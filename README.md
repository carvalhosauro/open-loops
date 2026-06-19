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
