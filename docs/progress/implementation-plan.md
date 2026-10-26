# Tarefa: concluir a implementação do plano do Carapanã

- **ID:** implementation-plan.
- **Responsável:** agente que retomar esta tarefa neste checkout; operador para validação manual
  do Mac. ID de sessão do agente não foi persistido neste arquivo.
- **Atualizado:** 2026-10-26, revisão 7.
- **Estado:** parcial; não está concluído nem inteiramente bloqueado.
- **Checkout de referência:** `feat/delivery-2-local-ipc`, base de código `2bd07f6`.
  O commit documental foi publicado em `origin`; o gate local de formatação e referências passou.

## Objetivo e limites

Implementar o [plano de desenvolvimento](../development-plan.md), começando pelos gates restantes
do Delivery 4 e depois pelas entregas seguintes. Fechar uma fatia não conclui o delivery nem o
roadmap. Não há percentual de conclusão validado.

- Rust **1.99.1**, conforme determinação do operador e `rust-toolchain.toml`.
- `ToolCallRequested` continua inerte. Não ativar tools, inferência/contexto reais ou autonomia
  implicitamente como consequência desta tarefa; conferir autorização separada para cada etapa.
- Sem merge/deploy. As autorizações anteriores de commit/push foram usadas no lote PD-55–61;
  não presumir autorização automática para publicar tarefas novas.
- Mac M3/16 GB será validado manualmente pelo operador; não acessar o Mac por SSH.
- Diagnóstico/testes sintéticos no Tirion foram autorizados, sem instalação, sudo ou mudanças
  persistentes. Campanhas Linux somente em unidade transitória exclusiva, delegada e limitada.
- O arquivo guarda contexto histórico de escopo, não concede novas permissões.

## Checklist e progresso

### Implementado e verificado na base

- [x] Fundação de daemon, SQLite, sessões, filas e CLI/TUI; Web/PWA permanece um protótipo isolado.
- [x] Primitives internas de filesystem, busca, parsing, edição/diff e artifacts.
- [x] Launcher interno Linux com imagens/snapshots mínimos, aprovações exatas, supervisor,
      cancelamento e cgroup com contenção real de memória/PIDs.
- [x] PD-55/56: reamostragem de memória durante execução e cotas por workspace físico.
- [x] PD-57/58/60: recovery de preparos incompletos/não exportados ou com término dos consumidores
      comprovado; leases antigas, incertas ou com consumidores possivelmente vivos ficam cobradas.
- [x] PD-59: espera explícita, persistente e limitada, justa entre workspaces elegíveis e FIFO interno;
      ticket não autoriza execução e reinício não inicia trabalho.
- [x] PD-61: baseline sintético de admissão no Ryzen/Tirion e helper que usa o pin Rust do repositório.
- [x] Correção de cleanup do probe macOS publicada; build/testes do CI Linux/macOS verdes.

### Ainda necessário para a etapa atual

- [ ] Proteger/classificar conteúdo, ambiente e outputs antes de UI/log/artifact/provider; impedir
      exposição de segredos. Denylist de nomes e sandbox não substituem esse gate.
- [ ] Autenticar a origem dos approvals e seleções no fluxo real CLI/TUI/IPC, não apenas tipar tokens.
- [ ] Integrar ferramentas à jornada real com esses gates, incluindo Git/worktrees operacionais e
      edição/execução preservando alterações anteriores. Ainda não existe agente de coding fim a fim.
- [ ] Resolver recovery de crash durante execução quando não há prova persistida de término;
      não liberar por PID/TTL nem tratar morte do controlador como prova de cleanup.
- [x] Medição local no Mac M3/16 GB da admissão e dos workloads disponíveis de daemon/sessões/fila. Defaults finais continuam pendentes por falta de runtime de agente, tools e provider. Resultado/limites em [relatório de calibração](../benchmarks/macos-resource-calibration-2026-10-26.md); dados integrais em [probe](../benchmarks/admission-control-plane-2026-10-26-mac.json) e [workloads](../benchmarks/macos-session-workload-2026-10-26.json).
- [ ] Medir workloads completos do harness para defaults finais de RAM, disco e concorrência;
      o baseline de admissão sozinho não permite essa calibração.
- [ ] Fechar o gate manual na conta ChatGPT autorizada e conectar o adapter à jornada de sessões,
      sem converter essa pendência em autorização para inferência/contexto reais nesta retomada.

### Depois da etapa atual

- [ ] Fechar Deliveries 5–9: Context Engine, Auto/Policy completo, skills, validation/completion e MCP,
      incluindo as jornadas de primeiro dogfooding definidas no plano.
- [ ] Fechar Deliveries 10–15: multi-device, Web/PWA real, expansão de providers, subagents, hardening
      e distribuição/release. Não confundir provas de componentes com conclusão dessas jornadas.

## Decisões e referências

- [Decisões de produto, incluindo PD-53–61](../product-decisions.md).
- [Threat model e lacunas de enforcement](../threat-model.md).
- [Baseline de admissão e instruções do Mac](../benchmarks/admission-resource-probe.md).
- [Resultado do baseline Rust 1.99.1](../benchmarks/admission-control-plane-2026-10-25.json).
- [ADRs](../adr/) e [ciclo de vida deste registro](../adr/ADR-011-task-progress-lifecycle.md).

O baseline mediu aproximadamente 5,2 MiB de VmHWM do **probe SQLite de admissão**, não RSS total
do harness. Não escolher defaults finais nem ranquear capacidade dos hosts com esses números.

## Evidências e validação

- `97d39c7`: implementação do lote PD-55–61.
- `b70d8f9`: correção do runner de medição após `EPERM` em macOS; CI `40175166034` verde nos três jobs.
- `41201f2`: registro durável da validação; CI `40175516818` verde em Web/Linux e Rust/Ubuntu/macOS.
- Gate integral local com Rust 1.99.1: 27 Vitest, 66 E2E, oito testes Python, 130 testes de storage e
  demais suites Rust/doctests/Clippy/builds. Três campanhas reais: 9 backend cgroup, 6 captured cgroup
  e 27 image launcher por campanha. Essas são evidências da base, não novos testes desta alteração.
- Solicitação documental concluída localmente: política em `AGENTS.md`, índice de progresso,
  snapshot e ADR-011 criados; README e plano referenciam o ciclo de vida.
- `pnpm format:check` (via mise) passou, incluindo Biome e Prettier; `git diff --check` passou.
  Conferidos 37 destinos de links locais nos seis documentos; todos existem. Essa conferência
  verifica arquivos/diretórios de destino, não valida automaticamente fragmentos de títulos.
- `make verify` não foi repetido nesta solicitação exclusivamente documental. Os testes de runtime
  acima são evidências históricas da base `41201f2`, não uma nova execução. Sem commit/push deste lote.
- Tentativa de validação Mac em `a9152ab`: host de referência confirmado; build offline do probe
  falhou na resolução local de `rusqlite` antes da compilação. Nenhuma medição executada. O checkout
  também não fornece workload end-to-end representativo para macOS; os testes de cgroup/launcher
  mencionados são específicos de Linux. Não inferir defaults de recursos desses resultados.
- Em seguida, com autorização do operador para baixar crates, probe e CLI release foram construídos.
  Campanha Mac local executou o probe de admissão e workloads de daemon/IPC/fila, documentados em
  `docs/benchmarks/macos-resource-calibration-2026-10-26.md`. São evidências do controle local de
  sessões/fila, não do runtime completo nem de capacidade máxima; defaults finais seguem pendentes.
  A primitive macOS agora preserva o `com.apple.provenance` automático e mantém recusados outros
  xattrs; o handshake do guardian tem limite bounded de 15 s. Foram adicionados Darwin goldens após
  revisão visual e timeout de 60 s para o caso de responsividade. `make verify` completo passou no
  Mac em 26/10, com 66 E2E, 27 Vitest, 11 Python, testes Rust/doctests, Clippy, fmt e build.

## Bloqueios e próxima ação

- **Operador/produto:** para calibrar o harness completo, definir workloads end-to-end que exercitem
  runtime, tools, provider e limites reais de recursos quando essas jornadas estiverem conectadas.
  A campanha Mac atual cobre somente sessões/fila; o macOS do CI não substitui workloads do produto.
- **Agente:** próxima fatia de implementação é proteção de conteúdo/outputs e origem autenticada
  dos approvals, antes de conectar a primeira jornada operacional CLI/TUI.
- **Entrega documental:** política/ADR e documentos validados localmente e publicados em
  `2bd07f6` (`docs(progress): track task lifecycle and roadmap status`) na branch
  `feat/delivery-2-local-ipc`. Push para `origin` confirmou avanço `41201f2..2bd07f6`; árvore limpa.
  Sem merge/deploy. O CI deste commit documental ainda não foi consultado.
- **Próxima ação:** continuar a implementação da etapa atual nos gates de conteúdo/segredos e
  origem autenticada de approvals; a validação do Mac e workloads representativos seguem com seus
  responsáveis indicados acima. Manter este arquivo porque o plano amplo continua parcial. Não há
  comando de background em execução a recuperar aqui.

Remover este registro somente quando o escopo acima estiver concluído e o resultado, evidências,
decisões e limitações estiverem preservados nos documentos permanentes; não ao terminar uma fatia.
