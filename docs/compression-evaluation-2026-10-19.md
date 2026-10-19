# Avaliação exploratória de compressão — 19 de outubro de 2026

**Status:** medições sintéticas one-shot e socket local concluídas; compatibilidade remota parcialmente documentada; nenhum codec escolhido ou habilitado.

## Escopo e método

- O harness [`prototypes/compression_eval.py`](../prototypes/compression_eval.py) compara ausência de codec, Brotli nos níveis 4/6 e Zstandard nos níveis 1/3. Usa somente cargas sintéticas determinísticas: eventos/envelopes JSON, snapshots, diffs, frames no limite atual de 64 KiB, conteúdo opaco pouco compressível e uma sequência SSE sintética de aproximadamente 256 KiB.
- O harness complementar [`prototypes/compression_transport_eval.py`](../prototypes/compression_transport_eval.py) mede round-trip em `AF_UNIX SOCK_STREAM socketpair`, com Brotli 4, Zstandard 1 e baseline sem codec. Inclui um frame sintético de 64 KiB e SSE sintético de 256 KiB dividido em 1.292 eventos; encoder e decoder são incrementais e cada evento comprimido é flushado antes do próximo.
- Não abre arquivos do workspace, não lê credenciais e não faz chamadas de rede ou inferência. As cargas não contêm contexto real.
- Cada perfil teve 30 aquecimentos e 300 amostras de compressão e descompressão, em três execuções seriais. O JSON com dados por execução, p50/p95, CPU por operação, tamanho e RSS está em [`docs/benchmarks/compression-evaluation-2026-10-19.json`](benchmarks/compression-evaluation-2026-10-19.json).
- O socket benchmark teve duas campanhas de aquecimento e três campanhas seriais de dez medições por perfil; a ordem dos perfis roda entre repetições. Inclui CPU das threads de envio/recepção, latência até o primeiro evento SSE completo e round-trip. Dados brutos por medição em [`docs/benchmarks/compression-transport-evaluation-2026-10-19.json`](benchmarks/compression-transport-evaluation-2026-10-19.json).
- Reprodução do socket benchmark: `PYTHONDONTWRITEBYTECODE=1 python3 prototypes/compression_transport_eval.py --iterations 10 --campaigns 3 > docs/benchmarks/compression-transport-evaluation-2026-10-19.json`.
- Testes: `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest prototypes.test_compression_eval prototypes.test_compression_transport_eval` (8 testes; requer bibliotecas Brotli/Zstandard do sistema).
- O benchmark one-shot exclui socket e transporte; o socket benchmark inclui `socketpair`, flushes e decoder incremental, mas exclui framing IPC do produto, cabeçalhos `Content-Encoding`, HTTP/TLS e rede remota. Ambos incluem Python/`ctypes`; nenhum resultado representa capacidade de produção.

Ambiente: CachyOS Linux, kernel `7.2.3-1-cachyos`, x86_64, AMD Ryzen 9 7900 (12 núcleos/24 threads), Python 3.14.7, CLI Brotli 1.2.0 e Zstandard 1.5.7. É um host de alto desempenho, não hardware modesto. O pico RSS observado foi 19,8–25,9 MiB por processo Python, incluindo runtime e biblioteca nativa; não isola alocações do codec e não é uma comparação contra processo sem compressão. Nenhuma medição foi feita em macOS.

## Bytes observados

Valores de corpos sintéticos determinísticos; `raw` é o baseline sem compressão.

| Caso                    |       Raw |       Brotli 4 |       Brotli 6 |         Zstd 1 |         Zstd 3 |
| ----------------------- | --------: | -------------: | -------------: | -------------: | -------------: |
| Bytes opacos            |      32 B | 36 B (expande) | 36 B (expande) | 41 B (expande) | 41 B (expande) |
| Evento JSON             |     256 B |          165 B |          157 B |          179 B |          179 B |
| Envelope JSON           |   4.096 B |          544 B |          489 B |          607 B |          602 B |
| Snapshot JSON           |  32.768 B |        1.768 B |        1.610 B |        1.646 B |        2.089 B |
| Diff sintético          |  33.942 B |        2.959 B |        2.208 B |        2.367 B |        2.476 B |
| Frame JSON              |  65.536 B |        3.126 B |        2.729 B |        2.441 B |        3.361 B |
| JSON com conteúdo opaco |  65.536 B |       53.883 B |       53.887 B |       53.906 B |       53.906 B |
| SSE sintético agregado  | 262.317 B |        8.309 B |        4.704 B |        4.921 B |        4.914 B |

## Latência e CPU

Microssegundos; valores são a mediana dos três p50/p95 por execução. CPU é tempo de processo médio por operação e inclui overhead Python/`ctypes`. RSS é o pico absoluto do worker Python, não memória incremental do codec.

| Caso                 | Codec    |    Encode p50/p95 | Decode p50/p95 | CPU encode/decode | Pico RSS |
| -------------------- | -------- | ----------------: | -------------: | ----------------: | -------: |
| 32 B opacos          | Brotli 4 |        7,5 / 11,2 |      1,4 / 2,5 |         8,8 / 1,8 | 20,4 MiB |
| 32 B opacos          | Zstd 1   |         2,7 / 3,0 |      1,2 / 1,3 |         2,9 / 1,3 | 20,0 MiB |
| Frame 64 KiB         | Brotli 4 |     188,3 / 309,1 |    27,3 / 33,3 |      202,4 / 28,1 | 20,9 MiB |
| Frame 64 KiB         | Brotli 6 |     376,6 / 541,5 |    26,9 / 35,5 |      402,7 / 27,9 | 22,5 MiB |
| Frame 64 KiB         | Zstd 1   |       39,3 / 52,7 |    15,2 / 22,9 |       41,7 / 16,4 | 20,0 MiB |
| Frame 64 KiB         | Zstd 3   |       43,8 / 51,5 |    16,6 / 27,2 |       45,6 / 19,1 | 20,1 MiB |
| Entropia JSON 64 KiB | Brotli 4 |     471,5 / 585,2 |  261,4 / 276,8 |     416,7 / 261,0 | 20,6 MiB |
| Entropia JSON 64 KiB | Brotli 6 |     373,2 / 568,4 |  258,9 / 274,5 |     418,9 / 259,4 | 23,3 MiB |
| Entropia JSON 64 KiB | Zstd 1   |       58,2 / 66,2 |    48,0 / 64,0 |       59,2 / 51,1 | 20,6 MiB |
| Entropia JSON 64 KiB | Zstd 3   |       64,3 / 80,2 |    49,8 / 62,3 |       66,5 / 52,1 | 20,4 MiB |
| SSE agregado 256 KiB | Brotli 4 |     647,0 / 885,2 |   86,1 / 107,8 |      684,7 / 86,4 | 21,6 MiB |
| SSE agregado 256 KiB | Brotli 6 | 1.474,2 / 2.122,8 |    68,9 / 91,3 |    1.568,1 / 71,7 | 25,9 MiB |
| SSE agregado 256 KiB | Zstd 1   |      94,8 / 136,3 |    42,2 / 58,7 |       99,8 / 44,0 | 20,7 MiB |
| SSE agregado 256 KiB | Zstd 3   |      99,9 / 151,0 |    39,2 / 67,5 |      105,4 / 45,0 | 21,5 MiB |

O caso opaco de 32 B expande 12,5% no Brotli e 28,1% no Zstd, antes de overhead de transporte. Os JSON estruturados comprimiram bastante; conteúdo opaco de 64 KiB caiu cerca de 17,7%, mas demandou mais tempo de descompressão que os casos estruturados. No host medido, Zstd foi mais rápido nos exemplos; o tamanho relativo variou por carga e nível. Isso não estabelece limiar: framing IPC real, custos da rede, negociação, CPU em hardware modesto e dados reais não foram medidos.

## Socket local e SSE incremental

Ambiente: o mesmo CachyOS/Linux x86_64 e Ryzen 9 7900 do ensaio anterior, Python 3.14.7, Brotli 1.2.0 e Zstandard 1.5.7. Cada perfil teve duas repetições de aquecimento e 30 medições cronometradas (3 campanhas seriais × 10); ordem rotativa. O receptor lê em blocos de 16 KiB. SSE é produzido sem atraso artificial e o compressor faz flush após cada evento, para expor o custo de preservar baixa latência. A fixture é verificada byte a byte após o round-trip; o decoder de teste limita a saída ao tamanho bruto conhecido da fixture.

| Caso                        | Codec    | Bytes no socket (razão/raw) | Primeiro evento p50/p95 | Round-trip p50/p95 | CPU sender/receiver p50 |
| --------------------------- | -------- | --------------------------: | ----------------------: | -----------------: | ----------------------: |
| Frame 64 KiB                | Nenhum   |               65.536 (100%) |                       — |   0,117 / 0,422 ms |        0,018 / 0,066 ms |
| Frame 64 KiB                | Brotli 4 |               3.128 (4,77%) |                       — |   0,668 / 1,027 ms |        0,413 / 0,182 ms |
| Frame 64 KiB                | Zstd 1   |               2.441 (3,72%) |                       — |   0,334 / 0,964 ms |        0,129 / 0,164 ms |
| SSE 256 KiB · 1.292 eventos | Nenhum   |              262.317 (100%) |        0,114 / 0,199 ms |   3,281 / 5,010 ms |        2,595 / 2,681 ms |
| SSE 256 KiB · 1.292 eventos | Brotli 4 |             35.964 (13,71%) |        0,294 / 0,397 ms | 40,826 / 57,768 ms |      29,980 / 25,600 ms |
| SSE 256 KiB · 1.292 eventos | Zstd 1   |             28.387 (10,82%) |        0,269 / 0,358 ms | 32,975 / 39,521 ms |      22,814 / 17,038 ms |

Tempos em milissegundos; CPU é p50 do tempo de CPU das threads de envio/recepção, incluindo Python/`ctypes` e chamadas de socket, não uma atribuição isolada ao codec. O flush por evento reduziu bytes SSE em 86,3% (Brotli) e 89,2% (Zstd), mas neste socket local aumentou bastante CPU e round-trip; não comprimir foi mais rápido. Isso é evidência sobre esse padrão de flush e fixture, não uma decisão de codec: batching, framing real, outra cadência de produção, hardware modesto e rede podem mudar o resultado.

## Proxy local com quota de recursos — não equivale a hardware modesto

Para observar o efeito de limitar concorrência, os mesmos harnesses foram executados serialmente em um
container rootless com quota de 1 CPU (`cpu.max = 100000 100000`) e memória máxima de 512 MiB. A rede
ficou desabilitada, a raiz do container somente-leitura e apenas `prototypes/` foi montado somente-leitura.
O container usou a imagem já disponível `mcr.microsoft.com/playwright:v1.58.2-noble`, Python 3.12.3,
Brotli 1.1.0 e Zstandard 1.5.5. Os oito testes dos dois harnesses passaram nessa configuração.
Reprodução a partir da raiz do checkout (o volume do código fica somente-leitura; a saída é gravada pelo
shell no host):

```sh
docker run --rm --pull=never --cpus=1 --memory=512m --network=none --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m --user "$(id -u):$(id -g)" \
  -v "$PWD/prototypes:/work/prototypes:ro" -w /tmp \
  -e PYTHONDONTWRITEBYTECODE=1 mcr.microsoft.com/playwright:v1.58.2-noble \
  python3 /work/prototypes/compression_eval.py --iterations 300 \
  > docs/benchmarks/compression-evaluation-2026-10-19-cpu-quota.json

docker run --rm --pull=never --cpus=1 --memory=512m --network=none --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m --user "$(id -u):$(id -g)" \
  -v "$PWD/prototypes:/work/prototypes:ro" -w /tmp \
  -e PYTHONDONTWRITEBYTECODE=1 mcr.microsoft.com/playwright:v1.58.2-noble \
  python3 /work/prototypes/compression_transport_eval.py --iterations 10 --campaigns 3 \
  > docs/benchmarks/compression-transport-evaluation-2026-10-19-cpu-quota.json
```

| Caso/protocolo                   | Codec    | Bytes (raw → enviados) |                     p50 / p95 |
| -------------------------------- | -------- | ---------------------: | ----------------------------: |
| Frame 64 KiB · one-shot          | Brotli 4 |         65.536 → 3.126 |           178 / 256 µs encode |
| Frame 64 KiB · one-shot          | Zstd 1   |         65.536 → 2.441 |             34 / 36 µs encode |
| Frame 64 KiB · socketpair        | Brotli 4 |         65.536 → 3.128 |   0,575 / 0,822 ms round-trip |
| Frame 64 KiB · socketpair        | Zstd 1   |         65.536 → 2.441 |   0,296 / 0,471 ms round-trip |
| SSE · 1.292 eventos · socketpair | Nenhum   |      262.317 → 262.317 |  3,307 / 22,604 ms round-trip |
| SSE · 1.292 eventos · socketpair | Brotli 4 |       262.317 → 35.964 | 49,141 / 69,291 ms round-trip |
| SSE · 1.292 eventos · socketpair | Zstd 1   |       262.317 → 28.387 | 33,335 / 61,259 ms round-trip |

Os resultados completos (stdout original dos harnesses) estão em [one-shot sob quota](benchmarks/compression-evaluation-2026-10-19-cpu-quota.json)
e [socket/SSE sob quota](benchmarks/compression-transport-evaluation-2026-10-19-cpu-quota.json); os limites
observados e a proveniência comum estão no [manifesto do ambiente](benchmarks/compression-evaluation-2026-10-19-cpu-quota-environment.json).
O one-shot usou 300 amostras por perfil; o socket, três campanhas seriais de dez medições. O quota limita
a execução concorrente das threads de envio/recepção, mas o processador físico segue sendo o Ryzen 9 7900;
512 MiB não foi um teste de pressão de memória. Estes dados não representam CPU de baixo consumo, macOS,
rede real ou integração de protocolo. Não comparar tempos absolutos com o ensaio anterior: Python e versões
nativas dos codecs diferem; compare perfis somente dentro da mesma campanha.

## Capacidades remotas oficiais e provider atual

- A [documentação oficial de boas práticas](https://developers.openai.com/api/docs/guides/production-best-practices) documenta request bodies JSON com `Content-Encoding: zstd` para `POST /v1/responses`. Os corpos comprimido e descomprimido têm limite de 128 MiB, e a expansão é limitada a 100×; conteúdo inválido/incompleto recebe HTTP 400 e violações de limite HTTP 413. Não encontrei suporte documentado a Brotli para esses requests na documentação consultada.
- O guia oficial de [streaming HTTP](https://developers.openai.com/api/docs/guides/streaming-responses) descreve SSE com `stream: true`, mas não especifica compressão do corpo da resposta SSE. O guia de [WebSocket mode](https://developers.openai.com/api/docs/guides/websocket-mode) documenta `permessage-deflate`, outro transporte que não escolhe Brotli/Zstd para HTTP/SSE.
- O provider em `crates/carapana-openai` envia `.json(&request_body)`, define `Content-Type: application/json` e `Accept: text/event-stream`, sem `Content-Encoding`. O `reqwest` usa `default-features = false` e não habilita features de descompressão automática. Nenhuma alteração foi feita.
- As fontes públicas consultadas não confirmam que o token SIWC do aplicativo seja aceito com `Content-Encoding: zstd`, nem que esse caminho esteja disponível sem um plano de API pago. Não houve chamada remota, endpoint privado/proxy ou inferência faturável. Compressão da resposta SSE e latência remota seguem não medidas.

## Segurança, compatibilidade e distribuição

- O IPC atual usa JSON com limite descomprimido de 64 KiB e não negocia codec nesta fatia. Qualquer mudança futura precisaria de versão/capacidade explícita, fallback sem compressão para clientes antigos e limites independentes para bytes comprimidos e descomprimidos. Limites remotos não substituem limites locais.
- Os testes incrementais cobrem round-trip, bytes inválidos, stream truncado e uma fixture altamente compressível que excede o limite absoluto de saída. O harness limita a saída em bytes, mas não implementa nem valida limites de janela/memória nativa, razão de expansão em todas as condições, orçamento de tempo/cancelamento ou comportamento sob ataque prolongado. Isso não é decoder de produção nem libera wiring.
- Compressão de prompts pode expor comprimento observável sob TLS quando segredo e texto controlável compartilham o corpo. Não comprimir esse conteúdo até análise e mitigação aprovadas; TLS continua necessário para integridade/autenticidade do transporte.
- O [Brotli upstream](https://github.com/google/brotli/blob/master/LICENSE) usa licença MIT; o software [Zstandard upstream](https://github.com/facebook/zstd/blob/dev/LICENSE) usa BSD de três cláusulas. Nenhuma biblioteca Rust/C foi adicionada. Licenças de wrappers/crates, manutenção, build/bundle em Linux/macOS e interoperabilidade exigem revisão específica se uma implementação for proposta.

## Conclusão

Resultados locais reprodutíveis, inclusive socketpair, SSE incremental e um proxy com quota de CPU, foram registrados sem escolher nem ativar codec. A avaliação ainda não fecha hardware modesto real/macOS, framing IPC real, HTTP/TLS, compatibilidade SIWC ou custo remoto. Medição remota futura exige autorização explícita para uma chamada oficial com prompt sintético fixo e confirmação de que não introduzirá uso pago. Contexto real permanece fora do escopo sem consentimento separado.
