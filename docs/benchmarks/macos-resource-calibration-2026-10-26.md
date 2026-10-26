# Calibração local de recursos no Mac M3 — 2026-10-26

## Escopo

Medição no MacBook Air M3 (Mac15,12), 16 GB, macOS 26.6.2, Apple Silicon, com Rust
1.99.1 (`rustc 1.99.1 (58cf8f9c3 2026-10-13)`). O CLI release foi construído com o pin do
repositório. SHA-256 do binário usado na campanha, então chamado `carapana`:
`f159fc031dfd9551df4e57dcd809c465d29f2b5bf0227aefd8cf22c7d8a966ba`.

Esta base não tem execução de agente end-to-end, tools habilitadas ou provider conectado. Os
workloads representam apenas operações realmente disponíveis no Mac: daemon local, criação de
sessões pausadas, enqueue persistente via CLI/IPC, armazenamento SQLite e leitura da fila. Nada
foi enviado a provider ou workspace real. Os resultados não calibram inferência, tools, sandbox
Linux/cgroup, consumo de contexto ou execução concorrente de agentes.

## Workloads executados

Cada repetição usou HOME, banco SQLite e workspace vazios em uma pasta privada temporária, removida
ao fim do processo. Um warmup não entra nas amostras. Cada cenário foi repetido três vezes, com um
daemon novo e serial; a concorrência indicada é de clientes CLI que enfileiram mensagens no daemon.
O modelo de chegada é fechado: cada worker espera sua operação terminar antes de iniciar a próxima,
não mantém taxa de chegada fixa. As seleções e mensagens são fixtures sintéticas.

| Workload              | Sessões | Fila por sessão | Produtores | Total de pedidos | Payload sintético |
| --------------------- | ------: | --------------: | ---------: | ---------------: | ----------------: |
| `typical-queued-work` |       4 |       8 × 1 KiB |          1 |               32 |            32 KiB |
| `bounded-queue-burst` |       8 |       4 × 8 KiB |          4 |               32 |           256 KiB |

O segundo usa o limite atual de 8 KiB por mensagem. Mantivemos quatro mensagens por sessão para
que o snapshot JSON de cada sessão permaneça abaixo do frame IPC de 64 KiB durante a validação
persistida. Cada campanha verifica que todas as mensagens esperadas continuam na fila.

## Resultados

Valores de latência e wall time incluem inicialização de processos CLI, serialização JSON, IPC e
persistência SQLite. `RSS agregado` é a soma de RSS observado do daemon e dos filhos CLI ativos; o
RSS individual do daemon é informado à parte. Memória foi amostrada com `ps` a cada 100 ms, não é
VmHWM nem um hard limit. A soma de RSS pode contar páginas compartilhadas mais de uma vez e a
amostragem pode perder picos curtos. O armazenamento é a soma dos arquivos no diretório privado
do app, incluindo DB/WAL/SHM, antes e depois do shutdown. Cada valor de latência/wall time é a
mediana das três repetições; a faixa apresentada para RSS é mínimo–máximo dessas repetições.

| Workload      | Wall p50 | Enqueue p50 / p95 | RSS agregado base → pico amostrado (mediana; faixa) | Pico do daemon amostrado (mediana) | DB/WAL/SHM antes → após shutdown |
| ------------- | -------: | ----------------: | --------------------------------------------------: | ---------------------------------: | -------------------------------: |
| Típico        |   0,43 s |    12,8 / 23,9 ms |                  8,97 → 15,83 MiB (15,80–16,28 MiB) |                           9,77 MiB |        1.186.376 → 196.608 bytes |
| Burst bounded |   0,15 s |    15,2 / 30,6 ms |                  9,08 → 21,19 MiB (10,92–36,75 MiB) |                          10,95 MiB |        2.092.776 → 679.936 bytes |

As medianas são de apenas três repetições; há variação relevante, sobretudo no pico amostrado e no
p95. São pontos de referência para esse fluxo local, não uma capacidade máxima, limite seguro ou
SLO. Não alterar defaults de RAM, disco ou concorrência a partir destas amostras.

## Probe de admissão

Executado conforme `admission-resource-probe.md`: um warmup e três amostras seriais, binário
release `admission_resource_probe` com SHA-256
`b200d55455c59ca3953e06494fa04c48f23d676902921b6fcdfca7d11267f0ab`. Os quatro registros JSON integrais estão em
[`admission-control-plane-2026-10-26-mac.json`](admission-control-plane-2026-10-26-mac.json).

Nas três amostras, o tempo total do runner teve mediana de 585,7 ms, o `child_max_rss_bytes`
mediano foi 3.653.632 bytes e os tempos internos medianos foram 10,6 ms para abrir, 254,9 ms para
imediatos, 33,4 ms para enqueue e 268,1 ms para claim. `child_max_rss_bytes` inclui o wrapper
Python pré-exec; não é RSS isolado do Rust. O resultado continua estritamente limitado ao controle
SQLite de admissão e não substitui a medição do daemon/fila acima.

## Reprodução e artefatos

```sh
bash scripts/rust-toolchain-command.sh cargo build --locked --release -p carapanan-cli --example admission_resource_probe
bash scripts/run-admission-resource-probe.sh
bash scripts/rust-toolchain-command.sh cargo build --locked --release -p carapanan-cli
python3 scripts/measure-macos-session-workload.py
```

- [`measure-macos-session-workload.py`](../../scripts/measure-macos-session-workload.py) executa
  warmup e os dois cenários, cada um com três repetições, e valida o estado persistido.
- [`macos-session-workload-2026-10-26.json`](macos-session-workload-2026-10-26.json) contém as
  medições integrais por repetição, sem IDs de sessão, textos de usuário, paths privados ou ambiente.
- O probe pequeno usa timeout de 30 s por amostra e até 8 KiB de saída; os workloads isolam cada
  daemon/DB em fixture temporária e encerram o daemon ao final.

## Verificação do checkout

- `python3 -m unittest discover -s scripts/tests`: 11 testes passaram; `cargo fmt`, Clippy e
  `cargo build --workspace --locked` passaram. O teste de integração `daemon_process` passou (17).
- No macOS, `com.apple.provenance` é aplicado automaticamente até a arquivos de fixture. A primitive
  de escrita agora preserva byte a byte apenas esse xattr do sistema no Darwin e continua recusando
  atributos desconhecidos; os 129 testes de storage passaram.
- O handshake do guardian passou a ter timeout bounded de 15 s. O teste que aguarda timeout do
  processo foi ajustado para permitir 5 s de execução sob carga, mantendo o limite total em 12 s;
  `captured_process` passou 9/9 em duas repetições padrão.
- Foram adicionados 32 golden screenshots Darwin ausentes, após revisão visual de desktop/mobile em
  claro/escuro. Eles são separados dos baselines Linux. Um teste de responsividade com 24 navegações
  recebeu timeout de 60 s; no gate Mac levou até 48,3 s sob dois workers.
- `make verify` passou integralmente com os Darwin snapshots selecionados: format, lint, check,
  Vitest (27), build Web, E2E (66), 11 testes Python, Clippy, testes/documentation tests do workspace
  e build Rust. Reprodução no Mac: `CARAPANA_E2E_PREVIEW_PORT=34173
CARAPANA_E2E_DEV_PORT=34174 CARAPANA_SNAPSHOT_ENV=darwin mise exec -- make verify`.
