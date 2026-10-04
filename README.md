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

Web abre na sessão atual; mobile abre em **Precisa de você**. Use o seletor
**Cenário de revisão** para explorar approvals, recuperação, offline, cota,
conflitos, segredos, recursos e demais fixtures. Tema acompanha o sistema com
override. pt-BR completo; inglês ainda parcial, com fallback explícito.

CLI headless não aguarda aprovação e não executa o prompt:

```sh
cargo run --manifest-path prototypes/terminal/Cargo.toml -- scenarios
cargo run --manifest-path prototypes/terminal/Cargo.toml -- run --scenario approval --json
```

Códigos ilustrativos: `0` conclusão, `2` pausa/erro, `3` validação incompleta,
`4` aprovação necessária. `doctor`, `info`, `config` e `sessions` também são mocks.

### Teclado

- Web: `Ctrl/⌘ K` busca, `Alt P` alterna perfil, `Ctrl/⌘ Enter` envia.
- TUI: mínimo **80×24**, sem mouse/truecolor obrigatório; `--plain` sem cores.
  `Enter` escreve/envia; `b/m/v` perfil/modelo/variante; `t` painel; `j/k` rola;
  `c` cenário; `a/n` permite/nega; `f` etapa segura; `g` conclui;
  `e/d/o` edita/remove/reordena primeira mensagem; `p/r/s` pausa/retoma/para;
  `q` sai. Letras funcionam fora do editor. Para intervir sem tecla especial,
  envie com Enter e use `w` para promover a última mensagem da fila à intervenção.
  Alt/F-keys são alternativas, não requisitos. `Esc` cancela edição sem apagar rascunho.

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
format/clippy/test/build Rust e testes CLI headless. Não há infraestrutura real
para subir no Delivery 0. Títulos dos testes começam com `Should` / `should_`.

Licença definida para o projeto: **Apache-2.0**. Integração com a assinatura ChatGPT e
demais capacidades dependem de comprovação técnica; não há fallback pago automático.
