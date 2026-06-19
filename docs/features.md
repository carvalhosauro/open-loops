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
