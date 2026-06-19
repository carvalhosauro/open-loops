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
