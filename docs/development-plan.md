
# Carapanã

## Plano completo de desenvolvimento

**Status:** especificação de produto e engenharia  
**Baseline:** outubro de 2026  
**Nome do produto:** Carapanã  
**Binário:** `carapana`  
**Categoria:** local-first, model-agnostic coding agent harness  
**Implementação principal:** Rust  
**Objetivo inicial:** obter uma experiência de uso comparável ou superior ao Claude Code utilizando prioritariamente o plano do ChatGPT, sem ficar preso a um único provider, dispositivo ou interface.

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

A OpenAI atualmente permite que aplicações participantes ofereçam login com ChatGPT e, para planos elegíveis, consumo da cota de Work/Codex sem compartilhamento de API key. A implementação do Carapanã deverá utilizar somente esse fluxo oficial quando disponível para o projeto; scraping de credenciais, reutilização indevida de tokens do Codex ou endpoints privados são proibidos.

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

Exemplo:

```text
┌─────────────────────────────────────────────────────────────┐
│ Carapanã · quintal-api     AUTO · GPT       Context 31%    │
├────────────┬────────────────────────────────────────────────┤
│ Sessions   │                                                │
│            │  ● Inspecting authentication flow             │
│ ● quintal  │  ● Editing refresh_token.rs                   │
│ ◐ tixnow   │  ✓ cargo test                                 │
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
quintal-api

Remote
github.com/acme/quintal-api

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

● quintal-api
  Running tests
  2m

● tixnow
  Implementing checkout retry
  7m
```

Pergunta que essa tela precisa responder imediatamente:

> Tem alguma coisa precisando de mim?

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

Os protótipos podem utilizar mocks.

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
cd ~/Code/quintal
carapana
```

quando já existir uma sessão ativa deve mostrar:

```text
Active session found

quintal-api
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

1. quintal-api
   Waiting for approval

2. carapana
   Tests failed repeatedly

3. tixnow-web
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
running
failed
```

conforme segurança.

Nunca repetir automaticamente operação destrutiva cujo resultado esteja incerto.

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

Evitar que dois agents modifiquem o mesmo working tree inadvertidamente.

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

Mesmo `yolo` respeita hard safety constraints configuradas pelo usuário.

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

`secret.read_value` exige autorização explícita.

---

# 80. Redaction e tainting

Valor sensível autorizado entra marcado como:

```text
Sensitive<T>
```

Não deve ser automaticamente:

- logado;
- armazenado;
- enviado a classifier;
- enviado a outro provider;
- colocado em artifact;
- incluído em report.

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

---

# 124. Dynamic defaults

Defaults devem considerar:

- RAM disponível;
- disco livre;
- arquitetura;
- OS.

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

Caso o delegated ChatGPT-plan integration dependa de habilitação externa ainda não concedida:

- mock permanece para testes;
- OpenAI API pode ser adapter temporário de desenvolvimento;
- não criar workaround não oficial.

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

---

# 177. Threat model

Manter documento vivo cobrindo:

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

```toml
[agent]
mode = "auto"

[model]
primary = "chatgpt:default"

[reasoning]
level = "high"

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

