# Tarefa: diagnosticar falha no smoke test do provider ChatGPT

- **ID:** chatgpt-smoke-invalid-response.
- **Responsável:** agente desta execução.
- **Atualizado:** 2026-10-26, revisão 13.
- **Estado:** concluído; smoke remoto confirmou o fallback SSE sem persistir conversa.
- **Checkout:** `feat/delivery-2-local-ipc`; operador autorizou publicar o lote validado completo do checkout, incluindo calibração Mac, renomeação da CLI e correção do smoke.

## Objetivo e limites

Diagnosticar por que a inferência do smoke test remoto retorna `InvalidResponse` com status HTTP 200,
sem acessar ou registrar credenciais, tokens, conteúdo da conta ou contexto privado. Corrigir o adapter
e adicionar regressão se a resposta válida estiver fora do formato atualmente suportado. Não repetir
chamada remota automaticamente: o resultado externo e o custo/uso podem ser incertos.

## Checklist e progresso

- [x] Inspecionar parser, contrato oficial implementado e testes locais do stream.
- [x] Reproduzir com fixture HTTP local ou classificar falta de evidência sem nova chamada remota.
- [x] Corrigir classificação de rejeições de stream sem registrar conteúdo sensível e testar regressão.
- [x] Rodar verificações aplicáveis e consolidar diagnóstico/limitações.

## Decisões e referências

- PD-22: inferência apenas via Responses API oficial, streaming e `store:false`; smoke usa prompt sintético fixo.
- Erro informado pelo operador: HTTP 200, `InvalidResponse`, sem código/request ID/body visíveis.
- Inspeção do adapter: `status: 200`, sem `code`, `request_id` ou `response_shape` aponta para rejeição local do formato/stream (por exemplo Content-Type inesperado, evento SSE inválido ou payload incompatível), não para falha de autenticação HTTP. O corpo remoto é deliberadamente omitido.
- Os testes existentes cobrem Responses API contra servidor local falso; eles não reproduzem a resposta observada na conta real.
- O operador confirmou que a listagem autenticada `models` retorna vários modelos visíveis, incluindo `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-sol` e `gpt-6-luna`; autenticação e catálogo estão funcionais.
- O slug usado no smoke foi confirmado como `gpt-6-luna`, presente no catálogo; isso reduz a hipótese de erro de seleção, mas não reproduz o conteúdo/eventos da inferência.
- O adapter agora marca falhas locais de sucesso HTTP com códigos estáveis e não sensíveis (`client_unexpected_content_type`, `client_invalid_event_json` e categorias equivalentes). Regressão local cobre Content-Type e JSON de evento inválidos sem expor corpo remoto.
- No Content-Type inesperado, o adapter agora registra somente o media type normalizado, limitado e ASCII gráfico, sem parâmetros; não registra corpo, credenciais ou outros headers.

## Evidências e validação

- `CARAPANA_E2E_PREVIEW_PORT=34173 CARAPANA_E2E_DEV_PORT=34174 CARAPANA_SNAPSHOT_ENV=darwin mise exec -- make verify` passou após a última alteração: Web format/lint/check, Vitest 27/27, build, E2E 66/66, Python 11/11, fmt, Clippy, testes Rust/doc-tests e build do workspace.
- O operador confirmou `Carapana SIWC smoke test OK`; a inferência sintética terminou e nenhuma conversa foi persistida.
- Auditoria pré-publicação: nenhum token, credencial, identificador de conta, path de workspace ou conteúdo real de conversa encontrado. Benchmarks contêm métricas sintéticas e metadados do Mac (modelo/OS); snapshots mostram somente fixtures sintéticas.

## Bloqueios e próxima ação

- O operador informou que o header é ausente/ilegível (`<missing-or-invalid>`). O parser aceita essa ausência apenas como tentativa de SSE e exige JSON de eventos válido e `response.completed`; testes locais cobrem sucesso completo, JSON inválido e EOF sem conclusão.
- Validação local e smoke remoto concluídos. Não foi necessária alteração adicional do parser nem persistência de conversa.
