# Carapanã — threat model e gates de execução

**Data:** 21 de outubro de 2026. **Escopo:** estado atual do Delivery 4.

Este documento distingue primitives verificadas de garantias de um executor ainda não implementado.
Ler junto de [PD-23–33](product-decisions.md) e do [plano](development-plan.md#158-delivery-4--core-coding-tools).
Não constitui aprovação para executar comandos, compartilhar contexto, fazer deploy ou desabilitar sandbox.

## Atores, ativos e fronteiras

- **Operador local:** escolhe workspace, tarefa e ações; aprova escopo exato. Texto de chat não equivale
  a aprovação nem transforma dados recebidos em política.
- **Repositório e dependências:** podem ser maliciosos, conter symlinks, configs, scripts, nomes e conteúdo
  secretos ou instruções de prompt injection. Confiança no workspace não autoriza cada efeito.
- **Provider/MCP/hooks:** saídas, erros e solicitações são dados não confiáveis. `ToolCallRequested`
  permanece inerte. Configuração do projeto não pode ampliar privilégios nem ativar extensões sozinha.
- **Daemon/CLI e armazenamento:** responsáveis por validação, isolamento de conta, lifecycle e preservação.
  IPC não é uma autorização genérica de execução. IDs de artifacts não são capabilities.
- **Processos externos e concorrência:** outro editor/processo pode modificar paths entre verificações;
  locks advisory coordenam somente participantes cooperativos, não processos hostis do mesmo UID.
- **Host e instalação:** kernel, executáveis de sistema e conta do usuário fazem parte da base de confiança.
  Comprometimento de root/kernel/binários do backend não é resolvido por um probe da aplicação.

Ativos protegidos: credenciais; código e histórico; alterações não salvas/anteriores; decisões de aprovação;
conteúdo e catálogo de artifacts; isolamento de processo/rede; recursos do host; identidade do workspace.

## Jornada mínima antes de execução real

1. Receber intenção como dados e resolver o workspace/tarefa pelo estado confiável da aplicação.
2. Construir uma proposta imutável: executável, argv, cwd, mounts, ambiente, rede, limites e finalidade.
3. Classificar acesso/conteúdo e minimizar arquivos/credenciais disponíveis; rejeitar escopo inseguro.
4. Aplicar política, confirmar concorrência/alterações e obter aprovação vinculada à proposta exata.
5. Revalidar identidade/estado e enforcement no ponto de efeito; falhar fechado se mudarem.
6. Executar sob supervisor com captura limitada, cancelamento e orçamento de tempo/processos.
7. Classificar/redigir output antes de UI/log/artifact/provider. Preservar o trabalho anterior e reportar
   resultados incertos sem retry automático de efeitos.

Somente etapas implementadas e testadas podem ser ligadas. Um probe positivo não implementa essa jornada.

## Matriz de ameaças e evidência atual

| Ameaça                                      | Mitigação existente e evidência                                                                                                        | Lacuna/gate bloqueante                                                                                      |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Traversal/symlink/substituição do workspace | `carapana-storage::workspace`: acesso relativo a descritores, `NOFOLLOW`, identidade, limites; testes reais de substituição e symlinks | Revalidar no futuro ponto de efeito; não há transação filesystem global                                     |
| Segredos em nomes protegidos                | Leitura/search/snapshot recusam nomes conhecidos, inclusive env e credenciais                                                          | Denylist de nomes não classifica segredos em conteúdo/output; não montar o repo inteiro para comandos       |
| Config maliciosa ampliando poderes          | `WorkspaceTrust`, grants exatos por action/resource/task, precedência de config; testes em `carapana-core`                             | Ainda modelam decisões; enforcement de execução não está ligado                                             |
| Prompt injection de provider/tool/MCP       | `ToolCallRequested` é evento de dados, sem executor; testes de provider                                                                | Não interpretar instruções embutidas como autorização; preservar fronteira ao introduzir executor           |
| Aprovação reaproveitada ou obsoleta         | Grants `Once` não clonáveis, task/expiry/action/resource exatos; invalidação de approval pendente                                      | Vincular executável/argv/cwd/mounts/rede e conteúdo de edição à aprovação real                              |
| Edição sobrescrevendo trabalho              | Snapshot, comparação de bytes/metadata e rename relativo, recusa de metadata não preservável                                           | Não é compare-and-swap contra processos externos; concorrência precisa confirmação própria                  |
| Exfiltração filesystem/rede por comando     | Probe Bubblewrap fixo verifica readonly, namespaces e loopback isolado, ambiente limpo                                                 | Não há launcher para comandos gerais, política de mounts mínimos nem controle de saída para provider        |
| Elevação de privilégios                     | Probe exige backend root-owned sem setuid/gravação ampla, `NoNewPrivs=1` e capabilities zeradas                                        | Verificação completa no processo de execução; seccomp e superfície de syscalls ainda não comprovados        |
| Sandbox ausente ou parcialmente funcional   | Probe retorna erro sem fallback; macOS é unsupported; gate opt-in exige prova positiva                                                 | Não reportar proteção do runtime a partir de tests que aceitam indisponibilidade segura                     |
| Processo órfão/fuga de lifecycle            | Supervisor de grupos próprio, host com pipe de controle e testes de shutdown/restart/descendentes                                      | Integrar o futuro launcher ao supervisor, budgets, cancelamento e isolamento; não matar por nome/PID alheio |
| Output/RAM/disco ilimitados                 | Store reserva quota, stream limitado, SHA-256, item count e cleanup bounded; probe limita pipes/tempo                                  | Captura streaming de stdout/stderr de comandos gerais ainda não existe; não usar buffer ilimitado           |
| Crash/catálogo e arquivos divergentes       | Estados pending/ready/deleting, quota de órfãos, graça de sete dias, retries de ID limitados; testes reais                             | Não prometer atomicidade DB+filesystem; metadata durável ausente/corrompida é preservada e reportada        |
| Um catálogo apagando blobs de outro         | Store aceita só `sessions.sqlite3` por diretório privado; teste de recusa antes de I/O                                                 | Não compartilhar diretório de blobs entre catálogos independentes nem fazer restore isolado sem revisão     |
| Exposição em diagnósticos/logs              | Probe imprime somente resultado fixo; raw stdout/stderr de falha não sai no CLI; tipos sensíveis evitam Debug onde documentado         | Revisar erros e todos os novos sinks; proteção de conteúdo não é inferida de privacidade do diretório       |
| Dispositivo pareado roubado/rede remota     | IPC local privado e nenhuma execução remota deste slice                                                                                | Pairing/revogação, políticas globais e transporte remoto pertencem a deliveries futuros                     |
| Supply-chain/update comprometido            | Lockfile, dependências exatas existentes, CI de build/test                                                                             | Assinatura/instalação/rollback e verificação de binários não estão implementados como fluxo de update real  |

## Prova positiva e testes de recusa

```sh
TMPDIR=/var/tmp CARAPANA_REQUIRE_BUBBLEWRAP=1 mise exec -- make verify
cargo run -p carapana-cli -- sandbox-probe
```

A variável `CARAPANA_REQUIRE_BUBBLEWRAP` faz os testes falharem se não houver enforcement real no Linux.
Sem ela, a suíte verifica também a recusa segura de backend ausente/restrito; verde nesse modo não comprova
enforcement no runner. O probe monta somente seu fixture sintético, usa output de até 4 KiB por pipe e
deadline de cinco segundos. Não usa inferência, contexto real, rede externa, sudo ou reconfiguração do host.

Testes de storage usam SQLite e filesystem reais. Fixtures simulam estados de crash; isso não equivale
a campanha de power-loss, auditoria independente ou prova contra atacantes com acesso de escrita do mesmo UID.

## Condições para manter bloqueado

- Qualquer gate de autorização, segredo, concorrência, sandbox, recursos ou cancelamento sem prova.
- Mudança da proposta após aprovação; erro/indisponibilidade do backend; downgrade silencioso.
- Resultado de escrita/commit incerto sem inspeção; falta de capacidade sem alternativa segura.
- Necessidade de ampliar mounts/rede/credenciais ou escapar do diretório aprovado sem novo consentimento.

## Próximas fatias de implementação

1. PD-34 implementa o contrato puro de proposta imutável, aprovação exata e revogação. Integrar origem
   autenticada do evento UI/IPC, todos os campos de isolamento/recursos e revalidação no ponto de efeito.
2. PD-36 implementa staging limitado dos arquivos explícitos e verificação de manifest/inventário;
   integrar autenticação da seleção, política de conteúdo/segredos e mount read-only seguro, sem HOME/config implícitos.
   PD-37 acrescenta reserva global compartilhada com artifacts e recovery de staging, sem transformar
   a quota lógica em hard quota do filesystem nem proteger contra escrita hostil fora das APIs do mesmo UID.
   PD-38 valida um manifesto explícito de arquivos de runtime, fixando descritores após checar origem
   root-owned, traversal sem symlinks, orçamento e SHA-256 esperado. Ainda falta vincular esse manifesto
   à aprovação e montar por descritores; não há descoberta de dependências nem confiança automática
   no conteúdo só porque o hash corresponde. Atualizações in-place por root continuam fora da garantia.
3. PD-35 implementa tails e orçamento agregado isolados; integrar leitura bounded antes de alocar chunks,
   timeout/cancelamento e budgets ao supervisor/backend sem renderizar ou persistir output bruto.
   PD-40 implementa leitores Unix nonblocking com chunks limitados/deadline/cancelamento em pipes reais.
   Ainda cabe ao supervisor consumir o motivo de parada, terminar o grupo próprio e recolher processos;
   o leitor não transforma EOF em prova de término nem faz classificação de segredos.
   PD-41 integra essa sequência ao guardian privado existente para captura interna: término do grupo,
   espera do host, timeout/cancelamento/EOF e transporte de tails bounded têm testes reais sintéticos.
   Não há integração com provider/tools; fuga de sessão/grupo e orçamento agregado do workload ainda
   exigem namespaces/cgroup no launcher, sem extrapolar a garantia atual do guardian Unix.
   PD-39 prepara controles de memória/PIDs em cgroup v2 explicitamente delegado. A validação inicial
   sem pai configurado comprovou somente recusa segura, não criação/readback ou contenção de workloads.
   A prova opt-in transitória autorizada em 22/10 passou depois: criação/readback/remoção, bloqueio real
   de forks e OOM kill foram comprovados em fixtures finitos admitidos por gate byte. O launcher geral
   ainda precisa de handshake antes do exec e retenção/recolhimento de seus recursos, sem assumir que
   a memória do interpretador confiável anterior à migração foi contabilizada no grupo.
   PD-43 adiciona admissão única de Child próprio e cgroup.kill por descritor com deadline. Testes reais
   comprovam término após setsid e recusam matar população inesperada sem admissão da lease. O contrato
   de filho bloqueado ainda precisa ser implementado pelo guardian antes de aceitar workloads gerais.
4. Execução local explicitamente aprovada e testes adversariais reais; somente depois conectar tools.
5. Git/worktree operacional e aplicação de edições com os mesmos gates, preservação e evidência de validação.
   PD-42 integra aprovação tipada de uma edição ao snapshot/diff e à escrita atômica existente. O efeito
   consome o token sem novos paths/bytes, revalida escopo/revogação e preserva conflitos observados. Ainda
   exige origem autenticada de eventos, tratamento de conteúdo/segredos e UX de revisão; não há tool nova.

Revisar a matriz antes de cada wiring, ampliação de privilégios, backend novo, acesso remoto ou mudança de
retenção/backup. Nenhuma etapa acima autoriza merge/deploy ou envio de contexto real a um provider.
