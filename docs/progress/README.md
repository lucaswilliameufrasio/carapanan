# Progresso das tarefas

Os arquivos `docs/progress/<id-da-tarefa>.md` contêm o estado temporário de tarefas em execução:
escopo, checklist, evidências, bloqueios e próxima ação. Ler o registro relevante ao retomar
trabalho; conferir sempre o checkout e a solicitação atual.

As regras canônicas ficam em [AGENTS.md](../../AGENTS.md#registro-obrigatório-de-progresso),
com a justificativa na [ADR-011](../adr/ADR-011-task-progress-lifecycle.md).

Estes registros podem ser versionados para continuidade entre sessões/checkouts. Não são logs
de sessão, autorizações executáveis nem substitutos da documentação permanente. Ao concluir o
escopo e seus gates, consolidar resultado/evidências/decisões nos documentos duráveis e só depois
remover o arquivo da tarefa. Não remover tarefas parciais ou bloqueadas.

Este README permanece mesmo quando não houver tarefas abertas. O histórico do produto continua
no [plano de desenvolvimento](../development-plan.md), nas
[decisões de produto](../product-decisions.md) e nas [ADRs](../adr/).
