# Carapanã

Local-first, model-agnostic coding-agent harness, com implementação principal em Rust.

## Planejamento

- [Plano de desenvolvimento](docs/development-plan.md)
- [Decisões de produto e comportamento — revisão de 4 de outubro de 2026](docs/product-decisions.md)

**Estado atual:** Delivery 0 em revisão: web/mobile e CLI/TUI com 21 cenários
compartilhados e estado apenas em memória. Sem runtime, daemon, banco, conexão com
providers/MCP ou execução de tarefas. Delivery 1 depende de aprovação explícita.

## Experimentar

Para desenvolver este protótipo: Node compatível com Vite 8, pnpm **12.9.1** e
Rust **1.98.1** (versões em `mise.toml`). Isso não define dependências do usuário
final: o produto distribuído deverá funcionar sem instalar Node/Rust.

```sh
pnpm install
make dev
```

`make dev` inicia web e TUI juntos. Ao sair da TUI, encerra a web. O caminho dos
logs web é exibido antes da TUI abrir. Para iniciar somente uma interface, use
`pnpm dev` ou `cargo run --manifest-path prototypes/terminal/Cargo.toml -- --plain`.

Para testar no celular, exponha a web na rede explicitamente:

```sh
HOST=0.0.0.0 PORT=5180 make dev
# Ou somente a web:
HOST=0.0.0.0 PORT=5180 pnpm dev
```

Abra `http://IP-DO-MAC:5180` no celular, no mesmo Wi-Fi; no Mac,
`ipconfig getifaddr en0` mostra o IP dessa interface. Via SSH, use o IP da máquina
remota ou o endereço Tailscale, com os dois dispositivos conectados.
`0.0.0.0` é o endereço de escuta, não o endereço para navegar. Libere a porta no
firewall se necessário; use apenas redes confiáveis, pois o protótipo não tem autenticação.
Sem variáveis (ou com valores vazios), dev usa `127.0.0.1:5173`; preview usa
`127.0.0.1:4173`. `HOST` e `PORT` também valem para
`pnpm --filter @carapana/web preview`. A porta deve estar entre 1 e 65535;
se estiver ocupada, o servidor falha em vez de mudar de porta silenciosamente.

Web abre na sessão atual; mobile abre em **Precisa de você**. Use o seletor
**Cenário de revisão** para explorar approvals, recuperação, offline, cota,
conflitos, segredos, recursos e demais fixtures. Tema acompanha o sistema com
override. pt-BR completo; inglês ainda parcial, com fallback explícito.

### Interface e identidade

Web/mobile: a busca encontra sessões, telas e abas, ignora acentos e oferece
↑/↓ + Enter, estado sem resultados e acesso por toque no cabeçalho.
Todos os seletores (incluindo tema, cenários, configurações e edição da fila/perfis)
abrem uma lista legível em dialog centralizado,
com seleção marcada, navegação por teclado e foco devolvido ao fechar.
As telas de gestão usam a largura disponível; em telas estreitas, ações quebram
para a próxima linha sem comprimir os nomes. Avisos não cobrem o composer.
Scrollbars seguem o tema com trilho neutro e puxador azul-acinzentado; rolagem
nativa por toque, teclado e mouse permanece. Alto contraste usa as cores do sistema.

Modos têm cores próprias: Planejar violeta, Perguntar azul, Auto verde-água e
Yolo coral. Low/default/high mudam a intensidade, mantendo o nome visível.
A cor da execução e das mensagens na fila segue a seleção capturada, não a
seleção da próxima mensagem. Âmbar continua reservado a atenção/aprovação.

Marca web e favicon são vetores originais de um mosquito/pernilongo, o inseto
conhecido como carapanã na Amazônia ([referência](https://portalamazonia.com/amazonia/portal-amazonia-responde-quanto-tempo-vive-um-carapana/)).
O ASCII fica **somente na TUI**, à esquerda do nome: anima as asas na entrada
(até 4 segundos) e durante o processamento simulado. `--no-animation` ou
`--plain` mantém a figura imóvel, sem alterar o fluxo da sessão.

### Fluxo da TUI para revisar

A TUI abre **vazia**, em Perguntar, sem tarefa/approval pré-carregados. Mensagens
arbitrárias são recebidas na prévia, sem fingir que um modelo as interpretou.
**`/demo` inicia explicitamente** o exemplo de autenticação, que progride por
leitura → plano → aprovação → ação simulada → validação → resultado.
O composer fica estável na base do terminal e usa toda a largura disponível.
Metadados são discretos; atividade concluída aparece recolhida, com detalhes em Ctrl+O.

Cores têm função, não decoram o texto inteiro: azul identifica interação/foco e
o marcador do usuário; ciano, resposta/atividade em andamento; verde, conclusão;
âmbar, aprovação/pausa; cinza, descrições e metadados. A seleção da paleta usa
fundo azul, com nome e descrição separados visualmente nas demais opções.
Terminais `256color`/truecolor usam cores indexadas estáveis (sem exigir RGB);
os demais usam ANSI básico. `--plain` remove todas as cores, mantendo os
marcadores, textos de estado, negrito e seleção por inversão.

A aprovação usa um seletor vertical numerado, com faixa de foco, contexto neutro
e borda âmbar. ↑/↓ ou 1/2 selecionam; **Enter confirma**. Negar é o padrão.
Tab leva ao composer sem autorizar e remove a faixa de foco da decisão.
Referências visuais: [captura oficial do Claude Code](https://code.claude.com/docs/en/permissions)
e [componente público de aprovação do OpenCode](https://github.com/anomalyco/opencode/blob/dev/packages/tui/src/routes/session/permission.tsx).
A captura do Claude tem lista vertical; o componente do OpenCode tem opções com
fundo destacado. Esta prévia combina esses sinais, sem copiar suas políticas de
permissão nem adicionar aprovação permanente. A branch `dev` do OpenCode é móvel;
não representa uma captura de uma versão instalada específica do V2.

A ajuda (`?` ou `/help`) separa Conversa, Seleção, Fila e Controle em páginas.
←/→, Tab ou 1–4 trocam o assunto; Esc fecha sem alterar o rascunho.
Teclas ficam alinhadas e destacadas em azul, descrições em texto normal,
com notas e navegação no rodapé. Todos os assuntos cabem em 80×24, inclusive
com rascunho multilinha aberto.

- A aprovação aparece **no contexto da ação**, sem precisar descobrir `/approve`.
  `←/→` ou `1/2` escolhe; `Enter` confirma. **Negar** é a opção inicialmente selecionada.
- `Tab` alterna entre a decisão e o composer; enviar dali só enfileira, nunca autoriza.
  Uma aprovação que chega enquanto você escreve não rouba o foco do rascunho.
- Mensagens seguintes aparecem em **Na fila**, com texto e seleção capturada.
  Após conclusão, a próxima começa automaticamente. Approval, negação, interrupção
  e validação incompleta bloqueiam o avanço.
- `Esc` interrompe; o histórico, o rascunho e a fila permanecem. `/resume` retoma
  explicitamente e reapresenta uma aprovação ainda necessária.
- Planejar demonstra apenas leitura/plano, sem alteração ou teste. Perfil/modelo
  escolhidos depois do envio não mudam a execução atual nem a seleção da fila.
- `/scenario` carrega explicitamente fixtures de revisão, preservando histórico,
  rascunho e fila. Não é a experiência inicial.

O roteiro `/demo` usa um exemplo fixo de autenticação: **não interpreta pedidos arbitrários**,
não chama modelo/tools nem toca arquivos. Menus e edição da fila pausam o relógio
do mock para permitir inspeção; isso não representa bloqueio de um runtime real.
Plano, diff e validação acompanham as etapas demonstradas, sem antecipar resultados.

CLI headless não aguarda aprovação e não executa o prompt:

```sh
cargo run --manifest-path prototypes/terminal/Cargo.toml -- scenarios
cargo run --manifest-path prototypes/terminal/Cargo.toml -- run --scenario approval --json
```

Códigos ilustrativos: `0` conclusão, `2` pausa/erro, `3` validação incompleta,
`4` aprovação necessária. `doctor`, `info`, `config` e `sessions` também são mocks.

### Teclado

- Web: `Ctrl/⌘ P` abre a paleta (`Ctrl/⌘ K` também), `Alt P` alterna perfil,
  `Ctrl/⌘ Enter` envia. A paleta intercepta o atalho para não abrir impressão.
- TUI: mínimo **80×24**, sem mouse/truecolor obrigatório; `--plain` sem cores.
  Input já focado: digite e envie com `Enter`; `Ctrl+J`, `Shift+Enter` (quando
  encaminhado pelo terminal) ou `\` seguido de `Enter` quebra linha.
  Colagem delimitada pelo terminal preserva quebras de linha e nunca envia sozinha.
  `↑/↓` recupera mensagens enviadas e restaura o rascunho ao voltar ao fim do histórico.
  `Ctrl+P` abre a paleta de opções (`Ctrl+K` e `/` também); `?` abre ajuda.
  `/model`, `/profile` e `/effort` abrem pickers, sem alterar a execução atual.
  Setas ou `Ctrl+P/N` navegam; `Enter` confirma; `Esc` cancela e preserva rascunho.
  No picker de modelo, esquerda/direita escolhe raciocínio explicitamente.
  `/queue` abre a fila: selecione qualquer mensagem; `Enter` edita, `d` remove,
  `-/+` reordena. Ao editar, `Enter` salva e `Esc` cancela, restaurando o rascunho.
  `/config`, `/sessions`, `/mcp`, `/plan`, `/diff` e `/scenario` abrem opções sob demanda.
  Para intervir com um rascunho, `Ctrl+P`, busque `intervene` e confirme.
  `/safe` simula etapa segura; `/approve` reabre uma decisão pendente;
  `/finish` avança manualmente o roteiro, sem contornar aprovações.
  `Ctrl+O` expande/recolhe detalhes mock de tools (ou retorna à conversa);
  `Ctrl+T` alterna plano/conversa; `PageUp/Down` rola; `Ctrl+Q` sai.
  Na conversa, `Esc` ou `Ctrl+C` interrompe a execução simulada sem perder fila,
  rascunho ou alterações; `/resume` retoma explicitamente. Em um popup, essas
  teclas cancelam apenas o popup; na edição da fila, cancelam apenas a edição.
  Quando ocioso, `Ctrl+C` limpa o input; outra pressão em até 2s sai.
  `Cmd+C` continua reservado à cópia pelo terminal, não é alias de interrupção.
  `Alt+P/M/V` são alternativas para modelo/perfil/raciocínio, não requisitos via SSH.
  A paleta mostra os atalhos diretos ao lado das ações. Dentro de um menu,
  `Ctrl+P` significa item anterior, não reabre a paleta.
  No terminal macOS, `Cmd+P` é aceito se o terminal encaminhar Command/Super.
  Terminais que interceptam essa tecla precisam mapeá-la para enviar `Ctrl+P`;
  `Ctrl+P` funciona sem esse mapeamento. Isso não altera atalhos do terminal automaticamente.

A revisão da TUI usa os padrões documentados de [modo interativo do Claude Code](https://code.claude.com/docs/en/interactive-mode)
e [picker de modelo](https://code.claude.com/docs/en/model-config#setting-your-model).
Não copia sua política de aplicação/persistência: a seleção continua válida apenas
para o próximo envio, conforme nossas decisões de produto. Não há integração real.

Comparação local realizada com Claude Code **2.1.289**, em diretório temporário
vazio, safe mode, sem tools/MCP e sem enviar prompts: input, autocomplete `/`,
picker `/model`, esforço ←/→, cancelamento e ajuda `?` foram observados no binário.
Interrupção, histórico, colagem e multilinha foram comparados com a documentação
oficial de modo interativo/configuração do terminal, não com execução real de agente.
Não se copia o default persistente do picker, envio que altera o turno atual,
nem atalhos de shell/rewind que impliquem runtime ou descarte de alterações.
O protótipo continua sem runtime. Mac e SSH remoto ainda exigem revisão própria.
Na reconstrução do fluxo, as telas locais do Claude e do binário Carapanã foram
capturadas em PTY e inspecionadas visualmente, incluindo sessão vazia, aprovação,
fila e resultado. Capturas reconstruídas de células de terminal não equivalem a
screenshot nativo do Mac nem a aprovação humana de UX.

### Revisão do Delivery 0

- [ ] Aprovar hierarquia desktop e entrada mobile por atenção.
- [ ] Revisar temas claro/escuro, densidade, foco, contraste e telas estreitas.
- [ ] Confirmar distinção entre execução atual, próxima mensagem e seleção capturada na fila.
- [ ] Exercitar intervenção segura, invalidação de approval, edição e recuperação da fila.
- [ ] Revisar plano, diff, checkpoint, evidências e validação incompleta.
- [ ] Revisar MCP, skills, providers, dispositivos, recursos e configuração efetiva.
- [ ] Testar teclado/SSH em Mac M3 e `tirion-tailscale`; validação local Linux não os substitui.
- [ ] Aprovar explicitamente antes do Delivery 1.

Fundação visual: graphite/cool-gray, azul mineral para interação e âmbar para
atenção; IBM Plex Sans/Mono. Tokens semânticos em `packages/design/tokens.css`;
labels em `apps/web/src/lib/copy.ts` e `content.ts`. Screenshots de referência em
`apps/web/tests/prototype.spec.ts-snapshots/` são baselines técnicos, não aprovação humana.

**Limites desta revisão:** formulários web simulam mudanças; painéis de gestão da
TUI são resumos navegáveis, não paridade funcional completa com a web. Pareamento
é ilustrativo; o manifest mobile não inclui cache offline/service worker. Sessões
e preferências desaparecem ao recarregar; “salvar padrão” apenas demonstra a ação.
Modelos/cotas não comprovam entitlement ChatGPT. Nenhum diagnóstico inspeciona o host.

## Verificação

```sh
pnpm --filter @carapana/web exec playwright install chromium
just verify
```

O gate inclui format/lint/typecheck/unit/build/e2e web, snapshots desktop/mobile,
format/clippy/test/build Rust, testes CLI headless e interação do binário em PTY
Unix (requer Python 3 para esse teste). O launcher tem regressão para o contrato
de template do `mktemp` BSD e encerramento da web na saída da TUI. A simulação do
contrato BSD no Linux não substitui execução no macOS. Não há infraestrutura real
para subir no Delivery 0. Títulos dos testes começam com `Should` / `should_`.

Para registrar frames do fluxo real do binário (JSON com células do terminal),
após `cargo build --manifest-path prototypes/terminal/Cargo.toml`:

```sh
CARAPANA_REVIEW_DIR=/tmp/opencode/carapana-review \
  python3 prototypes/terminal/tests/pty_smoke.py \
  "$PWD/prototypes/terminal/target/debug/carapana-prototype"
```

O teste PTY atravessa envio → atividade → decisão inline → resultado → avanço
da fila, além de menus, colagem, resize, interrupção e saída. Não apenas abre telas.

Licença definida para o projeto: **Apache-2.0**. Integração com a assinatura ChatGPT e
demais capacidades dependem de comprovação técnica; não há fallback pago automático.
