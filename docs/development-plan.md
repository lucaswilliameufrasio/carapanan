# Carapanã

## Plano completo de desenvolvimento

**Status:** especificação de produto e engenharia  
**Baseline:** outubro de 2026  
**Revisão de produto:** 4 de outubro de 2026\
**Nome do produto:** Carapanã  
**Binário:** `carapana`  
**Categoria:** local-first, model-agnostic coding agent harness  
**Implementação principal:** Rust  
**Objetivo inicial:** obter uma experiência de uso comparável ou superior ao Claude Code utilizando prioritariamente o plano do ChatGPT, sem ficar preso a um único provider, dispositivo ou interface.

## Decisões complementares obrigatórias

Ler este plano junto de [Decisões de produto e comportamento](product-decisions.md),
que registra a revisão aprovada de perfis, fila, autonomia, segurança, recovery, memória
e critérios de dogfooding. Suas regras específicas prevalecem sobre exemplos anteriores
incompatíveis, sem dispensar os gates deste documento.

As decisões estão aprovadas como especificação, não integralmente implementadas.
**Delivery 0 aprovado explicitamente em 6 de outubro de 2026; PR #1 integrada
em `main`. Delivery 1 autorizado**, limitado à fundação de engenharia descrita
na seção 155; essa aprovação não declara as entregas posteriores implementadas.

---

# 0. Regra de execução do roadmap

## DELIVERY 0 É OBRIGATÓRIO E BLOQUEANTE

O desenvolvimento começa pela experiência do produto.

**Delivery 0 = Product Design, UI e UX de:**

- CLI;
- TUI;
- Web/PWA desktop;
- Web/PWA mobile;
- sessões;
- approvals;
- Auto Mode;
- MCP;
- configurações;
- diff;
- plano da tarefa;
- conclusão/validação;
- multi-device;
- resource management;
- erros, estados degradados e recovery.

### Hard gate

Após a conclusão do Delivery 0:

**PARAR O DESENVOLVIMENTO.**

Não iniciar:

- agent runtime;
- integração real com providers;
- daemon;
- MCP runtime;
- sandbox;
- tools;
- context manager;
- banco;
- Auto Mode real;
- execução de comandos.

O Delivery 0 deve ser apresentado para revisão.

O desenvolvimento só prossegue quando houver **aprovação explícita do responsável pelo produto**.

Uma resposta equivalente a:

> Delivery 0 aprovado. Pode seguir.

libera o Delivery 1.

Correções solicitadas durante a revisão permanecem dentro do Delivery 0.

---

# 1. Visão do produto

Carapanã será um coding-agent harness em que a experiência pertence ao Carapanã e o modelo é substituível.

A arquitetura não será:

```text
LLM
 ↓
shell
```

Será:

```text
                       CARAPANÃ

              ┌─────────────────────┐
              │    Agent Runtime    │
              └──────────┬──────────┘
                         │
       ┌─────────────────┼─────────────────┐
       │                 │                 │
    Context            Skills           Policy
       │                 │                 │
       └────────────┬────┴────┬───────────┘
                    │         │
                  Tools    Decision
                    │         │
                    │   builtin / Jev / LLM
                    │
                 Sandbox
                    │
              Operating System

                         │
            ┌────────────┼─────────────┐
            │            │             │
          OpenAI      Anthropic     OpenRouter
            │            │             │
         ChatGPT       Claude        qualquer
            │
      plano ChatGPT
```

## Proposta central

> **Same agent. Any model. Any device.**

O primeiro caso de uso, entretanto, é intencionalmente mais específico:

> **Experiência próxima ao Claude Code usando o plano do ChatGPT.**

A OpenAI documenta autenticação e uso do plano ChatGPT para ferramentas open-source e
projetos pessoais locais elegíveis, com autorização própria da aplicação, sem API key.
A conta de referência é ChatGPT Pro de US$100; acesso, modelos, capacidades e cota
precisam ser comprovados por inferência real após o Delivery 0, não presumidos pelo
nome do plano. Usar somente o fluxo oficial; scraping de credenciais, reutilização
indevida de tokens do Codex ou endpoints privados são proibidos. Não há fallback pago
automático. Referências e regras de cota estão em PD-02 e PD-10 do complemento.

---

# 2. Princípios fundamentais

## 2.1 Local-first

O workspace, sessões, logs, configurações, artefatos e estado pertencem à máquina do usuário.

Nenhum Carapanã Cloud é necessário.

## 2.2 Provider-agnostic sem lowest common denominator

Não reduzir todos os modelos à mesma interface pobre.

Cada backend declara suas capacidades.

## 2.3 Autonomia com disciplina

Auto Mode não significa `--yes`.

A autonomia deve combinar:

- intenção;
- policy;
- sandbox;
- contexto;
- classificação de risco;
- validação;
- evidência.

## 2.4 Segurança antes de conveniência

O agente não recebe automaticamente acesso a:

- `.env`;
- chaves privadas;
- credentials;
- tokens;
- state de Terraform;
- kubeconfigs;
- arquivos considerados secretos.

Valores secretos nunca vão para modelo, classifier, chat, logs, artifacts, reports
ou memória externa. Uso local por comando autorizado não concede exposição. Essas
proteções exigem enforcement antes de tools reais, não apenas uma skill de prompt.

## 2.5 Entrega só existe quando há evidência

O agente não poderá afirmar simplesmente:

> implemented successfully

sem validação proporcional ao trabalho realizado.

## 2.6 O agente não deve destruir a máquina tentando ajudar

Isso inclui:

- comandos destrutivos;
- exfiltração;
- loops;
- 15 GB de RAM por sessão;
- escrita descontrolada no SSD;
- logs ilimitados;
- processos órfãos;
- indexação cega.

## 2.7 UI é produto, não decoração

A arquitetura não poderá determinar acidentalmente uma UX ruim.

Por isso o Delivery 0 existe antes do runtime.

---

# 3. Escopo inicial

## Incluído

- CLI;
- TUI;
- daemon local;
- Web/PWA;
- mobile supervision;
- sessões persistentes;
- multi-device;
- ChatGPT;
- providers adicionais;
- shell/filesystem/git;
- Auto Mode;
- policy engine;
- sandbox;
- MCP;
- skills;
- subagents;
- context management;
- testing/validation;
- local development automation;
- resource budgets;
- docs.

## Fora do escopo inicial

- SaaS hospedado do Carapanã;
- relay público próprio;
- túnel público;
- colaboração empresarial;
- billing;
- organizações;
- RBAC corporativo;
- cloud sync de workspaces;
- editor de código web completo;
- marketplace de plugins;
- ABI de plugins Rust;
- vector database obrigatório;
- memória global proprietária substituindo MCPs como `ai-memory`.

## Primeiro marco de dogfooding

O escopo acima é o roadmap, não uma exigência de implementar tudo antes do primeiro uso.
Após aprovação do Delivery 0, priorizar CLI/TUI, assinatura ChatGPT, edição/execução
seguras, sessões persistentes, MCP/skills e validação. Esse marco depende dos Deliveries
1–9 e dos gates de segurança, conforme PD-02 e PD-17 do complemento.

Conexão real de web/mobile, providers adicionais e subagents não bloqueiam o marco;
protótipos web/mobile continuam obrigatórios no Delivery 0. Tudo depende de viabilidade
comprovada, sem tornar API paga requisito no lugar da assinatura.

---

# 4. Tecnologias

## 4.1 Rust

Usar **Rust estável atual** com **edition 2024**.

Rust 2024 permanece a edição atual e o toolchain estável documentado em outubro de 2026 já está além da versão 1.97. O projeto não deverá congelar uma versão antiga apenas porque ela apareceu neste plano.

Criar:

```text
rust-toolchain.toml
```

para pin reproduzível, atualizado regularmente por Renovate.

## 4.2 Backend/runtime

- Tokio;
- Axum;
- Serde;
- serde_json;
- thiserror;
- anyhow somente nas bordas/aplicações;
- tracing;
- tracing-subscriber;
- reqwest;
- rustls;
- SQLx;
- SQLite;
- toml_edit;
- schemars;
- clap;
- tree-sitter;
- biblioteca `ignore` ou equivalente para traversal Git-aware.

Todas as dependências devem usar **a versão estável atual no momento da implementação**.

Nenhum número de versão copiado deste documento poderá ser tratado como requisito.

---

# 5. Frontend

## Web/PWA

- Svelte 5;
- SvelteKit;
- TypeScript;
- Vite;
- Tailwind como utility layer;
- CSS custom properties como design-token source of truth;
- Lucide ou equivalente consistente para iconografia;
- primitives headless acessíveis quando úteis;
- Vitest;
- Playwright.

Não usar shadcn ou outro kit como identidade visual do produto.

O Carapanã terá design system próprio.

## Package manager

`pnpm`.

A versão será controlada pelo repositório e atualizada automaticamente.

---

# 6. TUI

- Ratatui;
- Crossterm.

A arquitetura será baseada em:

```text
events
  ↓
update/reducer
  ↓
application state
  ↓
render
```

O renderer não poderá conter regras do agent runtime.

---

# 7. CLI

- clap derive;
- clap_complete;
- prompts interativos apenas quando realmente melhorarem o fluxo.

Executar:

```bash
carapana
```

abre a TUI por padrão quando possível.

Headless permanece first-class.

---

# 8. Persistência

## SQLite

Não exigir PostgreSQL.

SQLite armazenará metadata estruturada e estado pequeno.

Usar WAL com:

- checkpoint controlado;
- limites;
- batching;
- prepared statements;
- migrations;
- observação de tamanho.

Não persistir cada token de streaming individualmente.

Persistir também fila ordenada, origem, seleção de perfil/provider/modelo/variante e
estados de processamento. Após reinício, recuperar a fila com a sessão em pausa.

## Artifact store

Objetos grandes ficam fora do SQLite.

Exemplo Linux:

```text
~/.local/share/carapana/
├── carapana.db
├── sessions/
├── artifacts/
├── logs/
└── cache/
```

macOS e Windows devem usar os diretórios nativos equivalentes.

---

# 9. Estrutura inicial do repositório

Evitar começar com dezenas de crates.

```text
carapana/
├── Cargo.toml
├── rust-toolchain.toml
├── mise.toml
├── justfile
├── AGENTS.md
│
├── crates/
│   ├── carapana-cli/
│   ├── carapana-core/
│   ├── carapana-daemon/
│   ├── carapana-protocol/
│   ├── carapana-tui/
│   ├── carapana-policy/
│   ├── carapana-storage/
│   └── carapana-providers/
│
├── apps/
│   └── web/
│
├── packages/
│   └── design/
│
├── skills/
│   └── builtin/
│
├── docs/
│
├── tests/
│   ├── scenarios/
│   ├── adversarial/
│   ├── protocol/
│   └── resources/
│
└── fixtures/
```

Separações adicionais surgem quando limites estiverem comprovados.

Possíveis crates futuros:

```text
carapana-context
carapana-tools
carapana-sandbox
carapana-mcp
carapana-skills
```

---

# 10. Product Design Foundation — Delivery 0

## Objetivo

Definir a experiência completa antes de ligar qualquer agent runtime real.

Toda UI usa dados simulados.

Nenhum provider externo será necessário.

Os protótipos serão interativos, com seletor de cenários para reproduzir execução,
fila, intervenção, approvals, cota, falhas, recovery, desconexão e pressão de recursos.
Incluir headless simulado, JSON e códigos de saída, conforme PD-01 e PD-14.

---

# 11. Personalidade visual

Carapanã deve transmitir:

```text
technical
calm
dense
fast
trustworthy
terminal-adjacent
focused
```

Não deve parecer:

```text
cyberpunk
crypto dashboard
generic SaaS
AI generated admin panel
```

Evitar:

- glassmorphism;
- gradientes decorativos sem função;
- cards dentro de cards;
- arredondamento excessivo;
- sombras arbitrárias;
- excesso de cores;
- ícones emoji;
- animações gratuitas;
- giant headings;
- empty space excessivo.

A regionalidade do nome deve aparecer na identidade, não em caricatura visual.

---

# 12. Design system

Tokens semânticos:

```text
surface
surface-muted
surface-raised

text-primary
text-secondary
text-muted
text-disabled

interactive
focus

success
warning
danger
attention

border
border-strong
```

Também definir:

- spacing;
- radius;
- typography;
- icon sizes;
- density;
- motion;
- focus;
- elevation;
- code typography.

## Semântica obrigatória

`attention`:

> intervenção humana necessária.

`warning`:

> operação degradada, mas possível.

`danger`:

> falha crítica ou ação perigosa.

Cor nunca será o único indicador.

Validar claro/escuro desde o Delivery 0, seguindo o sistema com escolha manual.
Interface inicial em pt-BR, com textos separados do código para inglês depois;
comandos e identificadores técnicos permanecem estáveis.

---

# 13. Accessibility

Web/PWA deve buscar WCAG 2.2 AA.

Obrigatório:

- keyboard navigation;
- focus visible;
- screen-reader names;
- reduced motion;
- contraste adequado;
- touch targets apropriados;
- estados não identificados apenas por cor.

TUI:

- suporte a terminais sem truecolor;
- símbolos + texto além de cor;
- shortcuts documentados;
- foco claro.

Todas as ações por teclado via SSH, sem depender de mouse, truecolor ou teclas especiais.
Documentar alternativas a `Shift+Tab`. Suportar `80×24`, reorganizando painéis sem
esconder approvals/ações essenciais; abaixo disso, orientar ampliar o terminal.

---

# 14. Information architecture

```text
Carapanã
│
├── Sessions
│   ├── Needs Attention
│   ├── Running
│   ├── Idle
│   ├── Hibernated
│   └── Recent
│
├── Current Session
│   ├── Conversation
│   ├── Activity
│   ├── Plan
│   ├── Changes
│   └── Validation
│
├── MCP
├── Skills
├── Providers
├── Devices
├── Resources
└── Settings
```

---

# 15. Session screen

A sessão é a tela principal.

Precisa mostrar permanentemente:

- workspace;
- branch;
- estado;
- provider;
- modelo;
- reasoning;
- Auto Mode;
- contexto;
- resource state.

Separar visualmente `Executando: perfil · modelo · variante` da seleção
`Próxima mensagem: perfil · modelo · variante`. Trocar a seleção no composer, sem
enviar, não altera o agente em execução.

Exemplo:

```text
┌─────────────────────────────────────────────────────────────┐
│ Carapanã · sample-api     AUTO · GPT       Context 31%    │
├────────────┬────────────────────────────────────────────────┤
│ Sessions   │                                                │
│            │  ● Inspecting authentication flow             │
│ ● sample-api  │  ● Editing refresh_token.rs                   │
│ ◐ sample-web   │  ✓ cargo test                                 │
│ ! carapana │                                                │
│            │  Updated token rotation and added...          │
│            │                                                │
├────────────┴────────────────────────────────────────────────┤
│ >                                                           │
└─────────────────────────────────────────────────────────────┘
```

---

# 16. Progressive disclosure

Tool output bruto não deve dominar a UI.

Ruim:

```text
Running tool...
{"command":"cargo test"}
928123 bytes...
```

Ideal:

```text
✓ cargo test
  183 passed · 0 failed · 4.8s
```

Detalhes podem ser expandidos.

O output completo pode virar artifact quando necessário.

Na TUI, conversa principal com atividade resumida integrada. Plano/diff/validação/logs
completos abrem sob demanda; terminal largo pode mostrar detalhes em painel lateral.

---

# 17. Plan UX

Todo trabalho relevante possui plano persistente.

```text
Plan

✓ Inspect implementation
✓ Reproduce issue
● Update token rotation
○ Add tests
○ Run validation
```

Status:

```text
pending
running
blocked
done
skipped
```

Mudanças no plano devem aparecer como eventos.

Em Executar, apresentar plano não exige aprovação adicional; ações seguem permissões
efetivas. Planejar não implementa automaticamente. Sessões podem conter vários trabalhos
sem botão obrigatório de `Nova tarefa`. Autorizações temporárias expiram na conclusão;
ambiguidade exige pergunta antes de reutilizá-las.

---

# 18. Diff UX

Mostrar:

```text
Changes +124 −38

auth/service.rs       +48  -12
auth/repository.rs    +21   -4
auth/service_test.rs  +55  -22
```

Desktop:

- file list;
- hunks;
- previous/next;
- syntax highlighting.

Mobile:

```text
Previous   3 / 11   Next
```

Não construir IDE web.

Registrar estado inicial e distinguir alterações anteriores das feitas pelo agente.
Conflitos exigem confirmação, inclusive em Yolo. Restaurar checkpoint exige prévia e
confirmação, sem descartar trabalho do operador; conflitos impedem rollback forçado.

---

# 19. Approval UX

Nunca:

```text
Are you sure?
Yes / No
```

Sempre apresentar contexto.

```text
Approval required

git push origin feature/refresh-token

Repository
sample-api

Remote
github.com/example-org/sample-api

Branch
feature/refresh-token

Risk
Writes to remote repository

Reason
Required to open the requested PR.

[ Allow once ]
[ Allow for this task ]
[ Deny ]
```

Intervenção invalida approvals pendentes; ações necessárias são reavaliadas.
Para caminhos externos, oferecer também `Permitir sempre`, vinculado à ação e ao
caminho específico, com descendentes explícitos para pastas e sem seguir symlinks
para alvos externos. Permissões persistentes são visíveis e revogáveis.

---

# 20. Completion UX

Nunca finalizar apenas com:

```text
Done!
```

Formato:

```text
Completed

Changed
- refresh token rotation
- reuse detection
- integration coverage

Validation
✓ cargo fmt --check
✓ cargo clippy
✓ cargo test --workspace
✓ 183 tests passed
✓ API smoke test

Not validated
- Google OAuth callback
  External provider required

Changes
4 files · +124 −38
```

Sem validação suficiente, mostrar `Implementado, validação incompleta`, motivo e
evidências. Pausar a fila até decisão do operador; não apresentar como sucesso validado.

---

# 21. Mobile UX

Mobile será para:

- acompanhar;
- intervir;
- revisar;
- continuar conversa.

Casos principais:

```text
view sessions
view progress
send prompt
approve / deny
inspect diff
view validation
switch session
stop
resume
```

Não será editor completo.

Home:

```text
Needs attention

! carapana
  Waiting for approval
  20s


Running

● sample-api
  Running tests
  2m

● sample-web
  Implementing checkout retry
  7m
```

Pergunta que essa tela precisa responder imediatamente:

> Tem alguma coisa precisando de mim?

Inicialmente, attention queue destacada e avisos com PWA aberta. Notificações com a
PWA fechada ficam para investigação posterior; não prometer push no Delivery 0.

---

# 22. Command palette

TUI/Web desktop:

```text
Ctrl/Cmd + K
```

Ações:

```text
Switch model
Change reasoning
Toggle Auto Mode
Open session
Manage MCP
Manage skills
Run doctor
View context
Show resources
Open settings
```

`Shift+Tab` percorre perfis configuráveis na ordem escolhida. Presets: Planejar,
Perguntar, Auto e Yolo, cada um com provider/modelo/variante próprios. Seleção salva
na sessão; defaults gerais só mudam por `Salvar como padrão deste perfil`.
Novas sessões começam em Perguntar, salvo padrão explicitamente configurado.
Perfis/fila/intervenção seguem PD-03 e PD-04 do complemento.

---

# 23. Estados da sessão

```text
starting
running
waiting-for-tool
waiting-for-user
waiting-for-approval
validating
paused
detached
hibernated
recovering
failed
completed
stopped
```

---

# 24. Estados de MCP

```text
disabled
starting
healthy
degraded
restarting
auth-required
unhealthy
failed
```

---

# 25. Delivery 0 — artefatos obrigatórios

Entregar:

1. design principles;
2. information architecture;
3. tokens;
4. component inventory;
5. keyboard map;
6. TUI prototype;
7. CLI UX;
8. desktop web prototype;
9. mobile prototype;
10. session dashboard;
11. session detail;
12. attention queue;
13. approval flow;
14. diff viewer;
15. plan viewer;
16. validation/completion view;
17. MCP management;
18. provider/model settings;
19. skills management;
20. resource management;
21. device pairing;
22. remote attach;
23. doctor;
24. config;
25. error/degraded states;
26. reconnect/recovery experience;
27. accessibility checklist;
28. visual acceptance tests.

Os protótipos devem ser interativos e utilizar mocks, incluindo os cenários de
perfis/fila/intervenção, permissões, modelo/variante indisponível, contexto e headless.
Entregar seletor de cenários, roteiro de revisão e checklist de aceitação.

---

# 26. Critérios de aprovação do Delivery 0

A entrega não passa se:

- sessão ativa não estiver óbvia;
- mobile abrir em chat vazio;
- attention-required ficar escondido;
- tool logs dominarem a tela;
- approval não explicar risco;
- config exigir editar TOML para tarefa comum;
- web parecer SaaS genérico;
- TUI parecer apenas um log viewer;
- design não funcionar com várias sessões;
- resource warnings não existirem;
- MCP exigir reinício na UX proposta;
- estados de erro não estiverem desenhados.

**Depois da apresentação: STOP.**

Aprovação do conjunto CLI/TUI/web/mobile; revisões parciais não liberam runtime.
Validar macOS/Linux dentro e fora do Herdr, ambiente de teste e não dependência do
Carapanã. CLI/TUI também serão testadas via SSH, sem integração real nesta entrega.

---

# 27. Arquitetura runtime

Após aprovação:

```text
                    carapana-daemon

                         │
            ┌────────────┼────────────┐
            │            │            │
         Session       Agent        MCP
         Registry      Runtime      Registry
            │            │            │
            └───────┬────┴────┬──────┘
                    │         │
                 Policy     Tools
                    │         │
                 Sandbox   Processes
                    │
                 Storage
```

Clientes:

```text
CLI
TUI
Web
PWA
Remote CLI
```

O daemon é a autoridade do estado.

---

# 28. Comunicação local

CLI/TUI locais devem preferir:

- Unix Domain Socket em Unix/macOS;
- Named Pipe em Windows.

A API semântica permanece a mesma usada remotamente.

Web local é servido pelo daemon via loopback.

---

# 29. Comunicação remota

- HTTP;
- WebSocket;
- JSON inicialmente;
- TLS para acesso remoto.

Não introduzir protobuf antes de necessidade comprovada.

Protocolo versionado.

```json
{
  "protocol": 1,
  "type": "session.updated",
  "session_id": "...",
  "sequence": 182
}
```

---

# 30. Reconexão

Ao conectar, qualquer cliente recebe primeiro:

```text
full current snapshot
```

e depois:

```text
incremental events
```

Nunca depender apenas dos eventos ocorridos depois da conexão.

Isso garante que Web/PWA detecte imediatamente sessões já abertas.

---

# 31. Session Registry

Estado mínimo:

```text
id
workspace
branch
status
model
provider
agent_state
attached_clients
created_at
updated_at
attention_reason
```

---

# 32. Detecção automática de sessão ativa

Executar:

```bash
cd ~/Code/sample-api
carapana
```

quando já existir uma sessão ativa deve mostrar:

```text
Active session found

sample-api
Running · GPT · started 18m ago

[ Attach ]
[ Start new ]
[ View sessions ]
```

Nunca criar silenciosamente sessão duplicada.

---

# 33. Attention Queue

Centralizada no daemon.

```text
Needs attention

1. sample-api
   Waiting for approval

2. carapana
   Tests failed repeatedly

3. sample-web
   Needs clarification
```

Igual em:

- TUI;
- web;
- mobile.

---

# 34. Multi-client

Uma sessão aceita múltiplos clientes simultaneamente.

```text
MacBook TUI
Android PWA
Browser
```

Todos visualizam os mesmos eventos.

Agent runtime único.

---

# 35. Approval race

Se dois devices tentarem resolver o mesmo approval:

- operação idempotente;
- primeiro resultado válido ganha;
- demais recebem `already_resolved`;
- decisão persistida uma vez.

---

# 36. Session lifecycle

Separar:

```text
session persisted
session resident
agent executing
client attached
```

São quatro estados diferentes.

Fechar TUI não mata sessão.

---

# 37. Crash recovery

Persistir suficiente para recuperar:

- task;
- plan;
- messages;
- intent ledger;
- tool history;
- approvals;
- validation;
- workspace;
- checkpoint;
- provider metadata.

Após crash:

```text
recovering
```

e depois:

```text
paused
```

Recovery após crash/reinício nunca retoma execução automaticamente, inclusive em Yolo.
Mostrar onde parou e permitir retomada explícita. Falha na recuperação pode marcar
`failed`, com diagnóstico, sem iniciar ações.

Nunca repetir automaticamente operação destrutiva cujo resultado esteja incerto.

Preservar fila com ordem/texto/perfil/provider/modelo/variante. Na retomada, revalidar
arquivos/permissões, invalidando leituras antigas e pedindo confirmação em conflitos.

---

# 38. Event sourcing leve

Eventos relevantes:

```text
SessionCreated
UserMessage
AgentMessage
PlanChanged
ToolRequested
ToolStarted
ToolCompleted
FileChanged
ApprovalRequested
ApprovalResolved
ModelChanged
ContextCompacted
ValidationCompleted
SessionPaused
SessionCompleted
```

Não transformar tudo em event sourcing acadêmico.

Estado materializado continua existindo.

---

# 39. Storage schema

Entidades principais:

```text
sessions
session_events
messages
plans
plan_steps
tool_calls
tool_artifacts
approvals
intent_authorizations
validation_evidence
checkpoints
devices
mcp_servers
mcp_permissions
provider_accounts
audit_events
```

Representar também perfis/seleções por sessão, fila ordenada e estado de processamento,
confiança por workspace e permissões persistentes vinculadas à ação/caminho.
Contratos e nomes finais são definidos na fundação, não no Delivery 0.

---

# 40. Configuração

Formato:

```text
TOML
```

Precedência:

```text
defaults
  ↓
system
  ↓
user
  ↓
project
  ↓
environment
  ↓
CLI
  ↓
session
```

A precedência não autoriza o projeto a ampliar permissões ou desligar proteções.
Exigir confiança antes de ativar configs/MCPs/hooks do workspace; mudanças que introduzam
execução ou ampliem acesso exigem nova confirmação com diff, conforme PD-09.

---

# 41. CLI de configuração

```bash
carapana config path
carapana config list
carapana config show
carapana config show --effective

carapana config get <key>
carapana config set <key> <value>
carapana config unset <key>

carapana config explain <key>
carapana config validate
carapana config edit
```

`toml_edit` deve preservar comentários e formatação.

---

# 42. Config explain

Exemplo:

```text
model.primary

effective:
  chatgpt:gpt-...

source:
  ~/.config/carapana/config.toml

precedence:
  default       ...
  user          chatgpt:gpt-...
  project       -
  environment   -
  cli           -
  session       -
```

---

# 43. Settings pela TUI/Web

Configuração comum nunca exigirá TOML.

Ao salvar:

```text
Session only
This project
User defaults
```

---

# 44. Schema

Gerar:

```text
carapana.schema.json
```

a partir dos structs Rust.

Usar o schema para:

- validation;
- editor autocomplete;
- docs;
- migration checks.

---

# 45. CLI principal

Superfície planejada:

```text
carapana
carapana run
carapana attach
carapana sessions

carapana daemon
carapana remote
carapana device

carapana config
carapana doctor
carapana info
carapana resources
carapana storage

carapana auth
carapana provider
carapana model

carapana mcp
carapana skills

carapana update
carapana completion
```

Headless não interativo que precisar de approval pausa a sessão e retorna resultado
estruturado, sem aprovação automática nem espera indefinida do comando. Formatos,
códigos de saída e exemplos serão desenhados com mocks no Delivery 0.

---

# 46. Doctor

```bash
carapana doctor
```

Verificar:

```text
Core
Config
Storage
Git
Sandbox
Provider auth
Models
Decision engine
Skills
MCP
Resource limits
Child processes
Remote configuration
```

---

# 47. Doctor --fix

Pode corrigir apenas coisas seguras e determinísticas.

Exemplos:

```text
create missing config directory
checkpoint oversized WAL
remove expired disposable cache
rebuild generated schema
```

Nunca instalar software via sudo automaticamente.

---

# 48. Doctor --report

Gera relatório sanitizado.

Nunca incluir:

- tokens;
- secrets;
- conteúdo do código;
- prompts;
- home path completo se puder ser identificador;
- valores de env.

---

# 49. Info

```bash
carapana info
```

Responde:

```text
Workspace
Session
Provider
Model
Reasoning
Auto Mode
Context
Sandbox
Decision engine
Skills
Config locations
Instructions
Attached devices
```

---

# 50. Providers

Interface:

```rust
trait ModelBackend {
    fn capabilities(&self) -> ModelCapabilities;

    async fn infer(
        &self,
        request: ModelRequest,
    ) -> Result<ModelStream>;
}
```

---

# 51. Capability Matrix

Possíveis capacidades:

```text
streaming
reasoning
reasoning_effort
tool_calls
parallel_tool_calls
structured_output
images
prompt_cache
native_compaction
continuation
native_search
computer_use
```

Runtime adapta comportamento.

Não fingir que todos providers são OpenAI-compatible.

---

# 52. Provider roadmap

Primeiro:

```text
ChatGPT plan
```

Arquitetura desde o início preparada para:

```text
OpenAI API
Anthropic
OpenRouter
OpenAI-compatible
Gemini
local
```

---

# 53. ChatGPT provider

Objetivo principal do v0.x.

Deve utilizar somente autenticação oficialmente suportada.

Quando delegated ChatGPT-plan usage estiver disponível para Carapanã:

```text
Continue with ChatGPT
```

e nunca solicitar API key para esse modo.

A integração deve tratar:

- login;
- logout;
- refresh;
- revogação;
- quota;
- provider unavailable;
- unsupported capability.

O plano do ChatGPT não deve ser confundido com cobrança da API.

Na primeira troca de provider, explicar compartilhamento de contexto e pedir confirmação.
Oferecer dispensa por provider nesta sessão e configuração global para desativar esses
avisos, sem liberar segredos. Modelos/variantes indisponíveis pausam e exigem escolha,
sem substituição silenciosa. Retries, espera por cota e retomada seguem PD-10.

---

# 54. Provider auth storage

Tokens nunca ficam em TOML.

Preferir:

```text
macOS Keychain
Windows Credential Manager
Secret Service/KWallet
```

Fallback somente:

```text
encrypted local credential store
```

com consentimento explícito.

Nunca plaintext silencioso.

---

# 55. Model switching

Sessão pertence ao Carapanã.

Modelo não possui a sessão.

```text
GPT
 ↓
Claude
 ↓
Kimi
```

pode continuar o mesmo trabalho.

Antes da troca:

- capability negotiation;
- context adaptation;
- tool compatibility check.

---

# 56. Context architecture

Camadas:

```text
L0 immutable
  system + safety policy

L1 task
  goal + intent ledger

L2 project
  AGENTS.md + skills

L3 working set
  arquivos ativos

L4 retrieved
  referências

L5 ephemeral
  logs/tool outputs

L6 archive
  histórico resumido
```

---

# 57. Compaction

Prioridade:

```text
L0 never compact
L1 preserve
L2 preserve relevant
L3 preserve aggressively
L4 summarize/drop
L5 drop first
L6 summarize
```

O modelo não decide sozinho o que esquecer.

Compactação automática com evento visível não apaga histórico local. Preservar objetivo,
plano, autorizações e trabalho pendente; se informação essencial não puder ser preservada,
pausar. Hooks de memória complementam o processo, sem substituir estado local.

---

# 58. AGENTS.md

Suporte first-class.

Permitir instruções path-scoped.

Não criar formato proprietário quando convenção existente atende.

---

# 59. Code search

Inicialmente:

- ripgrep;
- git grep;
- filesystem traversal;
- tree-sitter on demand.

Não adicionar vector DB obrigatório.

---

# 60. Filesystem traversal

Respeitar:

```text
.gitignore
.ignore
.carapanaignore
```

Ignorar por padrão:

```text
.git
node_modules
target
dist
build caches
vendor caches
```

Não seguir symlink para fora do workspace sem autorização.

Detectar arquivos gigantes antes de ler.

---

# 61. Tool execution

Não usar `sh -c` para tudo.

Estrutura:

```rust
CommandSpec {
    executable,
    args,
    cwd,
    env,
    timeout,
    stdio_policy,
}
```

Usar shell apenas quando sintaxe de shell for necessária.

Não herdar todo o ambiente do terminal; variáveis sensíveis exigem autorização para
uso local sem exposição. Elevação de privilégios é bloqueada na versão inicial,
inclusive `sudo` e equivalentes em Yolo. Operações administrativas ficam para execução
manual fora do harness. Instalações locais/globais seguem PD-07.

---

# 62. File modifications

- writes atômicos;
- preservar permissões;
- preservar line endings quando adequado;
- gerar diff;
- bloquear arquivos protegidos;
- checkpoint antes de mudanças de risco relevante.

---

# 63. Git

Git CLI é fonte de verdade inicial.

Integrar:

```text
status
diff
branch
remote
fetch
worktree
rebase
commit
push
```

---

# 64. Worktrees

Sessões concorrentes no mesmo repo devem preferir isolamento.

Worktree é escolha do operador, não obrigação. Antes de permitir edição concorrente
no mesmo diretório, mostrar nome/caminho/estado/tempo das sessões existentes e oferecer
worktree, compartilhamento explícito ou voltar. Nova sessão recebe aviso próprio.
Registrar mudanças anteriores, detectar arquivos alterados desde a leitura e pedir
confirmação em conflitos, inclusive em Yolo; não sobrescrever silenciosamente.

---

# 65. Git safety

Detectar:

- dirty state;
- protected branch;
- upstream;
- force push;
- worktree ocupado;
- branch compartilhada.

Ações perigosas entram no policy engine.

Commit/push exigem pedido ou autorização para o trabalho atual, inclusive em Yolo.
Pasta-limite é o diretório de abertura, não raiz Git. Autonomia destrutiva e permissões
persistentes por ação/caminho seguem PD-08; não ampliar acesso através de symlinks.

---

# 66. Auto Mode

Auto Mode pertence ao Carapanã.

Não ao provider.

Pipeline:

```text
tool proposal
     ↓
deterministic policy
     ↓
context-aware policy
     ↓
optional decision engine
     ↓
LLM classifier
     ↓
execute / deny / human
```

---

# 67. Deterministic policy

Exemplos normalmente seguros:

```text
read ordinary workspace file
rg
git status
git diff
cargo check
cargo test
```

Exemplos normalmente bloqueados/escalados:

```text
rm -rf /
git push --force main
terraform destroy
DROP DATABASE
kubectl delete namespace
```

Nenhum LLM é necessário nesses extremos.

---

# 68. Context-aware policy

Exemplo:

```text
git push
```

não tem risco fixo.

Considerar:

```text
remote
branch
protected state
user intent
task
previous authorization
```

---

# 69. Decision engine

Abstração:

```rust
trait DecisionEngine {
    async fn decide(
        &self,
        request: DecisionRequest,
    ) -> Result<DecisionResponse>;
}
```

Implementações:

```text
builtin
jev
llm
hybrid
```

---

# 70. Jev

Jev ou alternativa equivalente será opcional.

Casos apropriados:

```text
safe / review / deny
continue / stop
retry / diagnose / escalate
skill selection
task complete? yes / no
doom loop? yes / no
model tier selection
```

Falha do Jev nunca derruba o harness.

Fallback:

```text
builtin
 ↓
LLM
 ↓
human
```

---

# 71. Classifier isolation

O classifier de segurança recebe apenas o mínimo necessário.

Não enviar automaticamente:

- chain-of-thought do agent;
- secrets;
- output bruto não confiável;
- arquivos inteiros.

---

# 72. Deny-and-continue

Policy denial não deve sempre interromper o usuário.

Fluxo:

```text
blocked
 ↓
agent receives structured reason
 ↓
tries safer strategy
 ↓
human only when genuinely necessary
```

---

# 73. Intent Ledger

Representação explícita do que foi autorizado.

```text
✓ modify workspace
✓ install local dev dependencies
✓ create feature branch
✓ commit

✗ merge PR
✗ modify main
✗ production deployment
✗ reveal secret values
```

---

# 74. Authorization scope

Cada autorização contém:

```text
action
resource
scope
source
expiry
```

Scopes:

```text
once
current task
```

Permissões persistentes são políticas configuradas separadamente.

---

# 75. Trust boundaries

Conceitos:

```text
workspace
remote repository
trusted domain
external domain
dev cloud account
production cloud account
secret store
```

Configuração pode declarar confiança explicitamente.

---

# 76. Sandbox

Interface:

```rust
trait SandboxBackend
```

## Linux

Investigar/usar:

```text
Landlock
namespaces
seccomp quando necessário
```

## macOS

```text
Seatbelt
```

## Windows

```text
Job Objects
AppContainer onde viável
```

Nunca mostrar:

```text
sandbox enabled
```

quando proteção real não estiver disponível.

---

# 77. Modes

```text
ask
auto
yolo
```

Modos de autonomia pertencem a perfis configuráveis, separados de Planejar/Executar.
Mesmo `yolo` respeita proteções de segredos, privilégios, trabalho anterior e bloqueios
catastróficos. Dentro da pasta da sessão pode executar ações destrutivas; fora dela
exige autorização específica, inclusive permissão persistente por ação/caminho.
Sem sandbox disponível, execução/alterações precisam de consentimento explícito e aviso
visível. Recursos sem implementação segura podem ficar indisponíveis naquele OS.

---

# 78. Secret Security

Arquivos protegidos por padrão:

```text
.env
.env.*
*.pem
*.key
id_rsa
id_ed25519
.aws/credentials
.npmrc
.pypirc
.netrc
kubeconfig
terraform.tfstate
```

Lista extensível.

---

# 79. Secret-aware tools

Preferir:

```text
secret.list_names
secret.exists
secret.describe
```

em vez de ler valor.

Valores secretos não são enviados ao modelo, mesmo com autorização ou em Yolo.
Não oferecer `secret.read_value` como forma de expor valores ao agente. Credenciais
podem ser usadas localmente por comandos autorizados, sem aparecer em modelo/chat/logs.
Até `.env.example` exige permissão antes da leitura; esperar nomes/placeholders não
dispensa verificar ausência de valores reais.

---

# 80. Redaction e tainting

Valor sensível autorizado entra marcado como:

```text
Sensitive<T>
```

Nunca deve ser:

- logado;
- armazenado em histórico de conversa ou artefatos de tarefa;
- enviado a classifier;
- enviado a qualquer modelo/provider;
- colocado em artifact;
- incluído em report.

Consentimento para uso local não libera esses destinos. A proteção precisa ser
comprovada também em outputs/env de comandos; bloquear fluxos que não puderem ser protegidos.
Armazenamento protegido de credenciais de autenticação continua seguindo a seção 54;
isso não autoriza sua inclusão no contexto, histórico ou outputs do agente.

---

# 81. Prompt injection

Tratar como conteúdo não confiável:

- web;
- README;
- issue;
- terminal output;
- logs;
- MCP resources;
- fetched docs.

Conteúdo externo nunca altera policy sozinho.

---

# 82. Skills

Categorias:

```text
builtin mandatory
builtin optional
official
user
project
```

---

# 83. Skills builtin

Inicialmente:

```text
security
maintainability
testing
local-development
frontend-design
git
rust
```

---

# 84. Skill security

Interfere em:

- context loading;
- file access;
- secret handling;
- tool execution;
- logging;
- validation.

Não será apenas prompt decorativo.

---

# 85. Skill maintainability

Antes de escrever:

```text
search existing implementation
search similar functions
search existing abstraction
inspect dependencies
inspect local conventions
```

Validar depois:

```text
duplication
dead code
premature abstraction
unnecessary dependency
copy-paste
god modules
```

---

# 86. Skill testing

Regra:

> nenhum funcionamento declarado sem evidência correspondente.

Tentar conforme aplicável:

```text
format
lint
build
unit
integration
e2e
smoke
```

---

# 87. Skill local-development

Objetivo:

> reproduzir localmente antes de depender de serviços externos.

Detectar:

```text
compose.yaml
Dockerfile
Containerfile
mise.toml
justfile
Makefile
.env.example
devcontainer
```

---

# 88. Local service substitutions

Quando adequado:

```text
PostgreSQL      local container
Valkey          local container
S3              MinIO
SMTP            Mailpit
Kafka           Redpanda
NATS            local NATS
OIDC            mock provider
webhooks        local receiver
AWS services    LocalStack somente quando útil
```

Não adicionar infraestrutura inútil.

---

# 89. Skill frontend-design

Fluxo obrigatório:

```text
inspect design system
inspect existing components
inspect typography
inspect tokens
inspect references
reuse primitives
implement
run
visually inspect
adjust
```

Regras contra UI genérica e inconsistência fazem parte dessa skill.

---

# 90. Skill loading

Não carregar todas sempre.

Matcher escolhe apenas as relevantes.

Pode utilizar:

```text
deterministic rules
Jev
small classifier
```

---

# 91. Skill paths

Compatibilidade desejada:

```text
.agents/skills/
~/.config/carapana/skills/
```

Import/compatibilidade com convenções de outros harnesses quando segura.

Evitar DSL própria.

---

# 92. MCP

MCP é first-class.

Transportes prioritários:

- stdio;
- Streamable HTTP.

SSE antigo apenas como compatibilidade quando necessário; implementações MCP atuais tratam Streamable HTTP como transporte remoto principal.

---

# 93. MCP hot reload

Modificar MCP:

```text
config changed
 ↓
registry diff
 ↓
start/stop/restart affected server
 ↓
refresh capabilities
 ↓
notify sessions
```

Sem reiniciar daemon.

---

# 94. MCP CLI

```bash
carapana mcp list
carapana mcp add
carapana mcp remove

carapana mcp enable
carapana mcp disable
carapana mcp restart

carapana mcp inspect
carapana mcp logs
carapana mcp test

carapana mcp pin
carapana mcp unpin

carapana mcp permissions
carapana mcp import
```

---

# 95. MCP scopes

```text
user
project
session
```

Exemplo:

```text
ai-memory      user/pinned
postgres       project
playwright     on-demand
```

---

# 96. Pinned MCP

Servidor `pinned`:

- sobe com daemon;
- health checked;
- disponível transversalmente;
- adequado a memória compartilhada.

Use case principal:

```text
ai-memory
```

---

# 97. MCP permissions

Registrar por tool.

Exemplo:

```text
ai-memory
  read_memory      allow
  write_memory     allow
  delete_memory    ask

postgres
  select           allow
  write            ask
  destructive      deny
```

Adicionar MCP não significa autorizar tudo.

Tools novas ou mudanças que ampliem acesso exigem nova aprovação; tools inalteradas
mantêm permissões. Configuração confiável não pode conceder privilégios novos sozinha.

---

# 98. MCP import

Planejar:

```bash
carapana mcp import claude
carapana mcp import opencode
carapana mcp import codex
```

O import deve mostrar diff antes de persistir.

---

# 99. MCP credentials

Sempre preferir flows seguros do protocolo.

Nunca pedir segredo no chat quando um OAuth/out-of-band flow adequado existir.

---

# 100. MCP failure handling

- exponential backoff;
- health status;
- logs limitados;
- restart isolado;
- session permanece viva;
- tool fica temporariamente unavailable.

---

# 101. Completion Gate

Antes de finalizar uma tarefa:

```text
agent says done
      ↓
completion gate
      ↓
requirements
diff
tests
build
runtime evidence
TODO/stub scan
      ↓
pass / incomplete / failed
```

---

# 102. Validation evidence

Persistir evidências estruturadas:

```text
command
exit code
duration
summary
artifact reference
timestamp
```

Não armazenar necessariamente stdout inteiro.

Validação incompleta não é sucesso validado: mostrar motivo e pausar fila até decisão.

---

# 103. Doom-loop detection

Detectar ciclos semanticamente equivalentes.

Exemplo:

```text
test
same error
edit
test
same error
edit
test
same error
```

Ao ultrapassar threshold:

```text
stop current strategy
re-read failure
inspect assumptions
choose alternative
```

Defaults configuráveis: três tentativas equivalentes sem progresso exigem diagnóstico
e outra estratégia; mais três sem avanço exigem pausa e intervenção, inclusive em Yolo.
Só evidência real reinicia contador. Diferenciar isso de retries de falha temporária
do provider; estes não autorizam loop infinito de edição/teste que esgote a cota.

---

# 104. Subagents

Planejados desde a arquitetura.

Exemplos:

```text
explorer
implementer
tester
reviewer
```

Cada um recebe:

```text
bounded task
bounded context
tool permissions
token budget
time budget
resource budget
```

Perguntar pede approval para delegar; Auto/Yolo podem delegar dentro de budgets e
permissões. Planejar só investiga. Não ampliar privilégios nem impor worktrees.
Sem capacidade para subagent, continuar sozinho ou aguardar; nunca aumentar limites
automaticamente. Pausar apenas se não houver alternativa dentro do orçamento.

---

# 105. Privilege inheritance

Subagent não herda automaticamente:

- secrets;
- destructive permissions;
- cloud permissions;
- external-write permissions.

---

# 106. Plugins

v0/v1:

```text
MCP
skills
hooks
```

Não implementar Rust plugin ABI inicialmente.

---

# 107. Hooks

Eventos possíveis:

```text
session.start
task.start
tool.before
tool.after
file.before_write
file.after_write
validation.start
validation.finish
task.complete
```

Hooks:

- timeout;
- bounded output;
- policy;
- structured payload.

Hooks seguem as mesmas permissões, sandbox, supervisão, proteção de segredos e limites
das tools. Consultas ai-memory no início/retomada e hooks antes de compactação e no
encerramento seguem PD-12. Gravações podem ser autorizadas por integração/projeto; exclusões e
gravações fora do escopo exigem autorização separada. Falhas são avisadas, sem apagar
histórico ou impedir encerramento. Memória é referência não confiável, não autoridade.

---

# 108. Multi-device

Carapanã daemon permite:

```text
local TUI
remote TUI
CLI
browser
mobile PWA
```

---

# 109. Remote access

Default:

```text
127.0.0.1 / local IPC
```

Nunca:

```text
0.0.0.0
```

automaticamente.

---

# 110. Tailscale/NetBird

Primeira estratégia recomendada de remote access.

Detectar interfaces conhecidas e permitir bind explícito.

Exemplo:

```text
tailscale0
wt0
lo
LAN
```

Não exigir exposição pública.

---

# 111. App-layer authentication

Mesmo dentro da VPN.

Pairing:

```bash
carapana device pair
```

Gerar:

- QR;
- short code;
- device key.

---

# 112. Device identity

Preferir Ed25519.

Cada device possui chave própria.

Revogação individual:

```bash
carapana device revoke <id>
```

---

# 113. Device permissions

Exemplo celular:

```text
view sessions       yes
send prompts        yes
approve              yes

reveal secrets       no
disable sandbox      no
edit global config   optional
```

Celular começa com visualização, prompts, intervenção/parada e approvals comuns.
Políticas globais, permissões permanentes e autorização sem sandbox ficam desativadas
por padrão, habilitáveis explicitamente por device. Não há device dono da sessão:
clientes autorizados compartilham fila única, com origem das mensagens e estado sincronizado.

Desconexão bloqueia comandos/approvals até snapshot atualizado. Preservar rascunhos,
sem enviar ações offline silenciosamente após reconectar.

---

# 114. PWA security

Service worker pode cachear:

```text
static app shell
icons
fonts
```

Não cachear automaticamente:

- código;
- prompts;
- tool output;
- secrets;
- diffs.

---

# 115. Resource Safety

Componente first-class.

Meta:

> sesssão idle custa quase apenas metadata.

---

# 116. Memory architecture

Não manter histórico completo da sessão na RAM.

Carregar:

```text
working set
active context
small indexes
current buffers
```

Restante:

```text
SQLite
artifact store
```

---

# 117. Tool output

Sempre streaming + bounded.

Nunca:

```text
read_to_string(unlimited)
```

para processo arbitrário.

---

# 118. Ring buffers

Output recente pode permanecer em RAM.

Ao exceder limite:

- summarize;
- truncate;
- spill to bounded artifact.

---

# 119. No per-token disk writes

Streaming do LLM:

```text
bounded memory buffer
 ↓
batch
 ↓
meaningful persistence
```

---

# 120. Session hibernation

Sessão idle:

- libera working set;
- libera parser caches;
- suspende MCP lazy;
- fecha processos temporários;
- mantém metadata.

---

# 121. MCP resource policy

Tipos:

```text
pinned
project
on-demand
```

Só `pinned` fica permanentemente ativo por decisão explícita.

---

# 122. Watchers

Nunca observar recursivamente repo inteiro sem filtros.

Respeitar ignore files.

Evitar watchers para árvores gigantes quando polling/eventos específicos resolverem.

---

# 123. Resource configuration

Exemplo conceitual:

```toml
[resources]
max_active_sessions = 8
max_concurrent_agents = 4
max_processes_per_session = 8

memory_soft_limit = "2 GiB"
memory_hard_limit = "4 GiB"

tool_output_memory_max = "16 MiB"
tool_output_capture_max = "256 MiB"

artifact_cache_max = "10 GiB"
log_storage_max = "2 GiB"
```

Valores finais serão calibrados por benchmark.

Sem budget para subagent, continuar sozinho ou aguardar capacidade, sem aumentar os
limites automaticamente. Pausar apenas quando não houver alternativa dentro do orçamento.
Medir harness e processos externos separadamente, mas controlar o total supervisionado.

---

# 124. Dynamic defaults

Defaults devem considerar:

- RAM disponível;
- disco livre;
- arquitetura;
- OS.

Considerar memória disponível e pressão atual, não somente RAM total. A referência
principal é Mac M3/16 GB; validar também Ryzen 9 7900/~64 GB e Tirion i5-8265U/~16 GB
via SSH. Tirion deve suportar uma sessão ativa com paralelismo adaptado. Metas numéricas
só após benchmarks reais, não no Delivery 0.

Máquina de 8 GB e máquina de 64 GB não devem ter exatamente o mesmo paralelismo.

---

# 125. Memory pressure

Soft limit:

```text
drop disposable caches
compact inactive sessions
hibernate idle sessions
reduce parallelism
spill bounded buffers
```

Hard limit:

```text
stop spawning subagents
pause low-priority work
warn user
```

---

# 126. Disk pressure

Warning threshold:

```text
disable nonessential caching
rotate logs
```

Critical threshold:

```text
block high-storage optional work
surface warning
```

Nunca apagar trabalho persistente automaticamente.

---

# 127. Cleanup priority

Pode apagar automaticamente conforme policy:

```text
expired cache
rotated disposable logs
temporary artifacts
```

Não pode apagar automaticamente:

```text
sessions
diffs
checkpoints
validation evidence
user files
```

sem política explícita.

---

# 128. Process supervision

Cada sessão possui árvore de processos controlada.

```text
Session
└── ProcessGroup
    ├── command
    ├── test server
    └── MCP
```

Encerramento:

```text
graceful
 ↓
timeout
 ↓
forced
```

Sem processos órfãos.

Ao parar, cancelar agente/comandos e encerrar processos temporários iniciados para o
trabalho. Não encerrar serviços anteriores nem processos marcados `Manter rodando`.
Preservar arquivos alterados, sem rollback automático; checkpoints seguem PD-06.

---

# 129. Resource UI

```text
Carapanã Resources

RAM
742 MiB / 2 GiB

Sessions
3 active
7 hibernated

Disk
Sessions     1.8 GB
Artifacts    3.2 GB
Logs         280 MB
Cache        640 MB
```

---

# 130. Resource CLI

```bash
carapana resources
carapana storage
carapana storage inspect
carapana storage clean
```

---

# 131. Resource benchmarks

Obrigatórios:

```text
idle daemon memory
1 active session
10 idle sessions
many persisted sessions
large tool output
long conversation
repo with millions of files
MCP lifecycle
crash recovery
```

---

# 132. Acceptance — resource safety

Obrigatório provar:

- abrir sessões históricas não carrega agents;
- output de múltiplos GB não entra inteiro na RAM;
- arquivo gigante não é carregado cegamente;
- symlink loop não trava traversal;
- WAL não cresce sem limite;
- logs rotacionam;
- child processes são encerrados;
- caches possuem teto;
- memória não cresce continuamente em sessão longa.

---

# 133. Development environment do próprio Carapanã

Carapanã deve praticar aquilo que cobra dos projetos.

```text
mise.toml
rust-toolchain.toml
justfile
compose.yaml somente se necessário
.env.example
```

Com:

```bash
mise install
just dev
```

---

# 134. Test doubles

Desenvolvimento principal não dependerá de provider real.

Criar:

```text
mock model backend
mock MCP
mock tool outputs
mock auth
mock remote client
```

---

# 135. Agent scenario tests

Fixtures determinísticas para:

```text
read-edit-test
approval
provider error
context compaction
doom loop
secret attempt
prompt injection
MCP failure
crash recovery
```

---

# 136. Rust tests

- cargo test;
- cargo nextest;
- proptest;
- insta;
- wiremock ou equivalente;
- criterion onde benchmarking fizer sentido.

---

# 137. Web tests

- Vitest;
- Playwright;
- visual regression.

---

# 138. TUI tests

Renderizar Ratatui para buffer e snapshot.

Cobrir:

- narrow terminal;
- wide terminal;
- 1 session;
- 30 sessions;
- long output;
- Unicode;
- approval;
- error;
- attention.

---

# 139. Visual regression

Golden states:

```text
sessions-empty
sessions-running
sessions-attention
session-active
session-approval
session-completed
settings
mcp
resources
mobile-home
mobile-approval
```

---

# 140. Security tests

Corpus adversarial:

```text
prompt injection
secret exfiltration
path traversal
symlink escape
shell injection
malicious MCP
malicious tool output
approval confusion
provider switch leak
```

---

# 141. Supply chain

Adicionar:

- cargo-deny;
- cargo-audit;
- dependency review;
- SBOM;
- release hashes;
- signed artifacts;
- provenance/attestations.

---

# 142. Dependency updates

Renovate.

Cobrir:

- Cargo;
- pnpm;
- GitHub Actions;
- Rust toolchain;
- docs tooling.

Nenhum workflow deve usar uma action antiga por ter sido copiada de exemplo.

Actions devem ser resolvidas na versão estável atual e, quando possível, pinadas por SHA imutável com Renovate cuidando dos updates.

---

# 143. CI

Pipeline:

```text
format
clippy
Rust tests
nextest
security
cargo-deny
web lint
web tests
Playwright
protocol contract tests
resource smoke tests
cross-platform build
```

---

# 144. Merge workflow

- `main` protegida;
- sem commit direto;
- Conventional Commits;
- trabalho via branch;
- PR obrigatória;
- considerar PRs/worktrees existentes antes de duplicar trabalho;
- rebase da branch antes do merge;
- merge normal preservando commits;
- sem squash automático.

---

# 145. Definition of Done

Uma tarefa não está concluída se faltar qualquer item aplicável:

```text
implementation
format
lint
build
tests
runtime validation
security check
resource impact
documentation
visual inspection
```

---

# 146. Documentation site

Stack:

```text
Astro
Starlight
```

Site:

```text
docs.carapana...
```

Domínio final não é requisito de desenvolvimento.

---

# 147. Docs information architecture

```text
Getting Started
├── Install
├── First session
├── ChatGPT
└── Local workflow

Concepts
├── Sessions
├── Auto Mode
├── Intent Ledger
├── Sandbox
├── Context
├── Skills
├── MCP
├── Subagents
└── Multi-device

Configuration
├── Layers
├── Providers
├── Models
├── Decision engine
├── Security
└── Resources

Remote
├── Tailscale
├── NetBird
├── Pairing
└── Devices

Reference
├── CLI
├── Config
├── Environment
├── Protocol
└── Exit codes

Troubleshooting
├── Doctor
├── MCP
├── Providers
├── Resources
└── Recovery
```

---

# 148. Generated documentation

Código é source of truth.

Gerar:

- CLI reference a partir do clap;
- config reference a partir do schema;
- default config;
- environment variables;
- protocol types quando possível.

Não manter seis versões manuais da mesma informação.

---

# 149. Observability

Local:

- tracing;
- structured logs;
- rotating log files.

Debug:

```bash
RUST_LOG=carapana=debug
```

---

# 150. Telemetry

Off por padrão.

Se existir posteriormente:

- opt-in;
- documentada;
- sem código;
- sem prompts;
- sem secrets.

---

# 151. Releases

Targets Tier 1:

```text
macOS arm64
macOS x86_64 quando necessário
Linux x86_64
Linux arm64
```

Windows poderá começar como Tier 2 e subir após estabilização.

---

# 152. Packaging

Avaliar `cargo-dist` atual no momento da implementação.

Fornecer futuramente:

```text
Homebrew
standalone binaries
cargo install quando adequado
```

Usuário final recebe produto pronto, sem precisar de Rust/Node para o Carapanã.
Dependências de projetos/MCPs ficam separadas e são explicadas pelo Doctor, sem
instalação silenciosa. Licença escolhida: Apache-2.0; incluir licença/notices na
distribuição e verificar compatibilidade de dependências/SDKs.

---

# 153. Self-update

```bash
carapana update
```

Canais:

```text
stable
beta
nightly
```

Update exige:

- assinatura/hash;
- manifest confiável;
- rollback quando possível.

---

# 154. Roadmap

## Delivery 0 — UI/UX Product Foundation

**BLOQUEANTE.**

Entregas já definidas nas seções anteriores.

Nenhum runtime real.

### Gate

Esperar aprovação explícita.

---

# 155. Delivery 1 — Engineering Foundation

Após aprovação do Delivery 0:

- Rust workspace;
- repo policies;
- CI;
- protocol types;
- config system;
- structured errors;
- tracing;
- test harness;
- deterministic mock provider;
- fake session engine suficiente para contracts.

Definir contratos de perfis/seleção, fila/edit/intervenção, resultados headless,
permissões por ação/caminho e confiança por workspace conforme o complemento.
Não confundir seleção do composer com mudança da execução atual.

Objetivo:

> construir fundação sem implementar autonomia ainda.

---

# 156. Delivery 2 — Daemon, Storage e Sessions

Implementar:

- daemon;
- SQLite;
- migrations;
- Session Registry;
- event persistence;
- snapshots;
- active-session detection;
- attach/detach;
- attention queue;
- crash recovery;
- hibernation base;
- process supervisor.

Persistir fila e seleções, recuperar sempre em pausa, distinguir processos anteriores,
temporários e marcados `Manter rodando`. Não repetir efeitos incertos automaticamente.

Conectar TUI/CLI à arquitetura real.

---

# 157. Delivery 3 — ChatGPT Provider

Implementar:

- provider abstraction;
- capabilities;
- official ChatGPT auth quando disponível;
- model streaming;
- tool calling;
- cancellation;
- rate-limit handling;
- reconnect;
- model switching.

O primeiro provider é exclusivamente SIWC oficial para contas elegíveis. Se o grant não permitir
uso do plano, manter o login com status de inferência desabilitada e solicitar escolha explícita;
não exigir API paga nem criar workaround não oficial.

Comprovar auth/inferência/capacidades na conta ChatGPT autorizada, sem tools executáveis
nesta fase. Implementar modelos/variantes sem fallback silencioso, confirmação ao trocar
provider e tratamento explícito de cotas conforme PD-10. Não repetir inferência automaticamente.
API paga não vira requisito do produto.

### Progresso do Delivery 3 — 13 de outubro de 2026

- `carapana-core::provider` contém catálogo puro de provider/modelo/variante, capabilities
  tipadas e resolução exata sem fallback. A fatia não adiciona adapter, autenticação, rede,
  inferência ou execução de tools; decisão em PD-20.
- O contrato mock-only usa `futures-core::Stream`, eventos/errors tipados e cancelamento
  cooperativo runtime-independent. Não escolhe executor, não faz retries/reconnect e não conecta
  o stream ao daemon/session engine; pedidos de tool são somente dados. Decisão em PD-21.
- Em 13/10/2026 o operador autorizou explicitamente implementar SIWC oficial, autenticação,
  inferência real e envio de contexto ao provider, sem estender a autorização a tools/tarefas,
  subprocessos, autonomia, deploy ou merge.
- Implementada a primeira fatia SIWC em `carapana-openai`: dynamic registration oficial, callback
  loopback/PKCE, state/nonce, validação de ID token/JWKS, checagem de escopo/identidade e persistência
  privada owner-only fora do SQLite. Refresh token é rotacionado sob lock interprocesso por conta.
- Implementado catálogo de modelos visíveis e adapter Responses SSE com `store:false`, streaming,
  cancelamento, limites de tamanho, erros/cotas tipados e sem retries. Tool calls inesperadas continuam
  dados inertes e não são enviadas na requisição.
- CLI agora oferece `chatgpt sign-in`, `accounts`, `models` e `smoke-test`; o último exige
  `--confirm-remote` e envia somente um prompt sintético fixo, sem sessão/workspace. Nenhum login,
  chamada real de inferência ou envio de contexto foi executado durante os testes locais.
- Testes usam o cliente de produção contra servidor HTTP falso local; incluem callbacks loopback,
  rotação de refresh, segurança de arquivos, erros, streaming e cancelamento. O teste real na conta
  autorizada ainda é um gate manual e não deve entrar nos testes comuns.
- Tools executáveis, execução de tarefas/processos e autonomia seguem desativadas. A fila e o daemon
  continuam desconectados do adapter nesta entrega.

---

# 158. Delivery 4 — Core Coding Tools

Implementar:

- filesystem;
- search;
- tree-sitter;
- shell;
- process execution;
- Git;
- atomic editing;
- diff;
- worktree awareness;
- artifact handling.

### Gate bloqueante antes de tools reais

Antecipar a camada mínima de segurança, sem esperar o Delivery 6 completo:

- proteção de arquivos sensíveis, inclusive `.env.example`, env e outputs;
- uso local autorizado de credenciais sem exposição ao modelo/chat/logs/memória;
- approvals básicos e diretório-limite de abertura, com validação de alvos/symlinks;
- confiança explícita em configs de workspace, sem ampliação automática de permissões;
- bloqueio de elevação de privilégios;
- sandbox efetivo onde suportado, ou autorização explícita e aviso para execução sem ele;
- supervisão/cancelamento de processos e preservação de alterações anteriores;
- worktrees opcionais, com confirmação de edição concorrente no mesmo diretório.

Controles devem ser comprovados antes de ativar a capacidade correspondente. Se não
for possível garantir proteção, bloquear a capacidade. Essas implementações são após
aprovação do Delivery 0; não antecipar runtime para produzir protótipos.

### Progresso do Delivery 4 — 16 de outubro de 2026

- A primitive `WorkspaceMetadata::read_file` em `carapana-storage::workspace` valida a identidade do
  workspace e abre arquivos Unix sem seguir symlinks. Aceita caminho relativo, arquivo regular e até
  256 KiB; `read_file_with_limit` permite um limite menor. Bloqueia nomes sensíveis conhecidos e
  falha em plataformas sem enforcement comprovado. Decisão PD-23.
- O novo crate `carapana-tools` lista arquivos relativos com `.gitignore`, `.ignore` e
  `.carapanaignore`, sem regras globais/ancestrais, sem symlinks ou travessia de filesystem, com
  exclusões padrão e limites de profundidade/entradas. Busca literal reutiliza o leitor seguro e limita
  arquivos, bytes e resultados. Decisão PD-24.
- Ignore files são lidos por `WorkspaceMetadata::read_file` e compilados de memória; a camada do
  parser não reabre caminhos. A enumeração agora é relativa a descritores em `carapana-storage`: abre
  cada componente com `openat`/`O_NOFOLLOW`, enumera via `rustix::fs::Dir` e classifica entradas com
  `statat(..., SYMLINK_NOFOLLOW)`. `carapana-tools` conserva a política de recursão/ignore; cada
  diretório usa a identidade descoberta, limite de entradas e revalidação da cadeia interna do
  workspace antes/depois. O orçamento conta também entradas omitidas por sensibilidade ou filesystem.
  Mudanças/substituições observadas falham fechadas; entradas protegidas e filesystems diferentes não
  são retornados. Decisão PD-27.
- Parser Tree-sitter sob demanda suporta Rust, TypeScript e TSX, conforme escolha do operador. Lê
  somente um arquivo explícito pelo leitor seguro; limita source, tempo, nós e diagnósticos e não
  devolve trechos de código. JavaScript/Svelte continuam fora do escopo inicial. Decisão PD-25.
- Prévia unified diff em memória compara dois textos fornecidos pelo chamador, sem ler nem alterar
  arquivos; limita entradas/tempo/saída e substitui controles de terminal/bidi. É somente visualização,
  não produz patch aplicável. Decisão PD-26.
- PD-28 adiciona snapshot efêmero não serializável e `WorkspaceMetadata::replace_file_atomically`, só
  para arquivos existentes de até 256 KiB vinculados ao snapshot. Usa temp owner-only, revalida bytes,
  metadata, extended attributes e cadeia de diretórios, rejeita symlinks, hard links, atributos
  estendidos, arquivos protegidos e metadata não preservável; o rename é relativo a descritores e o
  resultado pós-rename pode ser incerto. O arquivo passa brevemente por mode owner-only após o rename;
  crash/falha de cleanup pode deixar um temporário oculto sem recovery automático. Isso detecta conflitos
  observados, mas não é compare-and-swap: uma escrita/remoção externa na janela final ainda pode vencer;
  não habilita edição concorrente nem runtime.
- Testes de filesystem real cobrem leitura e limites, traversal, symlinks em arquivo/diretório,
  ignore files symlink/oversized, diretório não regular, nomes protegidos, regras ignore e precedência,
  enumeração relativa a descritores, substituição de diretório antes e durante a revalidação e limites
  de entradas; também cobrem matchers compilados de bytes lidos sem reabertura, exclusão de regras
  ancestrais, profundidade, arquivos ignorados, busca literal/case ASCII, binários, UTF-8 inválido,
  orçamento aplicado no limite de leitura, snippets sem controles de terminal e parser real de
  Rust/TypeScript/TSX com erros de sintaxe, posições, limites e ausência de source no resultado. A
  escrita tem regressões para substituição/mode, conflito e destino removido, symlink/substituição de
  diretório, workspace errado, limite de bytes, hard links e arquivos com extended attributes.
- As APIs continuam desconectadas do provider, `ToolCallRequested`, daemon, CLI/TUI, sessão e fila;
  nenhum conteúdo foi enviado. Ainda faltam diff ligado a arquivos/edições, Git/worktree, artifacts,
  execução e prova do gate de segurança. O diff atual é apenas prévia em memória. A denylist de nomes
  não detecta segredos em conteúdo; a escrita não coordena processos externos, e traversal/writes não
  devem ser ligados em runtime antes das revisões e gates próprios.
- Delivery 4 permanece incompleto. A implementação descriptor-relative continua isolada e não está
  ligada a runtime; ainda exige revisão/gates próprios e não equivale a proteção de conteúdo secreto,
  autorização, sandbox ou controle de execução. Não inferir autorização ou segurança a partir dessas
  primitives; cada capacidade seguinte exige enforcement e testes próprios antes de qualquer wiring.

### Progresso do Delivery 4 — 18 de outubro de 2026

- O CI macOS revelou fixtures de teste sob `/var` (alias symlinkado de `/private/var`), que os validadores
  corretamente recusam, e casos de filenames não UTF-8 que o filesystem macOS rejeita (`EILSEQ`). Após
  canonicalizar `/var`, um novo run mostrou que o prefixo de temp resultante tornava os caminhos de
  socket maiores que `SUN_LEN`; os fixtures com socket agora usam `/tmp` canonicalizado e verificam o
  limite de comprimento. A cobertura de filenames não UTF-8 permanece nas plataformas compatíveis; as
  proteções de runtime não foram afrouxadas.
- `TMPDIR=/var/tmp mise exec -- make verify` passou localmente depois da correção: formatação, lint,
  check, build, 24 testes Vitest, 64 E2E, Clippy e testes Rust do workspace.
- O run 39135075699 confirmou Rust/Linux verde. No run 39135994968, Rust/Linux passou, macOS ainda
  falhou por caminhos de socket acima de `SUN_LEN` após a canonicalização de `/var`, e Web/Linux
  continuou falhando nas quatro golden screenshots que também falharam na `main` (`24e1c9c`, run
  38352209334). A correção atual encurta somente os caminhos dos fixtures; reexecutar CI depois do push.
  A divergência de screenshot continua como baseline aberto; não atualizar snapshots sem validar.
- `carapana-tools` agora oferece `create_snapshot_diff_preview`: o diff deriva os bytes-base do
  `WorkspaceFileSnapshot`, carrega o path pelo mesmo objeto emprestado, rejeita conteúdo-base que não
  seja UTF-8 e reutiliza os limites/sanitização do preview existente. Testes cobrem associação ao path,
  ausência de escrita e recusa de UTF-8 inválido. A capacidade segue local e sem aplicação ou wiring.
- O parser puro de `git status --porcelain=v2 -z --branch` preserva paths como bytes e modela status de
  index/worktree, rename/copy, conflitos e metadados básicos de branch. Limita stdout recebido a 4 MiB,
  20.000 entradas e 16 KiB por path; não invoca Git, toca filesystem, autoriza caminhos ou faz wiring.
  Decisão PD-30; executar o CLI e conectar a saída continuam dependentes dos gates de processo e segurança.
- O parser puro de `git worktree list --porcelain -z` preserva paths/refs/reasons como bytes e modela
  HEAD, branch, detached, bare, locked e prunable. Limita saída a 4 MiB, 256 worktrees, 16 KiB por path
  e 4 KiB por metadado; não invoca Git, acessa paths ou equivale à detecção de sessões concorrentes.
  Decisão PD-31; listagem operacional e UI de convivência continuam pendentes dos gates.
- O run 39136617620 passou Rust/Linux e encontrou no macOS uma expectativa de `event_sequence` desatualizada
  (6 vs. 5 porque o filename não UTF-8 é omitido); a asserção agora é por plataforma. No run seguinte,
  39137785894, Linux/Rust passou, mas 12 testes `daemon_process` no macOS falharam porque o socket sob
  `HOME/Library/Application Support/Carapana/daemon.sock` excedia `SUN_LEN`. A raiz temporária estava em
  `/tmp`, porém o prefixo comprido do fixture mais os componentes de HOME ultrapassavam o limite. A
  regressão simulada de caminho Darwin falhou antes da correção e passou após encurtar só o nome da pasta
  temporária; os 18 testes `daemon_process` passaram localmente. Aguardando CI após o próximo push.
- No run 39138681004, a correção de caminho confirmou no macOS: os 13 testes unitários da CLI e os 17
  `daemon_process`, incluindo a asserção de `SUN_LEN`, passaram. O job falhou depois em
  `process_supervisor::should_reject_an_unavailable_private_host_without_leaking_specification`: o
  fixture hardcoded `/bin/true` não pôde ser canonicalizado (HostUnavailable) no runner macOS. Trocado por
  `sleep_binary()`, já localizado de forma portátil pelo mesmo arquivo de teste; o caso agora descreve
  explicitamente um executável que não implementa o protocolo do host e passou localmente. Revalidar no
  próximo CI.
- O run 39139075795 confirmou no macOS a correção anterior: passaram 13 testes unitários da CLI, 17
  `daemon_process` e 12 testes `process_supervisor` (2 fixtures ignorados). Falhou o smoke test PTY porque
  o `tempfile.TemporaryDirectory` do script Python usou o padrão macOS `/var/folders`; o validator recusou
  corretamente o componente symlink `/var`. O script agora aloca sob `/tmp` canonicalizado com prefixo
  curto e verifica o limite de socket `<104` bytes em Darwin. O teste PTY focado passou localmente; aguardar
  CI para confirmar no macOS.
- O run 39139671799 confirmou `/tmp` e o socket no macOS: passaram 13 testes unitários da CLI,
  17 `daemon_process` e 12 `process_supervisor` (2 fixtures ignorados), mas `tui_process` recebeu `EIO`
  em `tcgetattr(slave)` depois que o TUI (líder da sessão PTY) saiu. A leitura não chegou a comparar flags,
  então não demonstra falha de restauração. O smoke agora mantém um supervisor/líder de sessão vivo enquanto
  verifica as flags restauradas, e só então o libera; o teste focado passou no Linux. Revalidar no macOS.
- Web/Linux repetiu as quatro falhas conhecidas de screenshot (`home-light/dark`, desktop/mobile) no run
  39137785894; sem evidência nova de causa, baseline não foi alterado.
- Web/Linux repetiu as mesmas quatro falhas de screenshots no run 39138681004; sem evidência nova, baseline
  continua intocado.
- Web/Linux repetiu as quatro falhas de screenshots no run 39139075795; sem evidência nova, baseline segue
  intocado.
- Web/Linux repetiu as mesmas quatro screenshots no run 39139671799; sem causa nova, baseline segue
  intocado. Rust/Linux passou; Rust/macOS falhou apenas no `tcgetattr` pós-saída do TUI.
- Run 39140326674, no commit 0dfb674, passou `make verify-rust` em Ubuntu e macOS; o smoke PTY e as
  correções de socket/processo ficaram verdes nas duas plataformas. Web/Linux falhou somente nas quatro
  golden screenshots já conhecidas (`home-light/dark`, desktop/mobile). Nenhuma baseline foi alterada;
  PR #25 segue draft/open, sem merge/deploy.
- Adicionada a seção 185 ao final do plano para avaliar Brotli e Zstandard em tráfego local/remoto,
  com comparação mensurável, compatibilidade/negociação, limites de descompressão, análise de oracle,
  licença/manutenção e decisão explícita antes de adicionar codec ou alterar framing.

### Progresso do Delivery 4 — 19 de outubro de 2026

- No run 39140644424, Rust/Linux e Rust/macOS passaram; Web/Linux repetiu somente as quatro falhas dos
  goldens `home-light`/`home-dark` desktop/mobile. Os artefatos foram inspecionados: sem deslocamento
  estrutural visível, com diferenças concentradas nas bordas/rasterização do texto. A renderização de
  fontes do host é hipótese, não causa comprovada; a execução Playwright local com o snapshot Ubuntu
  passou nos quatro casos. Nenhum baseline foi alterado; manter a investigação aberta.
- O run 39313363432 também passou nos dois jobs Rust. No Web/Linux, 60 testes passaram e somente os quatro
  mesmos goldens falharam, com diferenças estáveis reportadas de 509/514 pixels no desktop e 30/8 no
  mobile (aprox. 0,01%); nenhum baseline foi atualizado. PR #25 permanece draft/open, sem merge/deploy.
- No primeiro gate local completo, os 64 E2E passaram, mas dois testes de processo leram `Active` logo
  após observar o arquivo do socket. `DaemonService::open` reserva o socket antes de concluir recovery;
  os testes agora aguardam um RPC `show` somente de leitura antes de inspecionar SQLite. Sem mudança no
  runtime: os dois casos direcionados passaram e a suíte `daemon_process` passou três vezes consecutivas.
- Criado harness local sem dependência/runtime ou rede para comparar ausência de codec, Brotli 4/6 e
  Zstandard 1/3 em nove cargas exclusivamente sintéticas, incluindo expansão para 32 B, JSON até o limite
  de 64 KiB, diffs e SSE agregado. Foram feitas três execuções de 300 amostras por fase; relatório e dados
  estão em [avaliação de compressão](compression-evaluation-2026-10-19.md) e
  [resultados JSON](benchmarks/compression-evaluation-2026-10-19.json).
- Acrescentado benchmark experimental de socket local com streams nativos incrementais: frame sintético
  de 64 KiB e SSE de 256 KiB dividido em 1.292 eventos, flush por evento, round-trip e latência do primeiro
  evento. Três campanhas seriais de dez medições mostraram grande redução de bytes, mas round-trip/CPU
  mais altos com flush por evento que sem compressão. Código e resultados ficam isolados em
  `prototypes/compression_transport_eval.py` e
  `docs/benchmarks/compression-transport-evaluation-2026-10-19.json`; isso não representa framing real,
  HTTP/TLS ou rede, nem decide um codec.
- Documentação oficial confirma `Content-Encoding: zstd` em request bodies de `POST /v1/responses` e
  limites documentados, mas não documenta compressão da resposta SSE. Compatibilidade desse caminho com
  token SIWC, HTTP remoto e hardware modesto/macOS não foi validada; nenhuma chamada remota foi feita.
  Testes locais cobrem round-trip, bytes inválidos, stream truncado e limite absoluto de saída, não todos
  os orçamentos adversariais necessários para decoder de produção. Nenhum codec selecionado/habilitado,
  dependência adicionada ou runtime ligado.
- Depois do run 39327747278, três ciclos de CI no PR #25 revelaram estados seguintes da sequência golden:
  `home` (39330682251), `approval` (39331491596), `resources` (39331491596) e `providers`
  (39332307370). As capturas Ubuntu correspondentes foram inspecionadas antes de atualizar apenas os
  PNGs Ubuntu: commits `c98e848` (home), `dc9e269` (approval) e `138ac08` (resources); o run seguinte
  também validou `provider-mapped-light` mobile. A alteração Providers atualmente em revisão contém
  quatro capturas do run 39332307370 (providers claro/escuro desktop, providers escuro mobile e provider
  customizado claro mobile). Nenhum baseline Linux foi alterado.
- `TMPDIR=/var/tmp mise exec -- make verify` passou localmente após a atualização das capturas dos
  estados Providers: formatação, lint, check, build, 24 testes web, 64 E2E, Clippy e suíte Rust completa.
  O run 39332307370 passou Rust/Ubuntu, mas Web/Linux ainda falhou nos quatro goldens Providers. Rust/macOS
  teve uma falha em `auth::tests::loopback_callback_rejects_wrong_state_then_accepts_the_active_registration`:
  reset de conexão no fixture e `CallbackTimeout` após 300 s. Não houve alteração no teste de auth; aguardar
  nova execução para distinguir intermitência de regressão.
- O run 39333942349 passou Rust/Ubuntu e Rust/macOS (incluindo o teste de callback OAuth), mas Web/Linux
  avançou e falhou somente em quatro estados seguintes: `provider-mapped` desktop claro/escuro e
  `updates-available` mobile claro/escuro. As capturas e os baselines foram comparados; as diferenças são
  restritas à rasterização de texto. Atualizados somente esses quatro PNGs Ubuntu; o gate local completo
  voltou a passar, incluindo 24 testes web, 64 E2E e a suíte Rust. O run seguinte ainda precisa confirmar
  os baselines e validar o restante da sequência, inclusive `updates-installed`.
- Para aprofundar a avaliação sem acesso a hardware modesto/macOS, os dois harnesses foram medidos em
  container rootless offline com quota de 1 CPU e limite de 512 MiB. O processador físico continua sendo
  um Ryzen 9 7900; portanto, é somente proxy de concorrência, não substituto de hardware lento. Os oito
  testes dos harnesses passaram no mesmo container; resultados brutos e limites observados estão em
  `docs/benchmarks/compression-*-2026-10-19-cpu-quota*.json` e a análise em
  `docs/compression-evaluation-2026-10-19.md`. Nenhum codec foi selecionado/habilitado e runtime não mudou.
- No run 39334633010 (head `b290693`), Rust/Ubuntu e Rust/macOS passaram. Web/Linux chegou aos últimos
  checkpoints golden e falhou somente em `provider-custom` desktop claro/escuro e `updates-installed`
  mobile claro/escuro. Capturas, diffs e conteúdo foram revisados; atualizados apenas esses quatro PNGs
  Ubuntu. Após a atualização, `TMPDIR=/var/tmp mise exec -- make verify` passou localmente, incluindo 64
  E2E e a suíte Rust. A validação CI desses últimos baselines ainda está pendente; PR #25 segue draft/open.

### Progresso do Delivery 4 — 21 de outubro de 2026

- O run `39335241433`, no HEAD `8fe76f9`, passou Rust/Ubuntu e Rust/macOS. Web/Linux passou formatação,
  lint, check, build e 24 testes Vitest; os únicos failures foram os goldens desktop `updates-available`
  claro/escuro (158/131 pixels, diferenças localizadas em rasterização de texto). Capturas, DOM e diffs
  do artifact do run foram revisados; conteúdo e layout permaneceram iguais. Atualizados somente os dois
  PNGs `-ubuntu` a partir das capturas do runner; nenhum baseline `-linux` foi alterado.
- `TMPDIR=/var/tmp mise exec -- make verify` passou depois da atualização: 24 testes Vitest, 64 Playwright,
  formatação/lint/check/build, Clippy, suíte Rust completa e build Rust. A validação no runner Ubuntu dos
  dois PNGs atualizados ainda depende de novo CI; não inferir aprovação remota a partir do teste local,
  que usa baselines `-linux`.
- A fatia de artifact store aprovada em PD-32 foi implementada isoladamente em `carapana-storage`: migration
  SQLite v6, catálogo sem payload/path de origem, blobs privados 0600 sob diretório 0700, reserva de quota
  transacional (10 GiB por usuário / 256 MiB por item), stream bounded com SHA-256, leitura verificada,
  paginação, uso, exclusão explícita, expiração temporária e reconciliação limitada. Advisory file locks
  protegem writers/leitores ativos da limpeza/remoção; erros de commit incertos são identificados e conteúdo
  durável ausente/corrompido não tem metadata removida silenciosamente. Sem wiring em tools, provider, daemon
  ou UI; sem compressão.
- Testes usam SQLite e filesystem reais e cobrem quota concorrente, limites, falha de source/sink, integridade,
  permissões, symlink, retenção, locks durante cleanup, recovery de estados pendentes/deleting, órfãos e
  migration de v5. A crate de storage passou com 80 testes; `cargo clippy --workspace --all-targets --locked
-- -D warnings`, `cargo build --workspace --locked`, formatter e gates Web (24 Vitest, 64 E2E) passaram.
- `make verify` não completou apenas por `DiskFull` em `daemon_process::should_return_complete_workspace_review_near_the_ipc_frame_limit`
  ao gravar SQLite. O `/tmp` do runner local é tmpfs de 31 GiB, 100% usado (76 KiB livres; inodes em 24%);
  `/var/tmp` tem espaço, mas o fixture desse teste canonicaliza `/tmp` explicitamente. Reexecutar o workspace
  sem esse teste passou todas as demais suites, com 17/18 testes `daemon_process` verificados no primeiro run.
  Nenhum arquivo foi removido para liberar espaço; não há evidência de falha causada pelo artifact store.
- CI remoto ainda não validou os dois snapshots Ubuntu atualizados nem esta implementação: o run 39335241433
  antecede essas alterações e não foi feito push. O Delivery 4 continua incompleto: shell/process execution,
  integração operacional Git/worktree e os gates de segurança/runtime não foram ligados nem autorizados.
- Após o ambiente voltar a ter espaço em `/tmp`, `TMPDIR=/var/tmp mise exec -- make verify` passou
  integralmente, sem skips adicionais: formatter/lint/check, 24 testes Vitest, 64 E2E, Clippy, todos os
  testes Rust (incluindo os 18 `daemon_process` e 80 de storage) e build. Nenhum arquivo alheio foi removido.
- Commit `4e63404` enviado à branch existente. A revisão seguinte reproduziu uma lacuna de crash recovery:
  um blob criado antes do commit SQLite pode ocupar o próximo ID após rollback. A alocação agora preserva
  arquivos existentes e tenta o próximo ID com limite explícito de 128 tentativas. Bytes de blobs órfãos
  gerenciados contam contra a quota mesmo durante a janela de retenção; a admissão falha fechada quando
  a enumeração ultrapassa o limite de accounting. Testes de filesystem real cobrem colisão, limite de
  tentativas e recusa de quota por órfãos recentes; sem ampliar permissões nem fazer wiring de runtime.
- O run `39662001311`, em `4e63404`, passou Rust/Ubuntu e Rust/macOS. Web passou 62/64 E2E, incluindo
  os goldens `updates-available` corrigidos; falhou apenas em `updates-installed` desktop claro/escuro
  (148/129 pixels). Capturas esperadas/atuais e diffs foram revisados antes de atualizar somente esses dois
  PNGs Ubuntu: conteúdo e layout iguais, diferenças localizadas de rasterização. Baselines Linux intactos.
- Após a correção de recovery/contabilização de órfãos, `TMPDIR=/var/tmp mise exec -- make verify` passou
  integralmente, com 83 testes de storage. Um novo CI ainda deverá confirmar a alteração de recovery e
  os últimos dois PNGs Ubuntu.
- O run `39662883496`, no commit `3f76175`, passou todos os jobs: Web/Linux e Rust/Ubuntu/macOS.
  A sequência completa de screenshots Ubuntu ficou verde. Sem merge/deploy.
- Após escolha explícita do operador por Bubblewrap/Linux (PD-33), `carapana-sandbox` adiciona uma prova
  sintética isolada e a CLI `sandbox-probe`, sem comando/path fornecido pelo usuário. A prova usa fixtures
  privados, mounts read-only e scratch limitado; verifica namespaces, `no_new_privs`, capabilities,
  filesystem e loopback isolados. Falha sem fallback, não reconfigura o host, limita output e tempo.
  `CARAPANA_REQUIRE_BUBBLEWRAP=1` exige enforcement positivo nos testes; o modo comum aceita indisponibilidade
  segura e não implica sandbox funcional naquele runner. Não é executor geral e não habilita tools.
- O gate `TMPDIR=/var/tmp CARAPANA_REQUIRE_BUBBLEWRAP=1 mise exec -- make verify` passou integralmente
  após esta fatia: cinco testes de sandbox (incluindo enforcement real, timeout e pipes limitados), dois
  testes CLI de processo, restante do workspace, formatter/lint/check/Clippy/build e 24 Vitest/64 E2E.
  A prova positiva foi realizada no host Linux local; o CI macOS deve testar somente a recusa unsupported.
- Revisão de preservação reproduziu e corrigiu o compartilhamento acidental de `artifacts/` por dois
  catálogos com nomes diferentes: a API aceita apenas `sessions.sqlite3` por diretório privado e recusa
  outros nomes antes de criar arquivos. O teste de regressão falhou antes da correção e passou depois.
- Adicionado [threat model](threat-model.md) com atores, ativos, fronteiras, evidência existente, lacunas e
  ordem das próximas fatias antes de execução real. Documenta que sandbox sintético e grants modelados
  não equivalem ao enforcement de um executor; `ToolCallRequested` continua inerte.
- O CI `39664308695`, em `d9e5720`, passou Web/Linux e Rust/Ubuntu/macOS. Depois da correção de
  isolamento de catálogos, o gate local completo passou novamente com enforcement Bubblewrap obrigatório,
  84 testes de storage, cinco testes de sandbox, dois CLI do probe, 24 Vitest e 64 E2E. O teste macOS
  comprova recusa unsupported, não disponibilidade de sandbox naquele OS.
- PD-34 adiciona o contrato puro `command_approval` no core: proposta imutável e limitada, solicitação
  com identidade opaca em memória, prazo monotônico, estado único e token não clonável ligado à proposta.
  Eventos de solicitação/workspace/tarefa errados não consomem o pedido; intervenção ou descarte do
  controlador revoga tokens já emitidos. Não há spawn, acesso a paths, serialização nem wiring de tools.
- O gate completo com Bubblewrap obrigatório passou após PD-34: 41 testes unitários de core, dois de demo
  e um doctest compile-fail comprovando que o token não é clonável; também passaram os 84 de storage,
  sandbox/CLI, restante do workspace, formatter/lint/check/Clippy/build e 24 Vitest/64 E2E.
- PD-35 adiciona `process_output` isolado no daemon: tails reservados por stdout/stderr com orçamento
  agregado, alocação lazy, bytes brutos sem Debug/Clone/serialização e sinal explícito de esgotamento.
  Não lê pipes, inicia processos, redige segredos, renderiza, persiste ou envia output. Testes exercitam
  saturação compartilhada, ring wrap, preservação de stderr em flood de stdout, bytes binários, limites,
  ausência de alocação em idle e comparação com modelo de referência em chunks intercalados.
- O CI `39665733418`, em `4dc5e54`, passou todos os jobs, incluindo a nova aprovação de comando em
  Ubuntu/macOS e Web/Linux. O run anterior `39664983864` foi cancelado pela atualização da branch, não
  falhou por regressão (Web e Rust/Ubuntu já haviam passado).
- O gate da fatia de output encontrou novamente `/tmp` sem espaço, agora em fixtures de TUI/CLI que
  ignoravam `TMPDIR`. Um teste Linux demonstrou a divergência `/tmp` versus `/var/tmp` antes da correção.
  Fixtures Rust com sockets e o smoke PTY Python agora respeitam `TMPDIR` no Linux, mantendo `/tmp`
  canonicalizado no macOS e as verificações de tamanho existentes. Nenhuma validação de runtime mudou
  nem foi removido arquivo de outra tarefa. O gate completo com `TMPDIR=/var/tmp` e Bubblewrap obrigatório
  passou depois, com nove testes de output, 36 unitários de daemon, 14 unitários CLI, 41 de core,
  84 de storage, demais suites Rust, doctests de privacidade/token, 24 Vitest e 64 E2E.
- O operador escolheu staging de snapshots explícitos (PD-36). `carapana-sandbox` prepara somente paths
  selecionados em árvore efêmera privada, com orçamento antes da leitura, manifest SHA-256 e verificação
  bounded de conteúdo/inventário (inclusive recusa de arquivos extras/protegidos inseridos depois).
  O probe Bubblewrap agora monta esse staging, não a origem, e testa ausência do fixture `.env.example`.
  Não há launcher geral, aplicação de mudanças nem compartilhamento automático.
- O run `39667041238`, em `1e057ba`, passou Web/Linux e Rust/Ubuntu, mas macOS repetiu o reset/timeout
  do callback OAuth observado anteriormente. Teste TCP local forçando accepted socket nonblocking e
  headers fragmentados reproduziu rejeição antes da correção. O reader agora normaliza blocking;
  o parser limita bytes antes de `read_line` e encerra em EOF incompleto, evitando crescimento/spin
  ilimitados. Quatro regressões locais cobrem fragmentação, EOF, limites e Host exato; a suíte OpenAI
  passou 32 testes. Confirmação da intermitência macOS ainda depende do próximo CI; não foi feita
  inferência ou autenticação real, nem alterada a validação de state/client_id.
- A correção de callback foi enviada separadamente em `67099fe`; testes de callback passaram também
  30 execuções locais consecutivas. Após finalizar snapshots, o gate completo com Bubblewrap obrigatório
  passou novamente: 13 unitários de sandbox e doctest de privacidade do bundle, 32 OpenAI, 41 core,
  36 daemon, 84 storage, demais suites Rust, formatter/lint/check/Clippy/build e 24 Vitest/64 E2E.
- O CI `39669535201`, no HEAD `aca0ce4`, passou Web/Linux e Rust/Ubuntu/macOS, confirmando snapshots
  selecionados e a correção do callback nas duas plataformas. PR #25 continua draft/open; sem merge/deploy.
- O operador aprovou compartilhar a quota de 10 GiB entre artifacts e staging (PD-37). A implementação
  adiciona leases pendentes com lock e árvore privada gerenciada, identity metadata no SQLite e schema v7
  para bloquear cleaners antigos. Remoção incerta ou troca de identidade preserva reserva/dados;
  órfãos sem controle também contam na admissão e respeitam graça de sete dias. Nenhuma evicção de duráveis.
- `ReservedSelectedSnapshotBundle` mantém a reserva global antes de ler/copiar conteúdo, é distinto do
  bundle sintético não contabilizado e remove a árvore antes de liberar a lease. O launcher futuro deverá
  exigir esse tipo. A primitive não autentica seleção, classifica segredos ou executa comandos.
- Os testes direcionados de storage/sandbox passaram (94/16 unitários, além de doctests), com SQLite e
  filesystem reais: concorrência de quota, lease ativa/abandonada, falha de preparação, recusa antes da
  leitura, identity swap, symlinks externos preservados, crash antes de publicar identidade, migração v6→v7
  e graça independente para árvore recente versus controle antigo.
- `TMPDIR=/var/tmp CARAPANA_REQUIRE_BUBBLEWRAP=1 mise exec -- make verify` passou integralmente após
  PD-37: formatter/lint/check/Clippy/build, 24 Vitest, 64 E2E e todas as suites Rust, incluindo 94 testes
  de storage e 16 de sandbox, com enforcement Bubblewrap positivo local. Novo CI ainda deverá confirmar
  a migração v7 e os novos testes de lease no Ubuntu/macOS. Launcher geral e tools continuam bloqueados.
- O run `39679546610`, em `04d9969`, passou o CI remoto, confirmando a fatia de quota compartilhada.
- O operador escolheu manifesto explícito para runtime (PD-38). `runtime_manifest` valida arquivos
  individuais, destinos exatos e SHA-256 esperado, sem PATH, `ldd`, download ou expansão de diretórios.
  Linux exige ancestrais/arquivo root-owned, sem escrita por grupo/outros ou symlinks, fixa descritores
  read-only e oferece revalidação de identidade/metadata/conteúdo. Políticas de contagem/bytes são explícitas.
  macOS permanece unsupported para esse validador; isso não altera o staging portátil existente.
- A suíte direcionada passou com 24 testes de sandbox e três doctests, incluindo runtime real sem execução,
  orçamento agregado, digest errado, paths/destinos inválidos, mudanças de bytes/permissões e recusa de
  arquivos especiais/symlinks. O hash derivado de `/usr/bin/true` existe somente no fixture de teste;
  produção precisa de digest aprovado independentemente. Não há launcher ou wiring de tools nesta fatia.
- `TMPDIR=/var/tmp CARAPANA_REQUIRE_BUBBLEWRAP=1 mise exec -- make verify` passou integralmente após
  PD-38: formatter/lint/check/Clippy/build, todas as suites Rust (incluindo 24 testes de sandbox,
  três doctests de privacidade e 94 testes de storage), 24 Vitest e 64 E2E. Enforcement Bubblewrap
  positivo foi exigido no host Linux local; confirmação remota desta fatia ainda pendente.
- O commit `388cfa0` foi enviado; run `39682596254` passou Web/Linux e Rust/Ubuntu/macOS, incluindo
  manifesto explícito e recusa unsupported no macOS. Árvore estava limpa após essa publicação.
- O operador escolheu cgroup v2 previamente delegado (PD-39). `delegated_cgroup` prepara apenas filhos
  vazios de um pai explicitamente configurado, exige cgroupfs v2, domínio e controladores memory/pids,
  aplica limites finitos com swap zero/OOM group e relê controles. Não habilita controladores no pai,
  migra PIDs, inicia ou mata processos. Cleanup remove apenas o filho criado, sem recursão e com identidade.
- Testes direcionados comuns passaram: 30 unitários de sandbox e quatro doctests. O teste positivo de
  criação/readback/remoção exige `CARAPANA_TEST_CGROUP_PARENT`; essa configuração não foi fornecida,
  portanto seu ramo real não foi executado. Não inferir enforcement de recursos desse verde: ainda falta
  prova com delegação real e, separadamente, contenção/saturação de processos antes do launcher geral.
- O gate comum completo com Bubblewrap obrigatório passou após PD-39 (formatter/lint/check/Clippy/build,
  todas as suites Rust, 24 Vitest e 64 E2E). O gate positivo `CARAPANA_REQUIRE_CGROUP=1` sem pai explícito
  foi executado e falhou como esperado, comprovando que a falta da configuração não vira sucesso.
  A fatia de cgroup permanece local, sem commit/push, aguardando pai delegado de teste para validação real.
- O operador informou que ainda não existe pai delegado de teste; não decidiu bloquear o desenvolvimento.
  A atribuição anterior foi incorreta e ele pediu expressamente continuar a implementação do plano.
  A prova real de cgroup é pendência de validação, não impedimento para avançar nas partes independentes.
- Continuação em 22 de outubro: PD-40 adiciona leitura real nonblocking dos dois pipes de um processo já
  supervisionado pelo chamador. A leitura alterna streams em chunks de 4 KiB, aplica o orçamento antes
  de cada read e observa deadline/cancelamento sem threads/filas. EOF é distinto de saída do processo;
  timeout/cancelamento/esgotamento retornam ao supervisor, não matam nem preservam processos sozinhos.
  Testes reais sintéticos cobrem binários, flood maior que a capacidade dos pipes, stderr com stdout
  silencioso, EOF parcial, processo vivo com stdio fechado e descendente que mantém pipes abertos.
  O progresso também está no MCP ai-memory em `progress/delivery-4-continuation-2026-10-22.md`.
- Após PD-40, o gate completo passou com Bubblewrap obrigatório: 44 unitários de daemon, 30 de sandbox,
  restante das suites Rust/doctests, formatter/lint/check/Clippy/build, 24 Vitest e 64 E2E. Os oito testes
  de pipes reais passaram também em três execuções consecutivas. O teste opt-in de cgroup continua sem
  pai configurado; esse limite de validação não é apresentado como enforcement positivo nem pausa do plano.
- Commit `67cb385` enviado; run `39728880317` passou todos os jobs (Web/Linux e Rust/Ubuntu/macOS).
- PD-41 integra captura ao guardian privado existente: `run_captured_owned_process` exige limites e
  timeout explícitos, recusa KeepRunning, transporta até 8 KiB de tails em frame limitado e espera o
  host terminar a limpeza do grupo próprio. EOF não equivale a sucesso; razão de término e exit code
  são separados. Não há handler novo de tool/IPC nem autorização de execução de comandos de modelo.
- Sete testes de processo real cobrem captura binária/exit code/flood, limite agregado com término do
  grupo, timeout após EOF, cancelamento, perda do controller, preservação de processo alheio e recusa
  de spawn em pré-cancelamento/KeepRunning. A prova usa somente fixtures sintéticos, sem provider remoto.
- O gate completo passou após PD-41, incluindo 47 unitários/3 doctests de daemon, sete novos testes CLI
  de captura e todas as suites anteriores, formatter/lint/check/Clippy/build, 24 Vitest e 64 E2E.
  Os sete testes de captura supervisionada passaram também em três execuções consecutivas.
- O run `39729973163`, em `881f2d1`, passou Web/Linux e Rust/Ubuntu/macOS.
- PD-42 liga snapshot, candidato e diff em `WorkspaceEditProposal` e aplica somente token tipado da
  mesma proposta. O controlador exato de PD-34 é compartilhado sem alterar os aliases/API de comandos.
  Aplicação consome o token mesmo em erro; revalida escopo/prazo/revogação e os conflitos físicos do
  snapshot. Não aceita caminho ou bytes diferentes depois da aprovação. Não há wiring de tools/provider.
  Testes reais cobrem candidato exato, solicitação errada, edição externa preservada, escopo/expiração/
  revogação/descarte de controller, workspace físico diferente e candidatos inválidos/oversized; doctests
  recusam token de comando, reuso por movimento e Debug de proposta. Gates de UI/segredos seguem próprios.
- O gate completo de PD-42 passou após uma falha transitória em quatro E2E web sob load médio >100
  no host. Os quatro cenários passaram isoladamente sem mudança de código/config/snapshots e o gate
  integral passou na repetição: 41 unitários de core, 41 de tools, doctests de tipagem/consumo/privacidade,
  demais suites Rust, formatter/lint/check/Clippy/build, 24 Vitest e 64 E2E. Não remover falhas anteriores
  da evidência nem tratá-las como regressão de UI comprovada sem reprodução estável.
- O run `39731329890`, em `5943200`, passou Web/Linux e Rust/Ubuntu, mas os seis fixtures novos de
  edição falharam no macOS com `UnsafePath("/var")`: o leitor de produção recusou corretamente o alias
  do diretório temporário. Regressão com raiz temporária symlink reproduziu a falha no Linux antes da
  correção; somente o fixture agora canonicaliza sua raiz antes de capturar o workspace. Nenhuma regra
  de produção de symlink/confinamento foi relaxada. A pergunta sobre unidade transitória cgroup foi
  interrompida pelo reinício e ainda não recebeu autorização; nenhuma unidade foi criada.
- Após novo pedido explícito, o operador autorizou a unidade transitória exclusiva de teste. O fixture
  `scripts/test-delegated-cgroup.sh` foi implementado e executado sem sudo ou configuração persistente;
  passou a prova real de criação/readback/remoção e de enforcement de memória/PIDs em cgroup v2.
  Workloads sintéticos aguardam gate byte antes de fork/alocação; verificações exigem eventos reais
  `pids.events:max` e `memory.events:oom_kill`. Cleanup removeu os grupos e nenhuma unidade própria
  ficou carregada. O produto não passou a depender de systemd nem recebeu descoberta/config automática.
- A repetição encontrou uma lacuna do fixture de memória: `bytearray` pode reservar páginas zero
  virtualmente sem materializá-las, então a tentativa não garantiu OOM. O fixture agora escreve em cada
  página (passo de 4 KiB) após o gate de admissão; a campanha completa passou três execuções isoladas
  consecutivas, incluindo os eventos de pids e OOM. Unidades transitórias próprias foram coletadas.
- O CI `39736753908`, em `06d8b99`, confirmou a correção de fixtures em Rust/macOS e passou também
  Rust/Ubuntu e Web/Linux. Após a prova real cgroup, o gate comum integral passou com 31 testes de
  sandbox, 42 tools, 41 core, 47 daemon e demais suites/doctests, format/lint/check/Clippy/build,
  24 Vitest e 64 E2E. A prova privilegiada não existe: todas as campanhas usaram usuário normal.
- PD-43 adiciona admissão única via `Child` próprio e término por descritor de cgroup, sem API de PID
  arbitrário. Revalida grupo/limites, confirma membership e conserva cleanup em resultado incerto.
  O fixture real passou usando essa API para PIDs/memória e demonstrando término de descendente após
  `setsid`; segunda admissão foi recusada. O chamador ainda deve estabelecer gate confiável antes do
  exec e recolher o filho; a integração específica ao guardian não é presumida por esta API.
- A API de PD-43 passou três campanhas reais consecutivas na unidade autorizada: nove testes de
  cgroup, incluindo fork/OOM, descendente após setsid, recusa de segunda admissão/filho encerrado e
  preservação de população inesperada sem ownership da lease. Nenhuma unidade própria ficou carregada.
  O gate comum completo também passou (33 sandbox e demais suites Rust/doctests, format/lint/check/
  Clippy/build, 24 Vitest e 64 E2E). CI `39777790977` do commit anterior `e97b7f5` passou todos os jobs.
- CI `39779051800`, em `baf16b0`, passou Web/Linux e Rust/Ubuntu/macOS. PD-44 integra admissão ao
  guardian privado antes de exec: Ready → migração verificada → Admit → membership real → início.
  Kill switch fixado em fd reservado do guardian permite cleanup mesmo com EOF do controller e
  descendente após setsid; o target recebe stderr em pipe, sem esse fd. Sem fallback fora do cgroup.
- Três campanhas reais autorizadas passaram nove testes de cgroup mais seis CLI de guardian com
  delegação: membership, setsid, perda de controller, Admit falso, reuse e OOM sem fallback. O teste comum
  de pre-exec gate também passou (oito CLI de captura no total). Filesystem/rede e aprovação completa
  ainda devem ser ligados ao launcher; esta API interna não habilita tools nem promete confinement total.
- A primeira tentativa do gate completo de PD-44 parou no startup dos servidores E2E (60 s), com load
  médio do host >100 e sem listeners residuais em 4173/4174 após cleanup. O gate integral passou na
  repetição, sem alterar timeouts/config/snapshots. A revisão posterior adicionou uma regressão no
  caminho de rejeição: stderr reservado não pode receber logging do CLI privado. O teste reproduziu
  saída por panic (101) antes de remover essa impressão; depois, oito testes comuns de captura e três
  campanhas reais (nove cgroup + seis guardian) passaram, incluindo saída de erro normal (1). Gate final
  após essa correção passou integralmente: formatter/lint/check/Clippy/build, todas as suites Rust e
  doctests, 24 Vitest e 64 E2E, com Bubblewrap obrigatório. Não houve alteração de timeouts/snapshots.
- CI `39784061018` do HEAD `eab8f0a` confirmado verde em Web/Linux e Rust/Ubuntu/macOS.
  Continuação PD-45 fecha a mutabilidade dos bytes de runtime entre hash e futuro mount: manifesto
  verificado é consumido em cópias memfd seladas, com budgets explícitos, leitura posicional limitada,
  identidade antes/depois e SHA-256 da cópia já selada. Descritores continuam CLOEXEC. Não há descoberta
  de dependências, mounts ou execução nova; transporte ao guardian e admissão global de memória para
  preparação continuam pendências próprias, não garantias desta imagem por si só.
- `133e309` publicou PD-45; CI `39785625298` passou Web/Linux e Rust/Ubuntu/macOS.
  PD-46 adiciona transporte SCM_RIGHTS limitado a socketpair privado: recebe CLOEXEC atômico,
  verifica seals/tamanho/hash contra manifesto esperado e recusa truncamento/quantidade incorreta.
  Testes reais cobrem transferência entre processos, integridade, prazos e fechamento dos direitos
  recebidos em erro (EOF no peer confirma ausência de descritores vazados). Sem unsafe, listener,
  paths de origem ou herança implícita de fds. Ainda não está ligado ao protocolo do guardian/mounts.
- `68a09d3` publicou PD-46; CI `39787157184` confirmado verde em Web/Linux e Rust/Ubuntu/macOS.
  PD-47 compõe execução real no helper privado: recebe imagem selada, fixa Bubblewrap por descritor,
  mapeia somente os arquivos individuais e fecha o canal antes do target. Root/workspace vazio readonly,
  scratch explícito, namespaces separados, caps zero e NoNewPrivs foram exercitados com fixtures locais.
- Cinco testes CLI reais passam: arquivos do host/cgroupfs/execs não declarados ausentes, ambiente
  sintético não herdado, namespace de rede distinto, stdin /dev/null, fds de runtime fechados e setup
  malformado recusado sem log de payload. Primeiro fixture falhou Permission denied porque o loader ELF
  estava classificado como suporte 0444; agora é explicitamente Executable no manifesto de teste,
  sem ampliar mounts. Testar sandbox isolado revelou feature `stdio` de rustix antes suprida por
  unificação do workspace; declarada explicitamente para build independente do crate.
  Guardian/cgroup, staging reservado, aprovação exata e admissão global continuam etapas próprias.
- `037062e` publicou PD-47; CI `39790206622` passou. PD-48 liga o helper ao guardian admitido,
  reutilizando captura/cleanup e enviando o socket privado do helper por SCM_RIGHTS somente após
  confirmar membership. Controle é lido sem buffering para não descartar direitos em read-ahead.
- Três campanhas reais na unidade transitória autorizada passaram nove testes backend cgroup,
  seis CLI resource-scoped e sete image_launcher (incluindo dois de composição com delegação).
  Cancelamento exige observar o argv sintético único do target já exec'd nos grupos próprios da
  unidade; não aceita cancelar só o helper no startup como prova de encerramento do target. Todos os
  grupos foram esvaziados/removidos e nenhuma unidade própria ficou carregada. Três testes unitários
  de canal cobrem tipo do fd, CLOEXEC, deadline e ausência de direitos. Staging/aprovação/global memory
  continuam pendentes; não houve ativação de tools, provider, merge ou deploy.
- Revisão de PD-48 encontrou que `dup` do controle não bufferizado não mantém CLOEXEC. Regressão nova
  reproduziu herança de fd privado no target antes da correção; agora a duplicação usa
  `fcntl_dupfd_cloexec` atômico no guardian e helper. Após a correção, nove testes comuns de captura,
  três campanhas reais (nove backend + seis resource-scoped + sete image_launcher) e o gate integral
  passaram novamente: format/lint/check/Clippy/build, suites Rust/doctests, 24 Vitest e 64 E2E.
  Nenhuma unidade própria permaneceu carregada, sem mudanças de snapshots/timeouts.
- CI `39792509025` em `63bbbe5` confirmado verde em Web/Linux e Rust/Ubuntu/macOS. PD-49 liga
  `ReservedSelectedSnapshotBundle` ao launch como imagem de arquivos selados sob `/workspace`,
  com lease de quota retida, identidade física da fonte e nenhum mount de diretório de staging.
  Helper valida destinos/dados readonly e recebe seleções de até 256 arquivos em lotes <=128 fds,
  preservando teto separado de runtime, 16 MiB agregados e limite por arquivo. Setup excessivo é
  recusado antes de revisão/efeito, sem expansão automática do frame de 128 KiB.
- Testes adicionados verificam seleção exclusiva e bytes antigos preservados após edição da fonte,
  quota retida até descarte do launch, staging alterado recusado antes de envio, rejeição de escapes/
  roles executáveis/conflitos/budgets e lote incompleto sem imagem parcial. A prova positiva de
  seleção máxima usa o guardian/cgroup real, sem copy-back ou ativação de tools.
- Três campanhas reais passaram nove backend cgroup + seis CLI resource-scoped + dez image_launcher,
  com seleção máxima de 256 arquivos efetivamente executada. O gate integral final passou: format/
  lint/check/Clippy/build, suites Rust/doctests (50 sandbox), 24 Vitest e 64 E2E. A primeira tentativa
  parou em quatro empréstimos desnecessários nos fixtures; corrigidos sem suppressions. Nenhuma
  unidade própria ficou carregada, sem alterar snapshots/timeouts.
- `272ad85` publicou PD-49; CI `39795317999` passou Web/Linux e Rust/Ubuntu/macOS.
  PD-50 adiciona `SandboxExecutionProposal` imutável e efeito `run_approved_image_process`, que consome
  token concreto e vincula IDs/identidade física, runtime, snapshots reservados, argv/scratch e budgets
  de captura/memória/PIDs. Sem argumentos ou arquivos substituídos depois da revisão. Relógio/escopo
  pertencem ao controlador confiável; não são input de modelo ou autenticação implícita de UI.
- Três campanhas reais passaram nove backend cgroup + seis CLI resource-scoped + quinze image_launcher.
  Provas novas: limite revisado exato de oito bytes, request errado recusado, oito rejeições sem admitir
  guardian (expiração/revogação/drop/IDs/workspace físico/limites/staging), expiração após Ready antes da
  admissão e revogação após observar target exec'd com cleanup. Compile-fail impede Debug da proposta,
  token de comando no efeito e reuso por movimento. Cleanup incerto conserva erro próprio, sem ser
  mascarado por erro de aprovação; nenhuma unidade própria ficou carregada.
- Gate integral após PD-50 passou: format/lint/check/Clippy/build, todas as suites Rust/doctests,
  24 Vitest e 64 E2E. Relógio após Ready foi avançado deterministicamente no fixture e o grupo
  permaneceu pronto/não admitido, sem sleeps usados como prova de expiração. Nenhum snapshot/timeout
  foi alterado; admissão global de preparação, conteúdo/segredos e origem autenticada de UI seguem próprios.
- PD-51 acrescenta ledger separado de bytes nominais/slots de preparação no catálogo único do usuário,
  em schema v8. Reserva/contagem são atômicas entre conexões; não há default final de RAM nem troca
  silenciosa da política compartilhada. Drop/crash conservam a cobrança/quarentena, sem TTL de liberação.
  Release explícito ainda exige prova independente do controlador de que buffers/consumidores terminaram;
  o ledger isolado não mede RSS, não é fila durável de jobs e ainda não intercepta as alocações do launcher.
- Nove testes novos usam SQLite/filesystem reais: concorrência com barrier, bytes/slots, política,
  abandono/reabertura/idade, trigger de falha no release, exit de subprocesso sem destructors, migração
  v7 preservando sessão/eventos/artifact e staging ativo, sequência de IDs e separação da quota de disco.
- Os nove testes do ledger passaram três repetições e o gate integral passou (103 storage, demais
  suites Rust/doctests, format/lint/check/Clippy/build, 24 Vitest e 64 E2E). A admissão antes das
  alocações e a prova de liberação de consumidores ainda não são inferidas desse verde de infraestrutura.
- PD-52 liga a reserva ao preparo: `AdmittedSandboxProposal` admite capacidade antes de verificar,
  copiar e selar runtime/snapshots. A proposta retém payloads e lease durante review/execução; somente
  seu token concreto entra em `run_admitted_image_process`, sem expor a proposta base ou descritores.
  Falha de construção ou descarte pronto fecha payloads antes de liberar capacidade; execução/panic
  ou erro de processo desconhecido mantém a cobrança em quarentena. Erro de aprovação só permite
  retorno a pronto quando o fluxo supervisionado confirmou cleanup, sem mascarar falha de cleanup.
- Quatro testes de admissão verificam rejeição antes de fontes inválidas, falhas de construção/Drop
  pronto, retenção durante review/execução até fechar payloads e quarentena após erro do host sintético.
  Três campanhas reais consecutivas de backend/captura/image launcher passaram, com 19 testes de
  image launcher. Regressão de descritores mantém ArtifactStore e PreparationAdmissionStore abertos
  no controlador e verifica que o alvo não herda seus descritores nem os controles privados.
- A conta é nominal, não medição de RSS/pressão ou limite total da máquina. Recovery de órfãos,
  defaults calibrados, fairness, UI autenticada, conteúdo/segredos e Git/worktree seguem pendentes;
  `ToolCallRequested` permanece inerte. CI de PD-50 e PD-51 confirmado verde nos três jobs.
- Provas adicionais do lifecycle admitido passaram em três campanhas reais: revogação após observar
  o alvo executando conserva a reserva até cleanup confirmado e descarte dos payloads; panic injetado
  no relógio antes de criar o guardian conserva quarentena, mesmo com grupo ainda pronto. São 21 testes
  image launcher. A prova de panic não simula um panic durante execução nem uma falha de cleanup kernel.
- CI `39808928031` de `6d7904f` passou Web/Linux e Rust/Ubuntu/macOS. Ao repetir o gate local em 24/10,
  a prova SCM_RIGHTS encontrou uma corrida no próprio teste: EOF imediato concorre com fork/exec de
  outros testes, pois CLOEXEC não fecha descritores no intervalo anterior ao exec. Falha reproduzida
  na suíte paralela; 100 runs isolados passaram e a instrumentação observou EOF posterior. A prova
  agora cria seus sockets somente após re-exec isolado do teste exato, mantendo EOF imediato e prazo
  bounded/reaping do filho. Trinta repetições da suíte sandbox completa passaram sem alterar protocolo.
- PD-53 adiciona política explícita de saúde do host Linux ao objeto admitido: leitura bounded de
  `MemAvailable` e PSI `some/full avg10`, sem fallback para RAM total/pressão zero, amostras do modelo
  ou defaults finais. Verifica antes das imagens, ao entrar no efeito e novamente nas fronteiras
  `BeforeEffect`, inclusive antes de transmitir descritores; cleanup incerto continua prioritário.
  Rejeição antes das imagens libera a reserva nominal sem tocar runtime/seleção inválidos.
  O gate não mede RSS nem monitora pressão continuamente; recovery de órfãos, fairness e calibração
  por máquina continuam pendentes. A hierarquia herdada é coberta separadamente pela PD-54 abaixo.
- Validação local de PD-53 em 24/10: cinco testes de parser/política e uma regressão CLI de rejeição
  antes de fontes inválidas passaram com sinais reais do kernel/SQLite/filesystem. O gate integral
  comum com Bubblewrap obrigatório e portas E2E 34173/34174 passou: 27 Vitest, 66 E2E, suites Rust,
  doctests, format/lint/check/Clippy/build. Testes que exigem cgroup opt-in não executam o corpo neste
  gate. A tentativa inicial da campanha real falhou antes de iniciar: `systemd --user` estava
  indisponível após OOM; investigação confirmou `user@1000.service=failed`, `MainPID=0`, `Result=signal`,
  `ExecMainStatus=9`. Nenhum serviço/configuração persistente foi alterado para contornar a ausência.
- O CI `40109502711` de `48f222d` passou Rust nas duas plataformas, mas Web encontrou interação
  anterior à hidratação em `Should keep the execution unchanged and snapshot the queued profile`.
  Artifact/DOM/trace preservados e revisados; regressão com scripts atrasados falhou antes da correção
  em desktop/mobile. `f02c475` mantém a aplicação inerte até mount, informa ausência de JavaScript e
  sincroniza o helper de teste nesse marco. Vinte execuções dos dois cenários afetados e o gate
  integral comum passaram, sem snapshots/timeouts alterados.
- CI `40110323707` de `f02c475` confirmado verde: Web/Linux e Rust/Ubuntu/macOS.
- PD-54 complementa PD-53 com a hierarquia visível de memória do controlador: descritores no
  cgroup2 padrão, no-follow, membership limitado e revalidado, margem mínima de `memory.max/high`
  menos `memory.current` e maior PSI entre ancestrais. Subárvore apresentada como raiz é rejeitada
  quando expõe `memory.current`, em vez de assumir ancestrais sem limites. Sem modificar controles.
- Dez testes de saúde do host/hierarquia passaram três repetições: parser, políticas, limites
  herdados, `max`, saturação, PSI, caminhos, symlink, filesystem incorreto e leitura real do próprio
  cgroup. Gate comum integral passou com 27 Vitest, 66 E2E e todas as suites Rust/doctests/builds.
- Em seguida, após recuperação do gerenciador, `TMPDIR=/var/tmp bash scripts/test-delegated-cgroup.sh`
  passou na unidade transitória delegada: `carapana-sandbox delegated_cgroup` (9/9),
  `carapana-cli --test captured_cgroup` (6/6) e `carapana-cli --test image_launcher` (22/22), incluindo
  limites reais de memória/PIDs, cleanup e lifecycle. A mensagem de panic em um teste é injetada
  deliberadamente e o próprio teste passou. Não houve alteração persistente do host.
- PD-53/54 foram publicadas em `01467e0`; CI `40114438822` passou Web/Linux e Rust/Ubuntu/macOS.

### Progresso do Delivery 4 — 25 de outubro de 2026

- PD-55 adiciona reamostragem de memória ao loop de execução admitida: política imutável de
  `MemAvailable`/PSI e hierarquia herdada, primeira leitura imediata, cadência monotônica de 100 ms.
  Durante execução verifica margem remanescente, sem exigir novamente o teto inteiro já admitido.
  Rejeição encerra o grupo; erro de cleanup/reap tem precedência e conserva quarentena.
- Prova real em controlador-filho de subgrupo descartável: após observar o alvo executando,
  reduz somente `memory.high` desse fixture (sem produzir OOM/pressão global), confirma erro
  `InsufficientAvailable`, grupo vazio e reserva retida até descarte dos payloads. A regressão falhou
  sem a checagem em execução e passou com ela. Dois testes unitários cobrem cadência e erros de leitura.
- O observador de exec dos testes podia omitir grupos ao examinar só as primeiras 64 entradas do
  pai; instrumentação observou 68 entradas entre controles e fixtures concorrentes. O orçamento agora
  é explícito de 1024 entradas e excesso falha, sem prefixo silencioso que esconda alvo vivo.
- Três campanhas transitórias consecutivas passaram: 9 testes backend cgroup, 6 captured cgroup e
  23 image launcher por campanha. Gate integral local passou com Bubblewrap obrigatório: 27 Vitest,
  66 E2E, suites Rust/doctests, format/lint/check/Clippy e builds. Sem alterações em unidades alheias.
- PD-55 permanece local. A amostragem não é limite total de RSS nem garantia contra OOM entre leituras;
  recovery de órfãos, fairness, calibração por máquina e gates de conteúdo/segredos/UI autenticada
  continuam separados. Tools/`ToolCallRequested` continuam inertes; sem inferência/contexto reais.
- PD-56 adiciona política explícita de bytes/leases por workspace físico, além do teto global,
  na mesma transação SQLite. Device/inode é a chave, não sessão, caminho ou identidade do modelo;
  o preparo revalida a fonte e reserva por essa chave antes de ler runtime/snapshots.
- SQLite v9 preserva leases v8 sem atribuição: contam globalmente e em cada workspace. Drop/crash,
  quarentena, tempo arbitrário e falhas de liberação não zeram a cobrança. Conexões com políticas
  diferentes falham e reserva sem identidade não pode contornar um teto local mais restritivo.
- Treze testes de admissão em storage passaram três repetições, incluindo concorrência real,
  limites por bytes/slots, quarentena, migração v8 com sessões/artifacts preservados e v7. A integração
  CLI prova dois preparos em workspaces distintos e rejeição de outro preparo no mesmo diretório
  antes de tocar fontes inválidas. Três campanhas transitórias adicionais passaram com 9 backend,
  6 captured cgroup e 24 image launcher. Gate integral local novamente verde: 27 Vitest, 66 E2E,
  suites Rust/doctests, Clippy e builds, com Bubblewrap obrigatório. PD-55/56 permanecem locais.
- Tetos por workspace são isolamento de capacidade, não scheduler justo nem garantia de ausência
  de starvation. Não foram escolhidos defaults finais nem liberadas leases órfãs por PID/TTL.
- PD-57 adiciona recovery explícito e limitado à fase anterior à exportação: arquivo de controle
  privado com identidade persistida e lock exclusivo acompanha a lease após os payloads na ordem
  de descarte. O preparo só aloca após confirmar esse registro. Antes de iniciar guardian/exportar,
  grava barreira irreversível; falha ou commit incerto impede o efeito.
- SQLite v10 acrescenta somente a proveniência de novos preparos recuperáveis. Leases v8/v9,
  raw e já exportadas não recebem prova retrospectiva e continuam cobradas. Recovery bounded em
  transação exige controle exato, íntegro e lock livre; não libera por idade, PID ou reinício.
  Reconciliadores concorrentes não recuperam duas vezes; erros de delete mantêm a reserva e o controle.
- Provas incluem saída real sem destrutores depois de construir memfds/staging (antes de exportar),
  lock ocupado, barreira de exportação, falha de gravação, controles ausentes/substituídos, symlink,
  hardlink, permissões/conteúdo inseguros e migração v9. A integração confirma que a recuperação de
  memória não apaga a reserva de disco do staging; panic após a barreira permanece em quarentena.
- Três repetições dos 22 testes de admissão em storage e três campanhas transitórias passaram,
  cada campanha com 9 backend cgroup, 6 captured cgroup e 25 image launcher. Gate integral local
  passou: 27 Vitest, 66 E2E, suites Rust/doctests, Clippy e builds, com Bubblewrap obrigatório.
  PD-55/56/57 continuam locais, sem commit/push; nenhum serviço persistente foi alterado.
- Recovery pós-exportação ainda não é implementado: lock livre do controlador não comprova fim de
  consumidores. Fair scheduling/calibração, conteúdo/segredos, origem UI autenticada e Git/worktree
  operacional continuam gates distintos; `ToolCallRequested`, inferência e contexto reais seguem inertes.
- PD-58 estende a recuperação somente a execuções cujo término dos consumidores foi comprovado:
  depois de cleanup/reap bem-sucedidos, persiste confirmação antes de Ready. Próxima execução invalida
  a confirmação antes do efeito; erros de processo/panic nunca a produzem. Recuperação de concluídos
  é explícita e separada da pré-exportação, exigindo também ausência do lock do dono dos payloads.
- Integração real executa uma imagem aprovada, comprova grupo vazio/removido e encerra o dono sem
  destrutores ainda retendo imagens; recovery de concluídos libera a reserva. O caso com panic após
  barreira não é recuperado por nenhuma das duas APIs. Storage comprova lock vivo, prova ausente e
  invalidação da prova anterior numa nova execução. Crash durante execução continua sem recovery.
- Após PD-58, três repetições dos 24 testes de admissão em storage e três campanhas reais passaram:
  9 backend cgroup, 6 captured cgroup, 26 image launcher por campanha. Gate integral passou novamente
  com 27 Vitest, 66 E2E e suites Rust/doctests/Clippy/builds. PD-55–58 permanecem locais, sem publicação.
- O operador aprovou a fila justa explícita em vez de manter recusa imediata (PD-59). SQLite v11
  persiste intenções bounded de capacidade por workspace físico, sem payloads/comandos/aprovações.
  A rotação persistida alterna workspaces elegíveis e preserva FIFO interno; novos workspaces entram
  no fim. Workspace sem capacidade não bloqueia outros elegíveis nem perde sua posição na rotação.
- Configuração da fila impede contorno pela admissão imediata. Claim explícito confere ticket,
  workspace e orçamento recalculado, concede lease e remove intenção na mesma transação. O preparo
  enfileirado reusa os gates de memória, snapshots e a aprovação exata posterior, sem execução
  automática. Cancelamento/expiração afetam só espera, não leases já admitidas.
- Três repetições dos 32 testes de admissão e três campanhas reais passaram: 9 backend, 6 captured
  cgroup e 27 image launcher por campanha. Cobertura inclui FIFO/rotação, newcomers, saturação local,
  espera total/por workspace, restore de metadados, expiração, cancelamento, replay, claim concorrente,
  rollback, migração v10 e preparo integrado. A primeira rodada do gate integral após PD-59 atingiu
  o timeout de 120 s da ferramenta durante E2E, após as campanhas; não é registrada como aprovação.
  A repetição independente passou com portas 36173/36174: 27 Vitest, 66 E2E, 126 testes de storage,
  demais suites Rust/doctests, Clippy e builds. PD-55–59 continuam locais, sem commit/push.
- PD-60 registra a intenção gerenciada na própria reserva (SQLite v12), cobrindo a janela antes
  de existir controle de posse. Recovery pode vencer essa fase sem payloads; a tentativa concorrente
  de registrar posse falha por FK e não pode iniciar alocação. Se o registro vencer, permanecem
  obrigatórios o lock e as provas anteriores. Reservas raw/legadas não são reclassificadas.
- Testes novos cobrem o reconciliador vencendo um dono ainda vivo antes do registro, falha de registro
  sem liberar raw leases e migração v11 conservadora. Controles órfãos vazios não são apagados sem
  proveniência: criação enumera com orçamento finito de 4096 entradas e recusa excesso, evitando
  crescimento irrestrito de metadata após crashes repetidos. Isso não é quota global de disco.
- Após PD-60, os 36 testes de admissão em storage passaram três repetições. Três campanhas reais
  passaram, com 9 backend cgroup, 6 captured cgroup e 27 image launcher por campanha. O gate inicial parou por
  conflito transitório na porta dev 36174 antes de E2E. Prova isolada observou somente um listener
  deste checkout e passou 66 E2E; não reproduziu a causa do conflito e não alterou o harness.
  `make verify-rust` passou separadamente; em seguida o gate canônico integral passou com portas
  36173/36174, 27 Vitest, 66 E2E, 130 testes de storage e suites Rust/doctests/Clippy/builds.
  Logs finais: `/var/tmp/carapanan-verify-intent-canonical-final-20261025.log` e campanhas
  `/var/tmp/carapanan-intent-recovery-cgroup-{1,2,3}-20261025.log`. PD-55–60 continuam locais.
- O operador autorizou `tirion-tailscale` para diagnóstico/testes sintéticos; Mac será validado
  manualmente pelo operador, sem SSH do agente. Preflight somente leitura confirmou i5-8265U/16 GB,
  user manager acessível, dependências instaladas e pressão baixa no momento. Sem instalar software.
- PD-61 acrescenta probe release do controle SQLite de admissão/fila, runner de saída bounded/schema
  fechado, timeout/reap e campanha reproduzível. Um warmup e três amostras serializadas em Ryzen e
  Tirion, com o mesmo SHA-256 de binário Rust 1.98.1, terminaram sem tickets ou reservas restantes.
  Resumo revisado em `docs/benchmarks/admission-control-plane-2026-10-25.json`; instruções do Mac em
  `docs/benchmarks/admission-resource-probe.md`. Unidades transitórias de 256 MiB/32 tarefas/CPU 100%
  e 60 s, swap da unidade zero; sem instalações ou alterações persistentes. Artefatos remotos removidos.
- A revisão detectou dois vieses antes de concluir o baseline: `ru_maxrss` podia incluir o wrapper
  Python pré-exec, e shims/override global usavam Rust 1.99.1 apesar do pin 1.98.1. O probe Linux agora
  reporta seu próprio VmHWM (mediana ~5,2 MiB nesse componente apenas); o runner identifica o dado
  pré-exec como tal. Gate e campanha usam helper `rustup run` com o pin, sem instalar ou mudar defaults.
  Seis testes Python cobrem schema, ambiente limpo, output excessivo, timeout/reap e override global.
- Os tempos observados não ranqueiam capacidade dos hosts: fsync/contenção variam. O probe não mede
  sessões, parsers, snapshots, agentes ou processos externos e não escolhe defaults finais. Validação
  completa posterior ao ajuste de toolchain/probe ainda precisa ser confirmada; resultados anteriores
  do gate não são apresentados como validação com o pin. PD-55–61 seguem locais, sem commit/push.
- O operador corrigiu explicitamente o toolchain-alvo para **1.99.1**. `rust-toolchain.toml` e README
  foram alinhados, preservando o helper que impede overrides silenciosos. O gate e três campanhas
  concluídos em 1.98.1 são evidência histórica, não substituem nova validação em 1.99.1. Baseline de
  1.98.1 será mantido identificado; repetir medidas com o pin solicitado antes de atualizar o resumo.
- Probe reconstruído com Rust 1.99.1 e repetido em Ryzen/Tirion: mesmo binário
  `70b05cd45124d6b9dfa4a0a089a053565c7fb706c2bd3dd3923f527b86ddfd58`, um warmup e três amostras
  por máquina, todas sem reservas/tickets restantes. O resumo ativo usa 1.99.1 e o baseline 1.98.1 foi
  separado como histórico. Mac continua manual; scripts e README seguem o pin 1.99.1 confirmado.
- Três campanhas reais e três repetições dos testes de admissão passaram com 1.99.1. O gate integral
  encontrou regressão no teste de `make dev`: o novo wrapper `rustup run` ultrapassava o cargo falso
  do fixture e iniciava a TUI real sem terminal. Corrigida a prova de orquestração para interceptar
  rustup, exigir o pin exato e encaminhar ao cargo controlado; o teste Python separado continua
  verificando o rustc real com override global inválido. Fixture agora usa diretório exclusivo 0700.
  Três repetições do launcher e seis testes Python passaram. A repetição do gate integral final em
  **1.99.1 passou**: 27 Vitest, 66 E2E, seis testes Python, 130 testes de storage e todas as suites
  Rust/doctests/Clippy/builds. Log: `/var/tmp/carapanan-verify-rust-1.99.1-launcher-final-20261025.log`.
  Nenhuma assertion de shutdown/exit code ou timeout foi removida. O operador autorizou commit/push
  desse lote após o gate, mantendo PR sem merge/deploy e ferramentas reais inertes.

---

# 159. Delivery 5 — Context Engine

Implementar:

- context layers;
- token accounting;
- AGENTS.md;
- working set;
- retrieval;
- compaction;
- provider adaptation;
- archived summaries.

Compactação automática visível não apaga histórico; preservar objetivo/plano/autorizações/
trabalho pendente. Pausar quando não houver preservação segura. Integração real ai-memory
fica no Delivery 9, sem dependência externa obrigatória para contexto local.

---

# 160. Delivery 6 — Auto Mode, Policy e Sandbox

Implementar:

- deterministic rules;
- context-aware rules;
- Intent Ledger;
- trust boundaries;
- approval engine;
- Jev adapter;
- LLM classifier;
- deny-and-continue;
- sandbox;
- secret protection;
- prompt-injection boundaries.

Este delivery amplia policy/Auto/sandbox sobre o enforcement mínimo do Delivery 4.
Não adiar proteção de segredos/privilégios/arquivos até classifiers ou skills.
Aplicar regras de Yolo, limites de diretório e permissões persistentes de PD-07/PD-08.

---

# 161. Delivery 7 — Built-in Engineering Skills

Implementar:

- security;
- maintainability;
- testing;
- local-development;
- frontend-design;
- git;
- rust.

Adicionar:

- skill matcher;
- scopes;
- project/user skills.

---

# 162. Delivery 8 — Validation & Completion

Implementar:

- completion gate;
- evidence;
- build/test discovery;
- smoke checks;
- doom-loop detection;
- duplicate detection;
- TODO/stub checks;
- transparent "not validated".

Validação incompleta pausa fila. Doom loops usam defaults 3 tentativas → diagnóstico/
mudança de estratégia, mais 3 sem progresso → pausa, inclusive em Yolo. Só evidência
real de avanço reinicia contador; não confundir com retries temporários do provider.

---

# 163. Delivery 9 — MCP

Implementar:

- stdio;
- Streamable HTTP;
- MCP registry;
- hot reload;
- health;
- scopes;
- pinned MCP;
- lazy MCP;
- permissions;
- logs;
- imports.

Validar especificamente MCP de memória persistente como uso prioritário.

Validar ai-memory e hooks antes de compactação/no encerramento, leitura relevante no
início/retomada, escopo de gravação autorizado e degradação segura conforme PD-12.
Atualizações de tools que ampliem acesso exigem approval; hooks seguem todas as proteções.

Com os gates dos Deliveries 1–9 satisfeitos, realizar as três jornadas de primeiro
dogfooding de PD-02, no macOS/Linux com a assinatura ChatGPT. Web/mobile reais e
providers/subagents adicionais não bloqueiam esse marco.

---

# 164. Delivery 10 — Remote Attach & Multi-device

Implementar:

- remote daemon;
- device pairing;
- device keys;
- permissions;
- snapshot/reconnect;
- remote CLI;
- remote TUI;
- Tailscale/NetBird interface selection.

Pareamento aplica defaults restritos do celular, fila compartilhada sem device dono,
origem de mensagens e bloqueio de ações offline até snapshot atualizado, conforme PD-13.

---

# 165. Delivery 11 — Web/PWA Production Connection

Pegar a interface aprovada no Delivery 0 e conectá-la ao runtime real.

Implementar:

- real session dashboard;
- live events;
- approvals;
- diff;
- plan;
- validation;
- settings;
- MCP;
- resources;
- pairing;
- mobile UX.

### Evolução posterior — notificações com a PWA fechada

Investigar suporte do sistema, infraestrutura e segurança em etapa posterior à
conexão inicial. UX inicial usa attention queue e avisos com a PWA aberta. Não prometer
push sem comprovar viabilidade, nem introduzir relay/cloud obrigatório como atalho.

---

# 166. Delivery 12 — Provider Expansion

Prioridade:

```text
OpenAI API
Anthropic
OpenRouter
OpenAI-compatible
Gemini
```

Adicionar apenas quando cada adapter aproveitar suas capacidades reais.

---

# 167. Delivery 13 — Subagents

Implementar:

- bounded agents;
- explorer;
- tester;
- reviewer;
- budgets;
- permission isolation;
- context isolation.

Delegação segue o perfil e os budgets conforme PD-15: Perguntar pede approval;
Auto/Yolo podem delegar; Planejar só investiga. Sem ampliar privilégios ou impor worktree.
Sem capacidade, continuar sozinho ou aguardar, sem aumentar limites automaticamente.

Role routing opcional:

```text
planner
coder
reviewer
classifier
```

pode usar modelos diferentes.

---

# 168. Delivery 14 — Hardening

- fuzzing;
- adversarial tests;
- resource soak tests;
- long-session tests;
- crash tests;
- network disruption;
- MCP failure;
- provider failure;
- SSD write profiling;
- RAM profiling.

---

# 169. Delivery 15 — Public OSS Release

Antes:

- docs;
- installation;
- security policy;
- threat model;
- contributing;
- code of conduct se público;
- release signing;
- SBOM;
- issue template;
- doctor report;
- compatibility matrix.

Distribuir sob Apache-2.0, incluindo licença/notices aplicáveis e artefatos prontos
sem toolchain Rust/Node obrigatório para usuário final.

---

# 170. Resource release gates

Nenhuma release estável se:

- sessão idle retiver contexto completo;
- RAM crescer indefinidamente;
- tool output ilimitado entrar na memória;
- caches não tiverem limite;
- WAL não possuir checkpoint;
- child processes ficarem órfãos;
- repo grande causar scan cego;
- daemon consumir armazenamento sem teto.

---

# 171. UX release gates

Nenhuma release estável se:

- cliente novo não descobrir sessões já abertas;
- mobile iniciar em tela de chat vazia;
- attention queue não existir;
- approvals forem genéricos;
- configurações comuns exigirem TOML;
- MCP exigir daemon restart;
- completion não mostrar validação;
- diff for inutilizável no celular;
- estado do agent não for imediatamente identificável.

---

# 172. Security release gates

Nenhuma release estável se:

- `.env` puder ser lido automaticamente;
- secrets aparecerem em logs;
- tool output puder alterar policy;
- MCP recém-instalado ganhar acesso irrestrito;
- provider switch puder vazar secret;
- sandbox for reportado como ativo sem enforcement;
- remote daemon bindar publicamente por padrão.

---

# 173. Dogfooding

Carapanã deverá desenvolver Carapanã.

Critérios:

- tasks reais feitas via Carapanã;
- PRs reais;
- tests;
- UI work;
- refactors;
- debugging;
- multi-device;
- mobile approvals;
- MCP memory;
- provider switching.

Uma feature de coding-agent que não sobreviver ao desenvolvimento do próprio harness ainda não está pronta.

---

# 174. Métricas técnicas internas

Acompanhar:

```text
daemon idle RSS
active session RSS
idle session incremental RSS
disk writes/minute
SQLite WAL size
artifact growth
tool output retained
process count
MCP memory
session restore latency
web reconnect latency
context compaction latency
```

---

# 175. Métricas de UX

Dogfood:

```text
tempo até iniciar primeira tarefa
intervenções manuais desnecessárias
approvals por tarefa
false-positive security blocks
false-negative dangerous actions
retries por tool
doom-loop incidents
tarefas finalizadas sem validação
tempo para continuar sessão no celular
```

---

# 176. ADRs obrigatórios

Criar Architecture Decision Records para decisões de alto impacto:

```text
ADR-001 Rust runtime
ADR-002 daemon/client separation
ADR-003 SQLite + artifacts
ADR-004 event-based session persistence
ADR-005 provider capability model
ADR-006 policy/decision separation
ADR-007 secret handling
ADR-008 multi-device protocol
ADR-009 resource budgets
ADR-010 MCP lifecycle
```

Decisão aprovada para SQLite + artifacts: [ADR-003](adr/ADR-003-sqlite-and-artifacts.md), detalhada em PD-32.

---

# 177. Threat model

Manter documento vivo cobrindo:

Estado inicial e lacunas do Delivery 4: [threat model e gates de execução](threat-model.md).

```text
malicious repository
malicious dependency
malicious MCP
prompt injection
compromised provider
stolen paired device
local attacker
secret exfiltration
destructive shell
remote network exposure
supply-chain compromise
update compromise
```

---

# 178. Política de compatibilidade

Protocol:

```text
major protocol version
```

Clients negociam versão no handshake.

Daemon deve rejeitar incompatibilidade claramente.

Atualização não pode produzir erro obscuro entre PWA/TUI e daemon.

---

# 179. Backward compatibility

Antes de v1:

- mudanças podem ocorrer;
- migrations obrigatórias;
- release notes claras.

Após v1:

- config migration;
- DB migration;
- protocol compatibility window;
- deprecation.

---

# 180. Config exemplo

Exemplo conceitual, não implementação do contrato final de perfis/fila. Defaults de
modelo/variante devem ser compatíveis com o catálogo real. `sandbox.enabled` não
garante enforcement disponível: ausência de isolamento exige consentimento explícito.

```toml
[agent]
mode = "ask"

[model]
primary = "chatgpt:default"

[reasoning]
level = "default"

[decision]
mode = "hybrid"

[decision.jev]
enabled = true

[security]
protect_secrets = true
redact_sensitive_values = true

[sandbox]
enabled = true

[resources]
max_concurrent_agents = 4
max_active_sessions = 8

[remote]
enabled = false

[mcp.ai-memory]
enabled = true
scope = "user"
lifecycle = "pinned"
```

Valores/model names reais devem vir do catálogo do provider, não ficar presos no código.

---

# 181. Critério de sucesso do produto

O Carapanã não precisa inicialmente vencer Claude Code em número de features.

Ele precisa atingir esta situação:

> Abrir `carapana` passa a ser preferível a abrir outro harness para o uso diário.

Especialmente:

```text
meu plano do ChatGPT
+
experiência de autonomia próxima ao Claude Code
+
provider freedom
+
skills disciplinadas
+
MCP persistente
+
sessão contínua
+
celular
+
Tailscale/NetBird
+
baixo uso de RAM/SSD
```

---

# 182. North Star

> **A experiência pertence ao harness, não ao modelo.**

Modelo pode mudar.

Device pode mudar.

Interface pode mudar.

A sessão, as regras, a segurança, o contexto, as ferramentas e a qualidade da engenharia continuam pertencendo ao Carapanã.

---

# 183. Primeira ação de implementação

A atualização documental e as decisões aprovadas não equivalem à aprovação do Delivery 0.
O próximo trabalho de implementação continua exclusivamente o protótipo com mocks.

A primeira tarefa do repositório não é implementar OpenAI, MCP ou shell.

É:

```text
Delivery 0
Product Design Foundation
```

Criar somente o necessário para produzir e testar:

```text
CLI UX
TUI UX
Web UX
Mobile UX
Design System
Interaction Model
State Model
```

com dados fake.

Quando estiver pronto:

```text
STOP
```

Apresentar para revisão.

**Só depois de aprovação explícita inicia-se o Delivery 1.**

---

# 184. Regra final para qualquer agente que execute este plano

Se estiver trabalhando no Delivery 0 e identificar trabalho pertencente ao Delivery 1 ou posterior:

**não implementar.**

Registrar como dependência futura e continuar o Delivery 0.

Não antecipar arquitetura "só para aproveitar".

Não implementar provider "só para testar".

Não implementar daemon "só para ligar a tela".

Mocks são suficientes.

A primeira entrega existe para impedir exatamente que a engenharia produza um harness tecnicamente sofisticado com uma experiência ruim.

---

# 185. Avaliação de compressão dos dados transportados

Antes de adicionar codecs a qualquer protocolo ou runtime, avaliar o uso de Brotli e Zstandard
(`zstd`) para tráfego local e remoto, mantendo a ausência de compressão como baseline:

- Cobrir IPC local daemon/CLI/TUI, incluindo frames JSON atualmente limitados a 64 KiB, e separar
  essa análise de tráfego HTTP remoto do provider e de futuros remote attach/MCP.
- Para tráfego remoto, verificar somente capacidades documentadas dos endpoints oficiais SIWC e
  comportamento HTTP de request/response/stream SSE; não introduzir endpoint privado, proxy ou API
  paga como requisito.
- Comparar payloads sintéticos representativos — frames pequenos/grandes, envelopes JSON, eventos,
  snapshots e diffs — medindo bytes transmitidos, taxa de compressão, latência p50/p95, CPU, pico de
  memória e custo de compressão/decompressão em hardware modesto. Incluir casos em que o codec expande
  payloads pequenos e fluxos contínuos.
- Avaliar compatibilidade/negociação por versão e capacidade, limiar para ativar compressão e
  comportamento com clientes antigos. Limites de tamanho descomprimido, razão de expansão, memória,
  tempo e cancelamento devem ser independentes do limite do frame comprimido; rejeitar truncamento,
  conteúdo malformado e decompression bombs sem alocação ilimitada.
- Revisar riscos de oracle de compressão quando segredos e conteúdo controlável forem combinados,
  particularmente sobre TLS; não comprimir esse tipo de payload sem análise e mitigação aprovadas.
  Preservar o tratamento de secrets e a integridade/autenticidade do transporte.
- Revisar licença, manutenção, compatibilidade Linux/macOS, impacto de dependência/binário e
  interoperabilidade entre Rust e clientes futuros antes de escolher uma biblioteca.
- Não escolher nem habilitar codec por antecipação: registrar resultados reprodutíveis e decisão
  explícita antes de adicionar dependência, alterar o framing/contrato ou ativar compressão em produção.

### Evidência experimental parcial — 19 de outubro de 2026

- A comparação local sintética de Brotli 4/6, Zstandard 1/3 e sem compressão está registrada em
  [`docs/compression-evaluation-2026-10-19.md`](compression-evaluation-2026-10-19.md), com dados por
  execução em [`docs/benchmarks/compression-evaluation-2026-10-19.json`](benchmarks/compression-evaluation-2026-10-19.json).
- Complemento incremental usa socketpair Unix local, sem protocolo/IPC framing, com fixture sintética de
  frame 64 KiB e SSE 256 KiB. Três campanhas de dez medições no Ryzen 9 7900 reduziram o stream SSE a
  13,71% (Brotli 4) / 10,82% (Zstd 1) dos bytes; flush por evento aumentou o round-trip p50 para 40,826 /
  32,975 ms versus 3,281 ms sem codec. Os valores e todas as amostras estão em
  [`docs/benchmarks/compression-transport-evaluation-2026-10-19.json`](benchmarks/compression-transport-evaluation-2026-10-19.json).
- O ensaio one-shot não inclui socket; o complementar mede socket local e flush incremental, mas nenhum
  deles mede framing real, HTTP/TLS ou rede. Ambos são Python/ctypes em host Linux de alto desempenho e
  não satisfazem ainda requisitos de hardware modesto/macOS.
- A documentação oficial OpenAI consultada declara zstd em request HTTP Responses, com limites de
  tamanho/expansão; não documenta Brotli HTTP nem compressão da resposta SSE. Aceitação com credencial
  SIWC e eventual cobrança não foram testadas. Sem chamada remota, mudança de contrato ou seleção de codec.
- Permanecem gates: integração com framing IPC real; HTTP oficial autorizado usando prompt sintético e
  confirmação de custo/compatibilidade SIWC; negociação/fallback e threshold baseado em transporte;
  medição em hardware modesto/macOS; decoder bounded por tamanho/janela/razão/tempo/memória/cancelamento;
  análise de oracle e revisão de licenças/builds candidatas. Testes locais de stream inválido, truncado e
  saída excessiva são exploratórios e não autorizam wiring.
