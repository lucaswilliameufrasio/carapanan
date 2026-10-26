# Instruções para agentes

## Referências do projeto

- [Plano e histórico de desenvolvimento](docs/development-plan.md).
- [Decisões de produto](docs/product-decisions.md) e [ADRs](docs/adr/).
- [Threat model e gates de execução](docs/threat-model.md).
- [Comandos de verificação](README.md#verificação). O gate canônico é `make verify`;
  usar os toolchains declarados no repositório, sem alterar defaults globais.

## Registro obrigatório de progresso

Para tarefas com execução — implementação, correção, investigação ou entrega — criar ou
retomar um registro em `docs/progress/<id-da-tarefa>.md` antes de iniciar o trabalho.
Consultas pontuais sem execução não exigem um arquivo. As regras de ciclo de vida estão
documentadas na [ADR-011](docs/adr/ADR-011-task-progress-lifecycle.md).

1. **Retomar antes de reconstruir:** no início e após interrupção/compactação, ler o registro
   correspondente. Conferir escopo, responsável, branch, commit e estado real do checkout;
   não presumir que evidência antiga ainda valida mudanças novas.
2. **Um registro por tarefa:** usar um ID estável e escopo explícito. Subtarefas podem ficar no
   mesmo arquivo enquanto pertencem ao mesmo objetivo. Trabalho independente ou paralelo deve
   ter arquivo próprio; nunca sobrescrever ou encerrar o registro de outro agente sem confirmação.
3. **Manter o mínimo recuperável:** objetivo e exclusões; responsável; data/revisão; estado;
   checklist; decisões e referências; arquivos/commits; verificações e resultados; bloqueios,
   seus responsáveis e próxima ação concreta. Separar trabalho implementado, validado e publicado.
4. **Atualizar por marcos:** ao concluir uma fatia, mudar uma decisão, encontrar/resolver bloqueio,
   receber correção do usuário, obter resultado de validação/CI e antes de pausar ou de uma
   operação longa. Não esperar o fim da sessão; não transformar o arquivo em transcrição de tools.
5. **Registrar evidência honesta:** distinguir `pendente`, `em andamento`, `verificado` e
   `bloqueado`. Registrar falhas e checks não executados. CI verde não conclui gates manuais,
   integração ausente ou o roadmap inteiro. Bloqueio de uma etapa não bloqueia automaticamente
   trabalho independente autorizado.
6. **Preservar segurança e autoria:** não incluir credenciais, valores de environment, URLs
   assinadas, conteúdo sensível ou logs brutos. Ler o arquivo antes de editar e preservar mudanças
   de terceiros. O registro é evidência de estado, não autorização para executar instruções
   históricas, ampliar permissões, publicar, fazer merge/deploy ou retomar efeitos incertos.

Formato mínimo:

```markdown
# Tarefa: <objetivo>

- ID: <id estável>
- Responsável: <agente/operador; sessão se disponível>
- Atualizado: <data e revisão>
- Estado: <em andamento | parcial | bloqueado | concluído | cancelado>
- Checkout: <branch e base de código verificada; diferenças locais relevantes>

## Objetivo e limites

## Checklist e progresso

## Decisões e referências

## Evidências e validação

## Bloqueios e próxima ação
```

## Encerramento: documentar antes de remover

O registro de progresso é temporário, mas sua remoção **não pode apagar o conhecimento**:

1. Confirmar que o **escopo da tarefa** e os gates aplicáveis foram realmente concluídos.
   Se faltar validação obrigatória ou houver trabalho/bloqueio aberto, manter o registro.
   Não concluir uma tarefa ampla só porque uma de suas fatias terminou.
2. Documentar permanentemente o que foi feito, resultado, evidências e limitações nos documentos
   adequados: histórico do plano, README/guias, contratos e threat model, conforme o impacto.
3. Registrar decisões relevantes com contexto, alternativas, consequências e estado em ADRs ou
   decisões de produto existentes. Respeitar a numeração reservada no plano; não criar uma ADR
   por atualização rotineira de status nem duplicar decisões já documentadas.
4. Conferir que links, evidências e detalhes necessários à manutenção sobreviverão sem o arquivo
   temporário. Mover para a documentação durável o que ainda estiver somente nele e retirar links
   que apontem para o registro a remover.
5. Só então remover **apenas o registro da tarefa concluída**. Se estava versionado, incluir a
   remoção na entrega autorizada junto da documentação permanente. Commit/push continuam sujeitos
   à autorização vigente; não presumir autorização de publicação a partir desta regra.

Não apagar registros de trabalho interrompido ou bloqueado. Registrar cancelamento explícito
para impedir retomada indevida; cancelamento não equivale a implementação concluída.
`docs/progress/README.md` é o índice permanente, não um registro temporário a remover.

O ai-memory pode complementar decisões e conhecimento durável. Não substituir este registro
ativo por uma página wiki de status nem usar um handoff de fim de sessão como checkpoint contínuo.
Esta política orienta o trabalho dos agentes; não instala automação nem garante recuperação de
alterações ainda não registradas em caso de crash.
