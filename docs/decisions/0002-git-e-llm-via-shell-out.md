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
