# Carapanã — decisões de produto e comportamento

**Data:** 4 de outubro de 2026\
**Status:** decisões aprovadas pelo responsável pelo produto\
**Escopo:** complemento normativo do [plano de desenvolvimento](development-plan.md)

Este documento registra a revisão do plano. Não declara integração real comprovada.
Delivery 0 aprovado explicitamente em 6 de outubro de 2026 e Delivery 1 autorizado;
PR #1 integrada em `main`. Ler os dois documentos juntos; regras
específicas aqui definidas prevalecem sobre exemplos anteriores incompatíveis do plano.
O gate do Delivery 0 foi cumprido com mocks e aprovação do conjunto. As entregas
seguintes continuam seguindo seus próprios escopos e gates do roadmap.

## PD-01 — Delivery 0 e aprovação

- Protótipos interativos com mocks de CLI, TUI, web desktop e mobile.
- Nada de runtime, provider real, daemon, SQLite, tools reais, execução de tarefas,
  sandbox ou integração MCP real. Rodar os próprios protótipos/testes de UI não equivale
  a habilitar execução pelo agente.
- Seletor de cenários simulados: execução, fila, intervenção, approvals, cota esgotada,
  autenticação/falha de provider, modelo/variante indisponível, erro de MCP, crash/recovery,
  desconexão e pressão de recursos.
- CLI headless simulada: saída legível, JSON e códigos de saída para sucesso, falha,
  validação incompleta e approval pendente.
- pt-BR por padrão, com textos separados da interface para inglês depois. Tradução
  inglesa completa não é gate inicial. Comandos/identificadores técnicos permanecem estáveis.
- Temas claro e escuro, seguindo o sistema por padrão, com seleção manual.
- Entrega integrada e navegável, com roteiro curto e checklist dos critérios do plano.
  Revisões parciais são possíveis, mas não liberam runtime.
- Aprovação do conjunto CLI/TUI/web/mobile. Depois da apresentação, parar e aguardar
  autorização explícita, mesmo se houver fundação técnica pendente.

## PD-02 — Primeiro dogfooding e ChatGPT

Depois do gate do Delivery 0, priorizar CLI/TUI, assinatura ChatGPT, edição/execução
seguras, sessões persistentes, MCP/skills (ai-memory prioritário) e validação.
Web/mobile continuam obrigatórios no protótipo, mas sua conexão real não bloqueia esse
primeiro uso. Providers adicionais e subagents também ficam para etapas posteriores.

O operador inicial usa ChatGPT Pro de US$100: referência de teste, não garantia de
elegibilidade, modelos, cota ou acesso ilimitado. A OpenAI documenta uso do plano por
ferramentas open-source e projetos pessoais locais, com autorização própria da aplicação,
sem API key. Confirmar acesso por inferência real após o Delivery 0.

Verificar novamente a documentação oficial na implementação:

- [Sign in with ChatGPT](https://developers.openai.com/siwc)
- [Integração open-source](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt)
- [Registro e autenticação](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)

Não reaproveitar credenciais do Codex, endpoints privados ou scraping. Não confundir
assinatura com cobrança de API. Adapter de API pode auxiliar desenvolvimento mediante
autorização explícita, mas API paga não é requisito nem fallback automático do produto.

### Gate do primeiro dogfooding

Validar no macOS e Linux, com a assinatura ChatGPT e sem perda de trabalho:

1. Corrigir um bug, reproduzi-lo e validar com testes.
2. Implementar uma feature seguindo as convenções do projeto.
3. Retomar sessão longa preservando contexto, fila e autorizações ainda válidas.

São objetivos condicionados à viabilidade, não promessa de capacidades não testadas.
Na sequência atual, esse marco depende dos Deliveries 1–9 e das proteções antecipadas
de PD-17, não apenas de conectar um modelo.

## PD-03 — Perfis e seleção para a próxima mensagem

Perfis podem ser criados, renomeados, removidos e reordenados. Cada um guarda modo de
trabalho, autonomia, provider, modelo e variante de reasoning próprios.

| Preset    | Trabalho                                                    | Autonomia                                                   |
| --------- | ----------------------------------------------------------- | ----------------------------------------------------------- |
| Planejar  | Investigar e planejar, sem alterações ou efeitos colaterais | Pedir acesso quando necessário                              |
| Perguntar | Executar                                                    | Pedir approval para ações com efeitos colaterais            |
| Auto      | Executar                                                    | Policy determina executar, negar ou pedir approval          |
| Yolo      | Executar                                                    | Autonomia ampliada dentro do escopo e das proteções rígidas |

- Presets começam com o modelo escolhido no onboarding e variante `default`, quando
  suportada. Não fingir suporte a variante inexistente. Depois, seleções são independentes.
- `Shift+Tab` percorre a ordem configurada; oferecer atalho alternativo.
- Mudar perfil/modelo/variante só configura a próxima mensagem; sem enviar, não muda
  o agente em execução.
- Mostrar separadamente `Executando: perfil · modelo · variante` e
  `Próxima mensagem: perfil · modelo · variante`.
- Voltar ao perfil recupera sua última seleção na sessão.
- Mudanças de modelo/variante são salvas automaticamente só na sessão atual.
- `Salvar como padrão deste perfil` altera explicitamente defaults gerais.
- Novas sessões começam em Perguntar até o usuário escolher outro padrão.
- Sessões retomadas preservam seleções, sem significar retomada automática.
- Planejar não implementa sozinho. Em Executar, apresentar plano não exige aprovação
  extra; ações seguem as permissões efetivas.

## PD-04 — Fila e intervenção

- Envio normal entra na fila, sem interromper o trabalho atual.
- Cada mensagem captura texto, perfil, provider, modelo e variante no envio. Seleções
  posteriores no composer não alteram mensagens enfileiradas.
- Permitir editar texto e seleção explicitamente, remover e reordenar antes do processamento.
- Distinguir `Na fila`, `Processando` e `Já enviada/processada`.
- A transição para processamento fixa o conteúdo. Se uma edição chegar depois,
  preservar o texto e oferecer novo envio ou `Intervir agora`; nunca descartá-lo.
- `Intervir agora` prioriza a mensagem e aplica seu perfil na próxima etapa segura,
  sem presumir segurança de interromper uma tool no meio. Mostrar intervenção pendente.
- Intervenção invalida approvals pendentes; ações ainda necessárias são reavaliadas.
- Mensagem em Planejar enviada normalmente espera sua vez; ao processar, não modifica
  arquivos nem inicia efeitos colaterais.
- Avançar automaticamente após conclusão, usando a seleção da próxima mensagem.
- Falha, dúvida, approval pendente ou validação incompleta bloqueiam avanço.
- Persistir texto, ordem, origem e seleção. Após reinício, recuperar em pausa; nada
  executa antes de retomada manual.
- A seleção gravada não é autorização permanente: revalidar permissões, revogações e
  estado dos recursos antes da execução.

## PD-05 — Trabalho, autorizações e validação

- Sessão pode conter vários trabalhos sem botão obrigatório de `Nova tarefa`.
- Continuação pode permanecer no trabalho atual; novo objetivo não herda autorizações
  temporárias anteriores. Ambiguidade exige pergunta antes de reutilizar autorização.
- Autorizações `current task` expiram quando o trabalho termina.
- Implementar feature não implica commit, push, publicação ou deploy.
- Commit/push exigem pedido ou autorização para o trabalho atual, inclusive em Yolo.
- Validação incompleta aparece como `Implementado, validação incompleta`, com evidências
  e motivo do que faltou, nunca como sucesso validado.
- Esse resultado pausa a fila; operador decide continuar ou tentar outra validação.

## PD-06 — Recovery, parada e checkpoints

- Crash/reinício recupera em pausa, mostrando onde parou.
- Não repetir automaticamente push/deploy/operação com resultado incerto; verificar antes.
- Retomada revalida permissões e arquivos. Após espera longa, invalidar leituras antigas,
  reler arquivos afetados, conferir diff e adaptar o plano.
- Conflitos com alterações do operador exigem confirmação, inclusive em Yolo.
- `Parar` interrompe o agente, cancela comandos e encerra processos temporários iniciados
  para o trabalho, com supervisão e encerramento seguro (dev server, API local, serviço de teste).
- Não encerrar serviços anteriores. Processos marcados pelo operador como `Manter rodando`
  permanecem. Parar não desfaz arquivos automaticamente.
- `Restaurar checkpoint` exige prévia do diff e confirmação. Restaurar apenas alterações
  atribuíveis ao agente, sem descartar mudanças anteriores/posteriores do operador.
  Se não for possível separá-las com segurança, mostrar conflito e não forçar rollback.

## PD-07 — Segurança mínima, segredos e privilégios

A camada mínima é bloqueante antes de tools reais, não adiada integralmente ao Delivery 6:
arquivos sensíveis, approvals básicos, limites de diretório, supervisão e consentimento sem sandbox.

### Segredos e ambiente

- Valores secretos nunca vão para modelo, classifier, chat, logs, artifacts, reports
  ou memória externa, inclusive em Yolo.
- Comando especificamente autorizado pode usar credenciais localmente sem revelar
  seus valores ao agente. Autorizar uso não autoriza exposição.
- Não oferecer `secret.read_value` como forma de enviar valor ao modelo.
- `.env.example` também exige permissão antes da leitura. Espera-se nomes/placeholders,
  mas nome de arquivo não garante ausência de credenciais reais.
- Autorizar leitura de arquivo sensível não permite expor valores secretos ao modelo.
- Comandos não herdam todo o ambiente do terminal: só variáveis básicas necessárias;
  variáveis sensíveis exigem autorização específica para uso local.

São requisitos de enforcement, não garantias já comprovadas. Demonstrar isolamento e
proteção de outputs; bloquear fluxos que não puderem ser protegidos. Redaction por
conveniência não substitui essa comprovação.

### Instalações e privilégios

- Sem elevação de privilégios na versão inicial: `sudo`, equivalentes ou outros
  mecanismos de execução como administrador ficam bloqueados, inclusive em Yolo.
- Não pedir, receber ou armazenar senha de administrador.
- Explicar operação administrativa para execução manual fora do harness; verificar
  resultado depois, sem privilégios.
- Auto pode instalar dependências locais necessárias dentro do escopo autorizado e
  proteções efetivas; Perguntar pede approval antes de instalar.
- Instalações globais/mudanças no sistema exigem autorização específica, inclusive em
  Yolo. Essa autorização não libera elevação de privilégios.

### Sandbox indisponível

- Permitir conversa/investigação segura conforme permissões.
- Bloquear execução/alterações até autorização explícita para trabalhar sem isolamento.
- Aviso permanece visível; sem fallback silencioso, inclusive em Yolo.
- Sem implementação segura de um recurso em determinado OS, deixá-lo indisponível
  com limitação explícita. Consentimento sem sandbox não libera riscos não controláveis.

## PD-08 — Pasta da sessão, concorrência e ações destrutivas

Pasta-limite é o diretório de abertura, não a raiz Git. Abrir em `repo/backend` não
libera ações destrutivas automaticamente em `repo/frontend`. Mostrar o limite;
ampliá-lo exige escolha explícita do operador.

### Worktrees e alterações anteriores

- Worktree é escolha do operador, não obrigação.
- Investigações podem ocorrer em paralelo. Antes de editar simultaneamente no mesmo
  diretório, mostrar sessões existentes e pedir confirmação para compartilhar.
- Identificar sessões por nome, caminho, estado e tempo, permitindo abri-las.
- Oferecer `Usar worktree`, `Compartilhar este diretório` e `Voltar`.
- Confirmação vale para a convivência apresentada, não globalmente. Nova sessão recebe aviso próprio.
- Registrar estado inicial e distinguir mudanças anteriores das feitas pelo agente.
- Detectar arquivos alterados desde a leitura, inclusive por outras sessões.
- Conflitos/sobrescrita de mudanças anteriores exigem confirmação, inclusive em Yolo.

### Ações destrutivas e permissões persistentes

- Perguntar/Auto pedem confirmação específica para ações potencialmente destrutivas,
  mostrando alvo e impacto.
- Yolo pode executá-las dentro da pasta da sessão, sem dispensar proteção de segredos,
  conflitos com trabalho anterior ou bloqueios de ações claramente catastróficas.
- Fora da pasta, Yolo também pergunta: `Permitir uma vez`, `Permitir neste trabalho`,
  `Permitir sempre`.
- `Permitir sempre` vincula a ação ao caminho específico, não a todos os caminhos externos
  ou tipos de ação. Para pastas, inclui conteúdo/descendentes explicitamente informados.
- Symlinks não estendem acesso a alvos externos: verificar o alvo efetivo.
- Permissões persistentes ficam visíveis/revogáveis nas configs, separadas das temporárias.

## PD-09 — Confiança em projetos, MCPs e hooks

- Confiança explícita por workspace antes de ativar configs/MCPs/hooks do repo.
- Antes disso, inspecionar arquivos comuns conforme policy, sem executar automaticamente
  comandos/extensões do projeto.
- Mesmo confiável, projeto não amplia permissões, libera caminhos ou desativa proteções
  sem aprovação do operador.
- Mudanças que introduzam execução ou ampliem acesso exigem nova confirmação com diff;
  tema/modelo padrão e outras preferências comuns não renovam confiança.
- Adicionar MCP não autoriza todas as tools.
- Tools novas ou mudanças de parâmetros/capacidades que ampliem acesso exigem nova
  aprovação. Tools inalteradas preservam permissões.
- Hooks seguem as mesmas permissões, sandbox, supervisão, proteção de segredos e budgets
  que as tools; ativá-los não concede acesso irrestrito.

## PD-10 — Providers, capacidades, retries e cota

### Compartilhamento e configurações incompatíveis

Contexto compartilhável é o necessário para continuar: mensagens, plano, trechos de
código e resultados relevantes de tools. Não implica enviar outras sessões/repo inteiro
e nunca inclui valores secretos.

- Primeira troca para outro provider explica esse compartilhamento e pede confirmação.
  Trocar modelo no mesmo provider não exige esse aviso novamente.
- `Não perguntar novamente para este provider nesta sessão` dispensa por sessão/provider.
- Dispensa revogável; configuração global pode desativar confirmações, não proteção de segredos.
- Provider de mensagem enfileirada passa pela mesma confirmação antes de enviar contexto,
  quando ainda não autorizado.
- Modelo salvo indisponível pausa, sem substituição silenciosa. Operador escolhe se
  troca vale para mensagem, perfil na sessão ou padrão geral.
- Variante incompatível exige escolha entre as suportadas; sem fallback silencioso
  para `default`, nem exibição de variante que o backend não aplica.

### Retries e renovação

- Falhas temporárias do provider: não Yolo tem até três retries com espera crescente,
  estado visível e cancelamento; depois pausa.
- Yolo tem retries sem limite de quantidade até cancelar, com espera crescente limitada
  e respeito às instruções de retry do provider. Sem chamadas em loop rápido.
- Não autoriza repetição cega de comandos com efeitos colaterais.
- Autenticação inválida/ação negada não são falhas temporárias solucionáveis por retry.
- Cota esgotada interrompe inferências; nenhum fallback pago automático.
- Se provider informar renovação, mostrar contagem/horário e aguardar.
- Yolo retoma automaticamente após renovação informada, salvo cancelamento.
- Outros perfis oferecem `Retomar quando a cota voltar`; sem escolher, ficam pausados.
- Sem horário fornecido, não inventar previsão/promessa de retomada nesse horário.
- Retomada revalida arquivos/permissões. Crash/reinício prevalece: recuperar em pausa
  mesmo se antes aguardava renovação com retomada automática.

## PD-11 — Proteção contra doom loops

Proteção obrigatória contra desperdício de cota, inclusive em Yolo. Retry temporário
do provider não é ciclo de edição/teste sem progresso.

Defaults configuráveis:

1. Três tentativas semanticamente equivalentes sem progresso: interromper estratégia,
   diagnosticar, revisar hipóteses e escolher outra abordagem.
2. Mais três sem progresso após isso: pausar e pedir intervenção.

Só evidência real de avanço reinicia o contador. Alegar estratégia nova/variar
superficialmente um comando não basta. Pausa impede avanço da fila; mostrar padrão e evidências.

## PD-12 — Contexto e ai-memory

- Compactação automática com evento visível, preservando objetivo, plano, autorizações
  e trabalho pendente; não apaga histórico local.
- Se informação essencial não puder ser preservada com segurança, pausar.
- ai-memory é MCP prioritário, sem memória proprietária concorrente.
- Consultar memória relevante no início/retomada, quando autorizado.
- Hooks compatíveis antes de compactação/no encerramento, com timeout e payload sanitizado.
- Gravações automáticas autorizáveis uma vez por integração/escopo de projeto.
- Exclusões/gravações fora do escopo exigem autorização separada.
- Falha em hook não apaga histórico nem impede encerrar; mostrar aviso.
- Indisponibilidade permite contexto local, com indicação de memória degradada;
  se faltar informação essencial, perguntar em vez de inventar.
- Memória recuperada é referência não confiável, nunca autorização para ações, policy ou privilégios.
- Nunca enviar valores secretos à memória.
- Sessão, fila, plano e autorizações permanecem no Carapanã. Memória complementa contexto,
  não substitui persistência nem torna válidas autorizações expiradas.

## PD-13 — Multi-device, desconexão e mobile

- Clientes autorizados enviam mensagens/intervêm/param conforme permissões.
- Fila única com origem das mensagens; estado/intervenções sincronizados, sem device dono.
- Celular recebe por padrão visualização, prompts, intervenção/parada e approvals comuns.
- Políticas globais, acessos permanentes e consentimento sem sandbox desativados por padrão,
  habilitáveis explicitamente por device. Segredos continuam não reveláveis.
- Desconexão bloqueia comandos/approvals até snapshot atualizado.
- Preservar rascunhos, sem enfileirar ações offline para envio silencioso ao reconectar.
- Mobile inicial: attention queue destacada e avisos com PWA aberta.
- Notificações com PWA fechada são evolução posterior, condicionada a suporte,
  infraestrutura e segurança; não implementar/prometer push no Delivery 0.

## PD-14 — TUI, SSH e headless

- Conversa principal com atividade resumida integrada; logs brutos não dominam.
- Plano/diff/validação/logs completos sob demanda; painel lateral em terminal largo.
- Todas as ações por teclado via SSH, sem depender de mouse, truecolor ou teclas
  especiais. Documentar alternativas a `Shift+Tab`.
- Mínimo `80×24`, reorganizando painéis sem esconder approvals/ações essenciais;
  abaixo disso, orientar ampliar o terminal.
- Validar protótipos CLI/TUI também via SSH, sem runtime no Delivery 0.
- Headless não interativo: approval pausa sessão e retorna resultado estruturado
  `Aguardando aprovação`, sem aprovar automaticamente nem esperar indefinidamente.
  Intervenção por outro cliente/retomada posterior permanece possível. Formatos/códigos
  de saída são especificados nos protótipos.

## PD-15 — Subagents e recursos

Na etapa posterior de subagents:

- Perguntar pede approval para delegar; Auto/Yolo delegam dentro de budgets/permissões.
- Planejar usa só investigação, sem alterações.
- Não ampliar privilégios nem criar worktree contra a escolha do operador; concorrência segue PD-08.
- Sem capacidade para subagent, continuar sozinho ou esperar mostrando limitação;
  nunca aumentar budgets automaticamente.
- Só pausar se continuar ultrapassaria orçamento e não houver alternativa segura.

Limites dinâmicos consideram memória disponível/pressão atual, não apenas RAM total.
Medir harness e processos externos (builds/testes/MCPs) separadamente, mas controlar
o total supervisionado. Não confundir consumo de memória com cota/tokens do provider.
Metas numéricas dependem de benchmarks; os exemplos conceituais do plano não são metas aprovadas.

## PD-16 — Plataformas, distribuição e licença

| Ambiente                        | Hardware                                                   | Papel                                         |
| ------------------------------- | ---------------------------------------------------------- | --------------------------------------------- |
| macOS                           | M3, 16 GB de RAM                                           | Referência principal de eficiência/uso diário |
| Linux local                     | Ryzen 9 7900, 12 núcleos/24 threads, aproximadamente 64 GB | Host mais potente                             |
| Linux remoto `tirion-tailscale` | i5-8265U, 4 núcleos/8 threads, aproximadamente 16 GB       | CPU limitada e SSH                            |

- Tirion deve suportar confortavelmente uma sessão ativa com paralelismo adaptado.
- Não exigir mesmo número de agentes em todas as máquinas.
- Operador testará macOS/Linux dentro e fora do [Herdr](https://github.com/herdrdev/herdr),
  ambiente de teste, não integração/dependência do produto.
- Benchmarks reais só após Delivery 0; nessa entrega, testar apenas protótipos.
- Usuário final não precisa de Rust/Node para o próprio Carapanã. Dependências de
  projetos/MCPs são separadas, explicadas pelo Doctor, sem instalação silenciosa.
- Licença escolhida: **Apache-2.0**. Incluir licença/notices na distribuição e revisar
  compatibilidade de dependências/SDKs.

## PD-17 — Dependências do roadmap e comprovação técnica

### Sequência e gates

| Marco              | Decisão aplicada                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------------------ |
| Delivery 0         | Toda UX simulada; entrega integrada; nenhuma integração real                                           |
| Delivery 1         | Contratos de perfis/fila/resultados/permissões; ainda sem autonomia real                               |
| Delivery 2         | Persistência da fila, recovery em pausa e supervisão                                                   |
| Delivery 3         | Auth oficial/inferência; comprovar capacidades/cota sem tools executáveis                              |
| Delivery 4         | Implementar/validar segurança mínima antes de ativar tools                                             |
| Delivery 5         | Contexto/compaction/instruções sem retirar proteções antecipadas                                       |
| Delivery 6         | Auto/policy/sandbox/classifiers avançados sobre enforcement mínimo                                     |
| Deliveries 7–9     | Skills, completion, doom-loop protection e MCP/ai-memory; primeiro dogfooding só com gates satisfeitos |
| Deliveries 10–11   | Remote/web/mobile reais reaproveitam UX aprovada                                                       |
| Etapas posteriores | Providers adicionais, subagents e investigação de notificações com PWA fechada                         |

Proteções mínimas, inclusive segredos, não aguardam uma skill de prompt no Delivery 7.
Autonomia só é liberada quando controles correspondentes funcionam. Essas dependências
não autorizam antecipar runtime ao Delivery 0.

### Pendências de comprovação

1. **ChatGPT:** conta real, inferência, modelos/variantes, tool calling, limites,
   informação de renovação e elegibilidade dos ambientes locais/host pessoal remoto.
2. **Enforcement por OS:** sandbox, bloqueio de elevação, fronteiras efetivas,
   env/outputs e uso local de segredos sem exposição; bloquear capacidades não demonstráveis.
3. **Fila/concorrência/recovery:** atomicidade, mudanças desde leitura, restauração sem
   perda de trabalho e verificação de efeitos incertos.
4. **ai-memory:** hooks, identidade projeto/sessão, permissões e sanitização; conectar
   MCP não garante integração de lifecycle hooks.
5. **Recursos:** calibrar RAM/CPU/disco/latências/writes/paralelismo nas referências,
   incluindo sessões longas e pressão externa.
6. **Delivery 0:** detalhar tokens/componentes/teclado/estados/cenários/formatos headless
   e roteiro nos protótipos, sem antecipar runtime.
7. **Notificações em background:** suporte PWA/infraestrutura segura em etapa posterior,
   sem introduzir relay/cloud obrigatório como atalho.

### Evidências adicionais de aceitação

- Seleção no composer não muda execução; fila mantém seleção do envio.
- Corrida de edição não perde texto; intervenção invalida approvals.
- Recovery preserva fila em pausa, inclusive após espera por cota.
- Nenhum fallback silencioso de provider/modelo/variante.
- Doom loop pausa em Yolo; retry temporário respeita espera/cancelamento.
- Config/hook/MCP não amplia permissões sozinho.
- Yolo respeita pasta-limite, symlinks, segredos, privilégios e mudanças anteriores.
- Compartilhamento do working tree é explícito, sem descartar trabalho alheio.
- Desconexão não envia approvals antigos; headless não aprova/espera indefinidamente.
- Compactação/falha de memória não apaga histórico nem recria autorização expirada.
- TUI funciona em `80×24`, por teclado e via SSH.

No Delivery 0, evidências são de interação com mocks. Enforcement, integrações,
performance e execução só são declarados validados com testes reais na etapa posterior.
