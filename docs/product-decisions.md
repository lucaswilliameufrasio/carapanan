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
- Verificação metadata-only (identidade, tipo, permissões, tamanho e timestamps) não prova
  que o conteúdo permaneceu igual e nunca libera retomada sozinha. O fingerprint limitado abaixo
  é a única leitura/hash aprovada nesta fatia; expansão exige decisão própria.
- Fingerprint de conteúdo aprovado somente para paths relativos explicitamente selecionados:
  SHA-256 local de arquivos regulares até 1 MiB. Negar nomes iniciados por `.env`, componentes
  `.ssh`/`.aws`, nomes contendo `secret`/`credential` (sem diferenciar maiúsculas) e extensões
  `.pem`/`.key`. Não recursar, seguir symlinks, persistir conteúdo ou enviar bytes para IPC,
  logs, modelo ou memória; persistir apenas o digest. Fingerprint não autoriza retomada sozinho.
- Revisão read-only revalida somente os paths explicitamente registrados e reporta cada um
  independentemente (`unchanged`, `changed`, `missing`, `unreadable`, `unsafe`, `too large`,
  `unavailable`). Falha da raiz é global/fail-closed. A revisão não altera sessão, fila ou
  workspace e não limpa a exigência de revalidação. Pelo socket Unix privado do mesmo usuário,
  é permitido expor somente path relativo + status ao CLI/TUI; nunca conteúdo, digest, metadata
  stat ou path absoluto. A visualização não confirma conflitos nem libera retomada.
- Exceção limitada à criação explícita de uma sessão: a raiz absoluta do workspace pode ser
  enviada como input pelo socket Unix privado. O daemon não a devolve em respostas nem a registra
  em logs/erros. Essa exceção não altera o contrato de revisão, que continua expondo somente
  paths relativos e status.
- Política v1 de retomada após recovery: exigir revisão read-only fresca e confirmação humana
  explícita de retomada, inclusive quando todos os paths reportam `unchanged`; metadata e
  fingerprints, isoladamente, nunca autorizam retomada. `changed`, `missing` e `too_large`
  exigem reconhecimento explícito por path. `unsafe`, `unreadable`, `unavailable` e qualquer
  mudança na identidade da raiz bloqueiam retomada até serem resolvidos e revisados novamente.
  O reconhecimento fica vinculado à identidade da raiz e às evidências locais disponíveis
  (metadata stat e digest somente quando já autorizado). Qualquer mudança detectável após o
  reconhecimento invalida a autorização e mantém a sessão pausada; metadata stat-only não
  detecta alteração de conteúdo que preserve todos os metadados. Nunca retomar
  automaticamente nem permitir bypass por Yolo. Conflitos com alterações do operador exigem
  confirmação, inclusive em Yolo.
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

Para recovery, persistir a identidade da pasta-limite como caminho absoluto e o par
device/inode observado no filesystem. Não canonicalizar nem seguir symlinks ao capturar
ou revalidar essa identidade. Se o caminho deixar de existir, contiver symlink ou apontar
para outra identidade, manter a sessão pausada. Esse registro é metadado, não autorização;
não habilita retomada nem amplia permissões.

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

## PD-18 — Hibernação base e limite da supervisão de processos

- Na fundação atual, hibernação é uma transição explícita do operador e altera somente o
  estado persistido. Só pode ocorrer para sessão pausada, sem mensagem ativa e sem recovery
  pendente; fila, seleções e histórico são preservados.
- Acordar uma sessão hibernada sempre volta a `paused`. Nunca inicia mensagens da fila.
- Essa transição não promete liberar working set, caches ou processos: a fundação atual não
  possui esses recursos de runtime. Hibernação automática/reclaim só será implementada com
  sinais de idle/pressão medidos e cleanup comprovado.
- Supervisão efetiva só pode afetar processos criados e identificados pelo Carapanã; processos
  anteriores ou marcados `Manter rodando` não são encerrados. A autorização específica de
  12 de outubro de 2026 cobre somente a primitive definida em PD-19; não habilita, por si,
  provider, tools, execução de tarefas ou autonomia. Contratos/estado inertes não implicam
  capacidade operacional.

## PD-19 — Supervisor local de processos próprios

**Data:** 12 de outubro de 2026

**Status:** aprovado para a fundação da Delivery 2; provider, tools e execução de tarefas seguem
desativados.

- O supervisor é Unix-only nesta entrega e permanece fora dos contratos IPC, CLI e TUI. Ele é
  uma primitive interna para código futuro explicitamente autorizado; enfileirar/retomar mensagem
  não chama o supervisor.
- A API interna permite criar, listar, parar e reiniciar somente identificadores próprios; restart
  encerra o grupo anterior antes de tentar iniciar a nova especificação no mesmo owner session.
- A façade do `DaemonService` exige que a sessão proprietária exista no SQLite e que stop/restart
  usem a mesma sessão registrada no processo. Esses métodos seguem indisponíveis no IPC.
- Executar um binário absoluto já existente com argv separado, sem shell, sem herdar ambiente e
  com stdin/stdout/stderr em `/dev/null`. Limitar quantidade/tamanho de args, caminho e processos
  gerenciados; não registrar executable, args, cwd ou payload de controle.
- Um modo privado do próprio binário Carapanã atua como guardian de cada processo e líder do
  process group dedicado; o executável gerenciado herda esse grupo. O guardian recebe comandos em
  pipes privados e, em stop, SIGINT/SIGTERM ou EOF do daemon, sinaliza o grupo com SIGTERM, aguarda
  800 ms e então SIGKILL (incluindo a si próprio). Permanecer vivo como âncora durante a limpeza
  impede reutilização do PGID enquanto ele envia sinais. O daemon aguarda os guardians por até 3 s.
  Nunca procurar/alvejar processos por nome, workspace ou PID externo.
- `KeepRunning` é uma política explícita de detach: shutdown da sessão/daemon ou saída natural do
  líder não sinaliza o grupo. Uma parada direta e explícita de um identificador ainda controlado
  pode encerrá-lo. Processos preservados não são re-adotados depois. Um gerenciador externo que
  sinalize todos os processos de um cgroup ainda pode afetá-los; a política só controla sinais
  enviados pelo supervisor Carapanã.
- Os processos são identificados somente pela resposta do guardian recém-iniciado e pela relação
  viva de pipes/handles: PID do executável e PGID/ PID do guardian são distintos. Não se restaura
  identidade a partir de PIDs persistidos; após detach ou perda do guardian, o processo não é
  re-adotado nem sinalizado pelo número antigo.
- Trade-off: um guardian extra por processo consome recursos, mas o EOF permite cleanup depois de
  crash do daemon, ao contrário de `Child` direto; um supervisor central seria mais econômico,
  porém teria ownership e reconexão mais complexos. Process groups não contêm descendentes que
  deliberadamente façam detach/setsid, nem sobrevivem a SIGKILL do guardian, encerramento forçado
  do host ou falha da máquina; esses limites não são anunciados como garantias.

## PD-20 — Catálogo puro de capacidades de provider

- A primeira fatia do Delivery 3 mantém o catálogo de provider/modelo/variante como dados puros
  em `carapana-core`, separado de transporte, autenticação, persistência e IPC. O protocolo atual
  continua carregando os identificadores capturados como strings.
- A resolução valida a seleção exata e retorna capabilities declaradas para aquela variante;
  provider/modelo/variante ausentes ou incompatíveis falham explicitamente, sem fallback.
- Capabilities são metadados, não autorização: `tool_calling` não habilita tool executável.
  Esta fatia usa somente catálogos de teste; não realiza auth, chamadas de rede ou inferência.
- Adapter ChatGPT real, uso de credenciais, inferência e prova na conta autorizada continuam
  bloqueados até autorização específica e comprovação conforme PD-02/PD-17.

## PD-21 — Contrato assíncrono de streaming de provider

**Data:** 12 de outubro de 2026

**Status:** aprovado para contrato e testes mock-only; não habilita integração real.

- `carapana-core` define request, eventos e errors tipados em torno de `futures-core::Stream`.
  O core não escolhe nem depende de executor async; adapters futuros fornecem a stream.
- Cancelamento é um token compartilhável e independente de runtime. Adapters devem observar
  seu future de cancelamento e encerrar o transporte/stream; o consumidor pode também descartar
  a stream. A interface não promete retry, reconnect ou cancelamento de trabalho remoto que o
  adapter não consiga interromper.
- Mensagens e seleção resolvida são passadas ao adapter somente por um futuro chamador, que deve
  aplicar a confirmação de compartilhamento PD-10 antes de enviar conteúdo remoto. Esta decisão
  não adiciona wiring de execução.
- Texto e pedidos de tool são eventos de dados não confiáveis. `ToolCallRequested` não executa
  tool nem concede autorização; tools continuam fora desta fase. Errors tipados preservam
  rate-limit e `retry_after`, mas esta camada não repete automaticamente.
- Preferido sobre callback síncrono para manter consumers não bloqueantes e adapters de rede
  assíncronos sem impor runtime ao core. Reavaliar somente se o runtime/adapters aprovados exigirem
  outra política de cancelamento.

## PD-22 — Integração oficial SIWC fora do core

**Data:** 13 de outubro de 2026

**Status:** aprovado para o Delivery 3, com autorização explícita do operador.

- O provider ChatGPT real fica em um crate de integração separado (`carapana-openai`); `carapana-core`
  permanece sem transporte, OAuth, runtime ou dependência de API externa.
- Usar exclusivamente o fluxo open-source oficial Sign in with ChatGPT: dynamic client registration,
  authorization code + PKCE, callback loopback em `127.0.0.1` com `/auth/callback`, validação de
  state/nonce e ID token por JWKS, audience/issuer/expiry, e uso somente quando o token conceder
  `chatgpt.tokens.use.direct`. Não reaproveitar credenciais Codex, endpoint privado ou API key paga.
- Tokens ficam fora do SQLite de sessões, em arquivos separados dentro do diretório privado do
  usuário (`0700` no diretório, `0600` no arquivo), escritos atomicamente; não registrar nem incluir
  credenciais em erros, logs, prompts, contexto ou commits. Esta proteção de filesystem não é
  criptografia em repouso e deve ser informada como limitação.
- Reautorizar uma conta reutiliza o client ID e `ext_agent_host_id` da conta, valida a identidade
  novamente e usa `id_token_hint`/`login_hint` apenas no authorize URL; pedir consentimento novamente
  (`prompt=consent`) exige uma escolha explícita do operador. Refresh usa o client ID emitido,
  `resource=https://api.openai.com/v1`, mantém o grant, grava o refresh-token rotacionado e é
  serializado por conta entre processos. Em erro terminal de refresh, limpar o refresh token inválido,
  conservar o mapeamento da conta e exigir novo sign-in; erros transitórios preservam as credenciais.
- Descoberta de modelos usa o catálogo da conta (`GET /v1/models`, somente itens `visibility=list`).
  Inferência usa apenas `POST /v1/responses`, `store:false`, `stream:true`, sem redirecionamentos;
  só `response.completed` confirma sucesso. O stream observa cancelamento e não reenvia inferência
  automaticamente. `retry_after`, status, códigos, parâmetro, request ID e a forma do erro remoto são
  preservados sem expor corpo bruto.
- O primeiro smoke test real usa apenas um prompt fixo sintético, sem contexto de sessão/workspace.
  A CLI exige `--confirm-remote` antes de enviar esse prompt. O envio de contexto real depende de
  consentimento explícito por provider conforme PD-10. O login oficial e a autorização de escopos no
  navegador não conectam a fila nem iniciam execução.
- Nenhuma tool será enviada ou executada nesta fase. `ToolCallRequested` permanece saída inerte; não
  ligar provider ao daemon, fila, tarefas, subprocessos ou modo autônomo.
- Testes usam cliente de produção contra servidor HTTP local falso; testes normais nunca acessam
  OpenAI nem usam tokens reais. Reavaliar armazenamento se o produto passar a exigir proteção contra
  leitura por outros processos do mesmo usuário ou multiusuário no mesmo host.

## PD-23 — Primitive de leitura limitada do workspace

**Data:** 14 de outubro de 2026

**Status:** aplicada à primitive local isolada; não habilita tools nem envio de conteúdo.

- `carapana-storage::WorkspaceMetadata::read_file` reutiliza a identidade do workspace e a abertura
  Unix com `openat`/`O_NOFOLLOW` já usada pelas observações de recovery, evitando uma segunda
  implementação de resolução de caminhos. A escolha mantém validação e leitura presas à identidade
  capturada; não significa que o crate esteja autorizado a persistir ou compartilhar o conteúdo.
- A operação aceita somente caminho relativo dentro do workspace, arquivo regular e no máximo
  256 KiB. Rejeita symlinks em qualquer componente, caminhos protegidos conhecidos (incluindo env,
  credenciais/configurações, chaves privadas e estado Terraform), troca de identidade/metadados durante
  a leitura e qualquer plataforma sem a primitiva comprovada. Arquivos são retornados como bytes ao
  chamador local; `read_file_with_limit` permite ao chamador impor um limite menor, nunca maior que
  256 KiB. Não há conversão, filtragem de saída ou detecção de segredos no conteúdo.
- A lista de nomes protegidos é defesa conservadora, não classificador completo de segredos. A seleção
  do workspace e o limite de caminho não substituem autorização por ação, revisão de conteúdo, sandbox,
  filtragem de outputs nem consentimento de compartilhamento com provider.
- A primitive não é ligada ao provider, `ToolCallRequested`, daemon, CLI/TUI, sessão ou fila. Não há
  listagem/recursão, busca, escrita, shell ou execução de processos. Esses recursos exigem slices e
  evidências próprias; capacidade do provider continua sendo dado inerte conforme PD-20/PD-21.
- Trade-off: manter a operação junto das primitivas de identidade/recovery evita duplicação de uma
  fronteira sensível e permite testes diretos contra filesystem real. A API não é uma aprovação de
  separar ou ativar o futuro executor de tools; a arquitetura poderá extrair um crate coeso quando
  existirem contratos e capacidades além desta leitura isolada.

## PD-24 — Inspeção local isolada em `carapana-tools`

**Data:** 15 de outubro de 2026

**Status:** implementada como API local não conectada; não ativa execução nem compartilhamento remoto.

- A listagem e a busca pertencem ao novo crate `carapana-tools`; `carapana-storage` continua dono da
  identidade persistida do workspace e da abertura/leitura segura. A extração foi feita quando a
  capacidade deixou de ser uma única primitive. Nenhum dos crates chama provider, daemon, CLI/TUI,
  fila ou executor.
- Os arquivos de regras são lidos por `WorkspaceMetadata::read_file` (abertura Unix sem-follow,
  caminho confinado e limite de bytes) e compilados em memória; o parser não reabre paths. A leitura
  protege a configuração contra symlink/TOCTOU. O walker de diretórios, porém, não é descritor-relativo
  e ainda exige revisão contra troca concorrente de paths antes de qualquer runtime wiring.
- A listagem usa `ignore` com versão exata e lockfile para semântica conhecida. `.ignore` tem
  precedência sobre `.gitignore`; `.carapanaignore` tem precedência sobre ambas. Configuração global e
  regras ancestrais são desativadas para tornar o resultado local ao workspace. Links simbólicos e outros
  filesystems não são percorridos; `.git`, `node_modules`, `target`, `dist`, `build`, `.cache` e
  `vendor` são excluídos por padrão. Profundidade e quantidade de entradas/arquivos são limitadas.
- Busca é literal e somente leitura, limitada a 512 arquivos, 8 MiB lidos e 200 linhas de resultado;
  `read_file_with_limit` aplica o orçamento restante durante cada leitura, além do teto de 256 KiB
  por arquivo e das proteções de `WorkspaceMetadata::read_file`. Busca sem distinção
  de caixa ignora caixa ASCII. Snippets substituem controles de terminal e formatação bidirecional,
  mas ainda são dados não confiáveis e podem conter segredos em arquivos com nomes não reconhecidos;
  consumidores devem escapar caminhos antes de renderizá-los.
- Preferido sobre manter traversal em `carapana-storage`, pois storage não deve possuir indexação ou
  comportamento de ferramentas; também preferido sobre implementar padrões ignore manualmente, para
  não divergir da semântica convencional. A dependência `ignore` usa a licença MIT disponível no
  pacote dual MIT/Unlicense e é fixada no manifesto e no lockfile.
- Listagem/busca não são approvals e não implicam envio ao modelo. Qualquer wiring futuro exige
  contratos de autorização/compartilhamento e prova do gate de segurança de Delivery 4, incluindo
  traversal relativo a descritores em ambientes adversariais; a API permanece isolada enquanto essa
  prova não existir.

## PD-25 — Parser Tree-sitter inicial e sob demanda

**Data:** 15 de outubro de 2026

**Status:** implementado no `carapana-tools`; parser local não conectado a runtime, provider ou UI.

- Conforme escolha explícita do operador, a primeira seleção suporta Rust (`.rs`), TypeScript
  (`.ts`, `.mts`, `.cts`) e TSX (`.tsx`). Arquivos JavaScript e Svelte não são inferidos como TSX nem
  processados por fallback; gramáticas adicionais exigem uma fatia posterior.
- O parser só executa para um caminho explícito e usa o leitor de workspace confinado/sem-follow, com
  limite de 256 KiB, denylist e validação de UTF-8. Parser e caminhada da árvore têm limites de tempo,
  nós e diagnósticos. O resultado contém idioma, tipo da raiz, estado de sintaxe e posições em bytes;
  não contém source snippets, compilação, execução ou validação semântica.
- `tree-sitter` e gramáticas Rust/TypeScript ficam em `carapana-tools`, pinados por versão exata e
  lockfile; todas as dependências adicionadas são MIT. A escolha evita colocar parsers/gramáticas no
  core e permite carga estritamente sob demanda.
- Sintaxe inválida é um resultado diagnosticável, não falha de parsing; timeout, UTF-8 inválido,
  extensão não suportada, grammar incompatível e falha de leitura são erros separados. Nenhum conteúdo
  é enviado ao provider. Reavaliar a lista inicial quando outro formato passar a ser necessário e
  antes de integrar o resultado a um contexto remoto.

## PD-26 — Prévia de diff textual sem edição

**Data:** 15 de outubro de 2026

**Status:** implementada como função em memória; não lê, escreve ou aplica arquivos.

- `carapana-tools` recebe dois textos já disponíveis ao chamador e produz uma prévia unified diff
  limitada; entradas de até 256 KiB cada, orçamento de 250 ms e saída de até 1 MiB. Usa `similar`
  pinado no manifesto/lockfile (Apache-2.0), com possível aproximação pelo prazo.
- Controles de terminal e controles bidirecionais são substituídos para evitar spoofing de terminal.
  A saída é somente para exibição e pode não preservar bytes ou ser aplicável como patch. Chamadores
  devem tratar entradas/saídas como não confiáveis e aplicar as regras de segredos antes de exibir ou
  compartilhar; nenhum texto é enviado ou persistido por esta função.
- Separar a prévia da escrita evita que gerar um diff cause efeitos colaterais. Escrita atômica,
  checkpoint, proteção contra mudança concorrente e approval seguem pendentes; não inferir autorização
  de alteração a partir desta função.

## PD-27 — Travessia de workspace relativa a descritores

**Data:** 16 de outubro de 2026

**Status:** primitive implementada isoladamente; gate geral e wiring de runtime continuam pendentes.

- A listagem em `carapana-tools` não deve usar um walker que reabre caminhos ambientais. O crate
  mantém a política de recursão, ignore, exclusões e limites; `carapana-storage` fornece somente a
  primitive de enumeração direta, abrindo cada componente com `openat` + `O_NOFOLLOW` e enumerando
  com `rustix::fs::Dir`/`statat(..., SYMLINK_NOFOLLOW)`.
- Cada diretório precisa corresponder à identidade capturada quando foi descoberto. A primitive
  aplica limite sobre todas as entradas examinadas (inclusive nomes protegidos e outros filesystems),
  não retorna nomes protegidos, identifica symlinks sem segui-los e ignora entradas de outro filesystem.
  Compara metadados da cadeia do workspace até o diretório antes/depois, além de reabrir o caminho
  relativo para verificar a identidade. Mudanças/substituições observadas falham fechadas; ancestrais
  externos ao workspace não participam da comparação para que atividade alheia (por exemplo, em `/tmp`)
  não invalide a leitura. Leituras de arquivos continuam passando pela API segura de storage.
- A escolha reaproveita `rustix` já usado pelo storage, sem `unsafe` e sem nova dependência. Manter
  a semântica de ignore no crate de tools evita transferir política/indexação para storage; substituir
  `WalkBuilder` é necessário porque ele não fornece a garantia descriptor-relative requerida.
- A API continua read-only e isolada. A abertura relativa a descritores e a revalidação reduzem a
  janela de substituição/escape e falham fechadas em mudanças observadas; não são lock do filesystem nem
  prova contra toda mutação concorrente. Isso não constitui o gate completo: detecção de segredos no
  conteúdo, autorizações, sandbox, escrita, processos e aprovação seguem independentes e bloqueiam
  runtime wiring.

## PD-28 — Snapshot e substituição atômica isolada

**Data:** 16 de outubro de 2026

**Status:** aplicada como primitive local isolada; não habilita edição pelo agente nem wiring de runtime.

- `carapana-storage` oferece snapshot em memória de um arquivo regular, não sensível, com limite de
  256 KiB, conteúdo e metadata capturados pelo mesmo descritor; o snapshot não é serializável nem
  imprimível por `Debug`. A escrita recebe esse snapshot, não um path/conteúdo-base livre, vinculando a
  proposta ao workspace, path, identidade, bytes e atributos observados.
- A primeira operação substitui somente arquivo existente. Rejeita criação, remoção, symlinks,
  componentes inválidos, outro filesystem, hard links, arquivos grandes, qualquer extended attribute
  ou metadata/ownership que não possa preservar. Exige conteúdo novo de até 256 KiB. Cria temporário
  exclusivo e owner-only no mesmo diretório aberto relativamente a descritores; conteúdo e mode básico
  são sincronizados, o snapshot é revalidado e o rename é atômico. O arquivo substituto fica owner-only
  durante o rename e só depois recebe o mode básico anterior, para não expor o temporário com permissões
  mais amplas. Erro de sync, ajuste de mode ou validação após rename é reportado como resultado incerto;
  o chamador não deve repetir cegamente. Erro normal tenta remover o temporário; crash ou falha nessa
  limpeza pode deixar um arquivo oculto owner-only, sem rotina automática de recovery nesta fatia.
- A comparação exata de conteúdo/metadata/atributos imediatamente antes do rename detecta conflitos
  observados, mas não é compare-and-swap do filesystem nem lock contra editores externos. Uma mutação
  adversarial entre a última validação e o rename ainda pode vencer a corrida; se outro processo
  remover o destino exatamente nessa janela, o rename pode recriá-lo. Escritores Carapanã e externos
  precisam de coordenação/approval próprios antes de qualquer ativação.
- O temporário preserva owner/group somente quando a identidade herdada ao criá-lo coincide com o
  arquivo observado; atributos estendidos e arquivos com hard links são recusados. Filesystem ACLs
  não representadas por extended attributes, flags de filesystem, durabilidade e o breve intervalo
  owner-only após rename ainda precisam de prova por OS/filesystem. Qualquer falha antes do rename
  falha fechada; após rename o resultado pode ser incerto.
- O primitive permanece local, bloqueado para paths protegidos e sem integração com `carapana-tools`,
  diff, approvals, provider, `ToolCallRequested`, daemon, CLI/TUI, sessão ou fila. A prévia de diff
  continua visualização em memória. Wiring exige fechar limitações de concorrência/metadata e todos os
  gates independentes de conteúdo/segredos, autorização, sandbox e teste.

## PD-29 — Prévia de diff vinculada a snapshot, sem aplicação

**Data:** 18 de outubro de 2026

**Status:** implementada como primitive isolada em `carapana-tools`; não habilita edição ou runtime.

- A nova prévia recebe uma referência a `WorkspaceFileSnapshot` e texto proposto; não aceita um path
  nem um texto-base livre, não reabre o workspace e não altera arquivos. A entrada-base vem somente dos
  bytes limitados e capturados pelo snapshot.
- O objeto de resposta retém um empréstimo do próprio snapshot e expõe o path relativo por acesso
  controlado; ele não pode sobreviver ao snapshot ou ser associado a outro arquivo. Não implementa
  serialização nem autoriza aplicação. A função reutiliza os limites e sanitização da prévia unified
  diff existente e falha explicitamente se os bytes capturados não forem UTF-8 válido. Tanto o path
  quanto o diff continuam dados não confiáveis e precisam de escaping apropriado na UI.
- Mantemos a prévia em memória independente do `replace_file_atomically`: geração de diff não chama
  escrita, approval, provider, `ToolCallRequested`, daemon, CLI/TUI, sessão ou fila. Ausência de
  detecção de segredos em conteúdo continua sendo limitação; o preview é dado local não confiável e não
  deve ser compartilhado sem os gates próprios.
- Preferida sobre receber `before`, `after` e path soltos, pois esse contrato permitiria que o chamador
  associasse a prévia ao arquivo errado. O empréstimo evita clonar os até 256 KiB do snapshot e torna a
  dependência temporal explícita; edições concorrentes, autorização e aplicação permanecem fora desta
  decisão.

## PD-30 — Parser limitado de status Git, sem subprocesso

**Data:** 18 de outubro de 2026

**Status:** implementado como parser puro em `carapana-tools`; não executa Git nem acessa o workspace.

- A integração futura usa `git status --porcelain=v2 -z --branch` como fonte de verdade, em vez de
  reconstruir status a partir da index/worktree. Esta fatia somente interpreta bytes já fornecidos pelo
  chamador; não inicia subprocessos, lê configuração Git, acessa credenciais/rede ou modifica arquivos.
- A saída preserva caminhos brutos (incluindo bytes não UTF-8), status separados de index/worktree,
  caminhos de origem/destino em rename/copy e metadados básicos de branch. Paths são dados não confiáveis:
  não autorizam acesso, não devem ser unidos a uma raiz sem confinamento separado e precisam de escaping
  antes de exibição. Os tipos que carregam caminhos/metadados não implementam `Debug`, evitando logs
  acidentais de nomes e branches.
- O parser limita o buffer a 4 MiB, 20.000 entradas e 16 KiB por caminho; registros incompletos,
  duplicação de metadados conhecidos e records desconhecidos falham fechados. Headers desconhecidos são
  ignorados para compatibilidade futura. O executor que vier a coletar stdout ainda deverá interromper
  a captura no limite, pois validar o tamanho após receber um buffer não limita a memória do processo pai.
- Preferido sobre parsing de nomes delimitados por newline/saída humana, que não preservaria paths com
  newlines ou bytes inválidos, e sobre implementar semântica Git própria. Esta primitive não é approval,
  detecção de branch protegida, sincronização de worktrees nem autorização de commit/push; execução de
  Git e wiring continuam bloqueados pelos gates de processo, segredos, autorização e concorrência.

## PD-31 — Parser limitado de metadados de worktrees Git

**Data:** 18 de outubro de 2026

**Status:** implementado como parser puro em `carapana-tools`; não executa Git nem acessa worktree paths.

- A integração futura usará `git worktree list --porcelain -z`; esta fatia só interpreta bytes recebidos,
  preservando path, HEAD, ref de branch, razões e marcadores detached/bare/locked/prunable. O parser
  limita stdout a 4 MiB, 256 worktrees, 16 KiB por path e 4 KiB por metadado; aceita OIDs SHA-1/SHA-256,
  ignora fields desconhecidos e falha em registros conhecidos malformados/duplicados.
- Caminhos e razões continuam dados não confiáveis: não são canonicalizados, abertos, usados como
  autorização ou renderizados sem escaping. `locked`/`prunable` são metadados administrativos do Git,
  não prova de posse do diretório nem sinal de que não exista sessão concorrente. O tipo não implementa
  `Debug`, para evitar log acidental de caminhos e razões.
- Preferido sobre parsing de saída humana delimitada por linhas, que não preserva newlines/bytes
  inválidos. Listar worktrees, detectar sessões Carapanã concorrentes e escolher worktree versus
  compartilhamento são etapas separadas; qualquer execução de Git/wiring continua bloqueada pelos gates
  de processo, autorização, segredos, sandbox e concorrência.

## PD-32 — Artifact store local com quota e retenção explícitas

**Data:** 21 de outubro de 2026

**Status:** aprovada para implementação isolada; não habilita captura automática, tools ou runtime.

- O SQLite guarda somente metadados e estado de lifecycle; bytes ficam em arquivos próprios no diretório
  privado por usuário. IDs numéricos gerados pelo SQLite, sem informação de origem embutida, determinam
  os nomes de arquivo; IDs são identificadores, não capability de autorização. Não persistir nome de
  origem, path do workspace, prompt ou conteúdo nos metadados/logs.
- Defaults ajustáveis pela policy do store: no máximo 10 GiB por usuário e 256 MiB por artifact. Esses são limites de
  admissão iniciais baseados nos valores conceituais do plano, não uma calibração final de hardware.
  Gravações reservam capacidade antes de consumir/gravar o payload; ao exceder limite ou quota, rejeitar
  sem remover artifacts duráveis e sem manter bytes parcialmente aceitos.
- Artifacts duráveis são retidos até exclusão explícita. Artifacts temporários expiram sete dias após a
  criação. Arquivos gerenciados sem registro e gravações pendentes podem ser limpos após sete dias; a
  limpeza é limitada por lotes e nunca toca arquivos fora do namespace privado gerenciado pelo store.
  Locks advisory coordenam writers/leitores ativos com recuperação e exclusão. Metadados de artifact
  durável ausente/corrompido não são removidos silenciosamente.
- Falha na limpeza mantém estado observável e pode bloquear quota; não liberar reserva enquanto bytes
  persistirem ou estado DB/filesystem for incerto. Após limite/quota, a operação falha explicitamente;
  não há evicção de trabalho persistente.
- A API é local e desconectada de `ToolCallRequested`, daemon, sessão/fila, provider e UI. Não há
  captura/cache automática de código, prompts, tool output, segredos ou diffs. Conteúdo não deve ser
  escrito sem os gates independentes de classificação, redaction e autorização; esta decisão não prova
  que um payload seja livre de segredos.
- Não adicionar compressão. A policy fica configurável para futura calibração com benchmarks e hardware
  suportado; qualquer aumento dos tetos requer revisão explícita da policy.
- Preferido sobre BLOBs SQLite para evitar crescimento do banco/WAL e sobre evicção LRU de todos os
  artifacts, que poderia apagar evidências ou trabalho durável. Os arquivos externos introduzem
  reconciliação crash-safe limitada; SQLite continua como catálogo, e estados pendentes/ausentes não são
  tratados como sucesso.

## PD-33 — Sandbox Linux inicial com Bubblewrap, fail-closed

**Data:** 21 de outubro de 2026

**Status:** estratégia aprovada pelo operador; implementação incremental, sem ativação de tools.

- O primeiro backend Linux usa Bubblewrap e namespaces sem privilégios elevados e sem rede por padrão.
  Bubblewrap é dependência de runtime do backend, não um substituto para aprovação, proteção de segredos,
  supervisão/cancelamento ou validação de conteúdo. macOS permanece sem execução por este backend.
- Começar com uma prova isolada e sintética: arquivos de fixture privados, workspace somente leitura,
  tmpfs temporário limitado, ambiente limpo, namespaces de rede/PID/usuário e `no_new_privs` comprovados.
  Não montar HOME, credenciais, repo real nem sockets do host. Falha/indisponibilidade/timeout nunca
  equivalem a sandbox ativo; não usar flags `*-try`, fallback sem sandbox ou elevação para passar o teste.
- A prova aceita somente script e parâmetros internos fixos, com saída limitada e timeout. Não é API
  pública de comando arbitrário nem permite inferir que um workspace real esteja livre de segredos.
  O wiring futuro exige autorização exata, seleção mínima de mounts, orçamento de processos/output,
  cancelamento e testes adversariais separados antes de `ToolCallRequested` executar algo.
- Preferido sobre Landlock sozinho para a primeira prova porque isolamento de filesystem não basta
  para demonstrar isolamento de rede/processos. Seccomp e outras camadas permanecem revisão própria;
  não declarar proteção completa contra todos os ataques a partir desta prova limitada.

## PD-34 — Proposta imutável e aprovação exata de comando

**Data:** 21 de outubro de 2026

**Status:** contrato local do gate de aprovação; sem execução, persistência ou wiring.

- `carapana-core` modela uma proposta imutável com executável absoluto, argv limitado, cwd relativo ao
  workspace e identidades de workspace/tarefa. Não resolve executáveis, abre paths, canonicaliza ou executa.
  Strings/paths são dados não confiáveis; não implementar Debug/serialização de payload nem inferir
  confiança ou classificação de segredos a partir da validação estrutural.
- A solicitação de aprovação tem identidade opaca em memória, prazo monotônico e estado único.
  Apenas evento direto do operador pode aprovar; evento de outra solicitação/workspace/tarefa falha
  sem consumir o pedido atual. Aprovação produz um token não clonável ligado por empréstimo à proposta
  exata; repetição/negação não produz outro token. Invalidar o pedido revoga inclusive token já emitido.
  Descartar o controlador do pedido também revoga tokens; o futuro chamador deve mantê-lo vivo enquanto
  a aprovação puder ser usada. Nenhum token pode autorizar algo depois do descarte silencioso da revisão.
- Esse contrato inicial representa somente aprovação de uso único, não permissão persistente nem
  elevação de autonomia. Tokens nunca são salvos/recriados após restart. A UI/IPC futura precisará mapear
  sua identidade externa a esta solicitação sem tratar dados do modelo como decisão do operador.
- O futuro executor deve consumir o token por valor e revalidar prazo, revogação, identidades e todos os
  gates de sandbox, mounts, conteúdo, concorrência e recursos imediatamente antes do efeito. Ter token
  não basta para spawn. Mudança de proposta exige nova solicitação; aprovação não cria configuração,
  acesso à rede, credenciais ou autorização para um cwd fora do workspace.
- Preferido sobre um booleano `approved` separado de argv/path, que permitiria substituir o comando
  depois da revisão. O contrato não altera `AuthorizationGrant` existente nem habilita `ToolCallRequested`.

## PD-35 — Orçamento agregado e tails limitados de output

**Data:** 21 de outubro de 2026

**Status:** primitive isolada, sem captura de subprocessos nem wiring de tools.

- A primitive no daemon recebe chunks já limitados do futuro leitor, preserva bytes brutos e mantém
  tails separados para stdout/stderr. O chamador escolhe explicitamente o orçamento de cada tail e o
  limite agregado de bytes aceitos entre os dois streams; não há defaults nem realocação automática
  de orçamento entre streams. Output de um stream não pode expulsar o tail reservado ao outro.
- Tetos iniciais da primitive: 16 MiB de payload retido somando os dois tails e 256 MiB de captura
  agregada, derivados dos valores conceituais do plano, não calibrados como defaults de produto.
  Alocação de tails é lazy e limitada; falha de reserva de memória retorna erro antes de aceitar bytes.
  Esses limites são de payload, não uma promessa de RSS incluindo allocator/processos/leitores externos.
- Após esgotamento, chunks posteriores não são aceitos. O resultado distingue bytes aceitos/recusados,
  perda de histórico no ring e orçamento esgotado. O futuro chamador precisa parar a leitura e encerrar
  o processo supervisionado; deixar de drenar um pipe não é cancelamento nem backpressure seguro por si só.
- Não decodificar/renderizar/logar output bruto, implementar Debug/Clone/serialização do buffer, persistir
  automaticamente em artifacts ou enviar a provider. Output ainda precisa dos gates independentes de
  conteúdo/segredos e escaping. Tails são memória volátil, não histórico durável.
- Preferido sobre um buffer por stream com limite agregado implícito, ou `read_to_end` antes de truncar,
  que não limitariam a memória do leitor. Esta primitive não prova execução, supervisão ou consumo bounded
  de bytes antes da chamada; os leitores futuros devem aplicar limites antes de alocar seus chunks.

## PD-36 — Staging privado de snapshots explicitamente selecionados

**Data:** 21 de outubro de 2026

**Status:** estratégia de snapshots selecionados aprovada pelo operador; primitive local isolada.

- O primeiro fornecimento de arquivos ao sandbox parte de uma lista explícita de paths relativos,
  sem enumeração automática do repo. Reutilizar a leitura confinada de `WorkspaceMetadata`: recusar
  traversal, symlinks, arquivos protegidos e arquivos acima do limite existente de 256 KiB.
- Preparar cópias efêmeras em diretório privado novo (0700, arquivos 0600), não um bind do workspace real.
  Não copiar permissões executáveis, HOME, credenciais, arquivos não selecionados ou configuração implícita.
  Falha de preparação tenta descartar apenas staging criado pela operação e nunca altera a origem.
  Crash/falha de cleanup pode deixar staging privado; recovery de órfãos e orçamento global de staging
  ainda precisam ser integrados antes de um launcher de produção.
- O chamador informa limites explicitamente, sem defaults; tetos iniciais da primitive são 256 arquivos,
  16 MiB de conteúdo e paths de 4 KiB/32 componentes. Esses são limites de segurança da implementação,
  não budgets finais calibrados de build. A leitura aplica o restante do orçamento antes de alocar bytes.
- Guardar apenas manifest efêmero com paths, tamanho e SHA-256, sem Debug/Clone/serialização. Verificação
  posterior relê o staging pelo mesmo confinamento e falha se conteúdo/identidade mudar. Essa verificação
  é observacional, não lock contra um atacante do mesmo UID nem prova de imutabilidade após a checagem.
- Staging não autoriza executar/compartilhar conteúdo nem prova ausência de segredos. O futuro chamador
  ainda deve autenticar seleção/aprovação, classificar conteúdo, montar read-only por identidade segura e
  impedir alterações durante o uso. Aplicação de mudanças ao workspace real exige diff/revisão e aprovação
  separadas; não há cópia de retorno, launcher de comandos ou wiring de provider nesta primitive.
- Preferido sobre montar o repo vivo, pois evita expor arquivos não selecionados e mudanças posteriores
  da origem. Builds que precisem de outros arquivos devem solicitar nova seleção, sem ampliar mounts
  automaticamente ou cair para uma cópia completa do repo.

## PD-37 — Quota global compartilhada por artifacts e staging

**Data:** 21 de outubro de 2026

**Status:** política aprovada pelo operador; integração incremental do armazenamento, sem launcher geral.

- Staging deve reservar capacidade dentro da mesma quota de 10 GiB por usuário aprovada em PD-32,
  antes de copiar bytes. Não introduzir quota adicional nem remover artifacts duráveis para abrir espaço.
- Reservas de staging permanecem cobradas enquanto o bundle estiver ativo ou a remoção de seus bytes
  estiver incerta. O encerramento normal tenta remover o staging e só então libera sua reserva; erros
  preservam accounting/recovery em vez de declarar capacidade livre.
- Reutilizar a graça de sete dias para reservas/árvores órfãs. Locks de lease protegem bundles ativos;
  limpar somente o namespace gerenciado de staging em lotes limitados, sem seguir symlinks. Bytes órfãos
  sem registro também precisam contar na admissão, e scans excedendo limites devem falhar fechados.
- O catálogo usa schema v7 para impedir que cleaners antigos, que desconhecem árvores de staging,
  voltem a abrir o catálogo e liberem reservas antes de remover seu conteúdo. A migração exige encerrar
  clientes antigos: `user_version` não revoga conexões já abertas. O tipo de bundle com reserva será distinto do bundle
  sintético não contabilizado; o launcher futuro deverá exigir o tipo contabilizado e os demais gates.
- O launcher/supervisor deve manter a lease até o processo e seus consumidores de staging terminarem.
  Não habilitar detach/`KeepRunning` sem transferência comprovada da lease ao host que mantém o processo;
  apagar paths não prova liberação física de arquivos ainda abertos/montados. A quota é de admissão lógica,
  não hard quota do filesystem ou isolamento contra escrita arbitrária do mesmo UID.

## PD-38 — Manifesto explícito de arquivos de runtime

**Data:** 21 de outubro de 2026

**Status:** estratégia aprovada pelo operador; validação isolada, sem launcher geral.

- Cada entrada declara origem absoluta, destino absoluto exato, papel (executável ou suporte) e SHA-256
  esperado. Não descobrir dependências com `ldd`, interpretar scripts, buscar em PATH, baixar arquivos
  ou expandir diretórios. Dependência ausente exige revisar o manifesto, sem fallback para `/usr` inteiro.
- O primeiro validador Linux abre cada componente por descritor com `NOFOLLOW`, exige diretórios e
  arquivos root-owned sem escrita por grupo/outros, recusa arquivos especiais e setuid/setgid e verifica
  conteúdo por stream limitado. Preserva descritores read-only, identidade e assinatura observada.
- Destinos ficam apenas em `/runtime/`, `/usr/`, `/bin/`, `/lib/` ou `/lib64/`, sem traversal, duplicatas
  ou conflitos arquivo/diretório. Isso não autoriza executar arquivos nem comprova ausência de segredos:
  o manifesto precisa vir de configuração confiável/revisão do operador, nunca do repo/provider sozinho.
- O chamador informa limites de contagem e bytes; tetos da primitive: 128 arquivos, 256 MiB por arquivo
  e 256 MiB agregados. Não são defaults de produto. Hash esperado deve ser aprovado independentemente;
  calcular o hash do arquivo atual automaticamente não constitui allowlist de confiança.
- Revalidação recusa mudança de origem, metadata ou conteúdo. Descritor fixado não impede modificação
  in-place por root nem cria uma cópia imutável. O launcher futuro deve montar arquivos mínimos por
  identidade segura, vincular o manifesto à aprovação e manter todas as outras verificações; esta fatia
  não oferece API de spawn/mount, cache de runtime, rede ou ativação de tools. macOS retorna unsupported.
- Preferido sobre runtime empacotado para começar sem definir distribuição/update de imagem; rever
  essa escolha se o conjunto explícito se tornar impraticável. O probe sintético existente continua
  separado e não comprova isolamento de comandos gerais com este manifesto.

## PD-39 — Limites de memória/PIDs por cgroup v2 previamente delegado

**Data:** 21 de outubro de 2026

**Status:** estratégia aprovada pelo operador; preparação isolada, sem launcher geral.

- Exigir um pai cgroup v2 absoluto, explicitamente configurado e previamente delegado ao usuário sem
  privilégios. Não procurar um pai automaticamente, usar sudo, habilitar controladores, mover processos
  existentes ou alterar limites/configuração do pai. macOS permanece unsupported.
- Validar cgroupfs v2, domínio normal e `memory`/`pids` já habilitados no `cgroup.subtree_control` do pai.
  Criar somente um filho com nome gerado, por descritor, sem adotar cgroups preexistentes. Configurar
  `memory.max`, `memory.swap.max=0`, `memory.oom.group=1`, `pids.max` e proibir subgrupos; reler controles
  antes de retornar. Limites finitos são explícitos, sem defaults; memória usa múltiplos de 64 KiB para
  evitar arredondamento acima do orçamento em hosts com páginas de até 64 KiB.
- Esta primeira primitive prepara/verifica/remove apenas grupos vazios. Não expor migração por PID nem
  tratar preparação como prova de que um processo está contido. O launcher precisa de handshake antes
  de executar código não confiável: migração não captura descendentes já criados nem toda memória prévia.
- Remover somente o filho criado pela operação, após revalidar identidade e ausência de processos;
  não remover recursivamente, matar processos alheios nem declarar cleanup concluído em erro. Drop é
  best-effort e crash pode deixar cgroup vazio: recovery e supervisão continuam gates próprios.
- Testes comuns não alteram cgroups do host. Um teste opt-in exige pai de teste explicitamente delegado
  e valida criação/readback/remoção de filho vazio. Sem essa configuração, não declarar enforcement de
  memória/PIDs comprovado; saturação real e lifecycle de processos ainda precisam de prova positiva.
  O operador informou que esse pai ainda não existe. Isso não é uma decisão de interromper o
  desenvolvimento: ele corrigiu expressamente essa interpretação e pediu continuar o plano.
  A prova real de recursos segue pendente, sem dispensar os gates técnicos antes de execução real.
- Preferido sobre dependência inicial de systemd de usuário para manter a interface independente do
  gerenciador de sessão. Rever se o provisionamento da delegação for impraticável. RLIMIT isolado não
  substitui orçamento agregado de memória/descendentes, e este slice não limita CPU/I/O/HugeTLB por si só.

## PD-40 — Leitura limitada e não bloqueante de stdout/stderr

**Data:** 22 de outubro de 2026

**Status:** implementação local dos leitores, independente de delegação cgroup; sem ativar tools.

- Estender PD-35 com um leitor Unix que recebe a posse de `ChildStdout`/`ChildStderr` já criados por
  um chamador confiável. Não iniciar processos, herdar ambiente ou abrir paths nessa API. Configurar os
  pipes em nonblocking e ler chunks fixos de até 4 KiB, alternando streams sem fila ou threads auxiliares.
- O orçamento agregado de captura limita também cada read antes de alocar/aceitar bytes; tails continuam
  reservados por stream. Verificar cancelamento/deadline entre reads e retornar motivo explícito:
  EOF dos dois pipes, orçamento esgotado, cancelamento ou timeout. EOF não significa processo finalizado.
- O chamador/supervisor deve terminar e reaproveitar/recolher somente processos próprios ao receber
  cancelamento/timeout/esgotamento/erro. Parar a captura não mata o produtor; a API não promete isso nem
  fecha pipes prematuramente ao devolver tails. Não usar `KeepRunning` como fallback de erro de captura.
- Output permanece bytes brutos privados, sem Debug/Clone/serialização/logs/artifacts ou envio automático
  a provider. A integração com o supervisor e os gates de conteúdo/segredos continuam requisitos separados.
- Preferido sobre uma thread por pipe com canal, que pode deixar leitores presos em descendentes que
  mantêm descritores abertos. Polling nonblocking tem latência ociosa de até 10 ms; testar pipes reais,
  flood, stream silencioso, EOF parcial, cancelamento e deadlines antes de integrar o host.
