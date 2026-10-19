# Avaliação exploratória de compressão — 19 de outubro de 2026

**Status:** medições locais concluídas; compatibilidade remota parcialmente documentada; nenhum codec escolhido ou habilitado.

## Escopo e método

- O harness [`prototypes/compression_eval.py`](../prototypes/compression_eval.py) compara ausência de codec, Brotli nos níveis 4/6 e Zstandard nos níveis 1/3. Usa somente cargas sintéticas determinísticas: eventos/envelopes JSON, snapshots, diffs, frames no limite atual de 64 KiB, conteúdo opaco pouco compressível e uma sequência SSE sintética de aproximadamente 256 KiB.
- Não abre arquivos do workspace, não lê credenciais e não faz chamadas de rede ou inferência. As cargas não contêm contexto real.
- Cada perfil teve 30 aquecimentos e 300 amostras de compressão e descompressão, em três execuções seriais. O JSON com dados por execução, p50/p95, CPU por operação, tamanho e RSS está em [`docs/benchmarks/compression-evaluation-2026-10-19.json`](benchmarks/compression-evaluation-2026-10-19.json).
- Testes do harness: `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest prototypes.test_compression_eval` (3 testes; requer bibliotecas Brotli/Zstandard do sistema).
- Os bytes medidos são os do corpo codificado, incluindo o framing interno do codec, mas excluem cabeçalhos `Content-Encoding`, prefixos IPC, enquadramento HTTP/TLS e tráfego de socket. Os tempos/CPU incluem o loop Python e a fronteira `ctypes`, não serialização JSON, transporte nem latência de rede. O caso SSE comprime uma sequência agregada em uma chamada _one-shot_; não mede flush, latência do primeiro evento ou streaming real.

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

O caso opaco de 32 B expande 12,5% no Brotli e 28,1% no Zstd, antes de overhead de transporte. Os JSON estruturados comprimiram bastante; conteúdo opaco de 64 KiB caiu cerca de 17,7%, mas demandou mais tempo de descompressão que os casos estruturados. No host medido, Zstd foi mais rápido nos exemplos; o tamanho relativo variou por carga e nível. Isso não estabelece limiar: custos de IPC/rede, negociação, CPU em hardware modesto e dados reais não foram medidos.

## Capacidades remotas oficiais e provider atual

- A [documentação oficial de boas práticas](https://developers.openai.com/api/docs/guides/production-best-practices) documenta request bodies JSON com `Content-Encoding: zstd` para `POST /v1/responses`. Os corpos comprimido e descomprimido têm limite de 128 MiB, e a expansão é limitada a 100×; conteúdo inválido/incompleto recebe HTTP 400 e violações de limite HTTP 413. Não encontrei suporte documentado a Brotli para esses requests na documentação consultada.
- O guia oficial de [streaming HTTP](https://developers.openai.com/api/docs/guides/streaming-responses) descreve SSE com `stream: true`, mas não especifica compressão do corpo da resposta SSE. O guia de [WebSocket mode](https://developers.openai.com/api/docs/guides/websocket-mode) documenta `permessage-deflate`, outro transporte que não escolhe Brotli/Zstd para HTTP/SSE.
- O provider em `crates/carapana-openai` envia `.json(&request_body)`, define `Content-Type: application/json` e `Accept: text/event-stream`, sem `Content-Encoding`. O `reqwest` usa `default-features = false` e não habilita features de descompressão automática. Nenhuma alteração foi feita.
- As fontes públicas consultadas não confirmam que o token SIWC do aplicativo seja aceito com `Content-Encoding: zstd`, nem que esse caminho esteja disponível sem um plano de API pago. Não houve chamada remota, endpoint privado/proxy ou inferência faturável. Compressão da resposta SSE e latência remota seguem não medidas.

## Segurança, compatibilidade e distribuição

- O IPC atual usa JSON com limite descomprimido de 64 KiB e não negocia codec nesta fatia. Qualquer mudança futura precisaria de versão/capacidade explícita, fallback sem compressão para clientes antigos e limites independentes para bytes comprimidos e descomprimidos. Limites remotos não substituem limites locais.
- O harness testa round-trip válido, mas não conteúdo truncado/corrompido, expansão hostil, uso de memória sob ataque, timeout/cancelamento ou implementação incremental. Esses gates continuam obrigatórios; decoder local teria de limitar saída, janela/memória, razão e tempo antes de qualquer wiring.
- Compressão de prompts pode expor comprimento observável sob TLS quando segredo e texto controlável compartilham o corpo. Não comprimir esse conteúdo até análise e mitigação aprovadas; TLS continua necessário para integridade/autenticidade do transporte.
- O [Brotli upstream](https://github.com/google/brotli/blob/master/LICENSE) usa licença MIT; o software [Zstandard upstream](https://github.com/facebook/zstd/blob/dev/LICENSE) usa BSD de três cláusulas. Nenhuma biblioteca Rust/C foi adicionada. Licenças de wrappers/crates, manutenção, build/bundle em Linux/macOS e interoperabilidade exigem revisão específica se uma implementação for proposta.

## Conclusão

Resultados locais reprodutíveis foram registrados sem escolher nem ativar codec. A avaliação não fecha os requisitos de hardware modesto, round-trip IPC/HTTP, SSE incremental ou compatibilidade SIWC; permanecem abertos. Medição remota futura exige autorização explícita para uma chamada oficial com prompt sintético fixo e confirmação de que não introduzirá uso pago. Contexto real permanece fora do escopo sem consentimento separado.
