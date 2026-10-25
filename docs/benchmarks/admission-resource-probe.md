# Baseline sintético de admissão

Este probe mede **somente o controle SQLite de leases e fila**, não a capacidade total do harness.
Não usa inferência, contexto real, tools, workspace do usuário ou processos externos de tarefas.
O orçamento nominal reservado não é uma alocação equivalente de RSS. Não escolher defaults de
sessões/agentes a partir deste resultado. Referências: PD-61 e
[`admission-control-plane-2026-10-25.json`](admission-control-plane-2026-10-25.json).

## Campanha reproduzível

Com os requisitos já instalados, na raiz do checkout:

```sh
bash scripts/rust-toolchain-command.sh cargo build --offline --locked --release -p carapana-cli --example admission_resource_probe
bash scripts/run-admission-resource-probe.sh
bash scripts/rust-toolchain-command.sh rustc --version
uname -sm
```

O helper usa o pin do repositório, sem instalar software ou mudar defaults globais. Offline falha
se faltarem dependências no cache; não há fallback silencioso para outro compilador. O runner
produz quatro linhas JSON: descarte a primeira (warmup), preserve as outras três. Cada processo
usa fixtures privadas novas, 16 operações por fase, prazo de 30 s e saída de até 8 KiB.

Linux exige unidade transitória própria de `systemd --user` com 256 MiB, swap zero, 32 tarefas,
CPU 100% e 60 s, `Delegate=memory pids` e `DelegateSubgroup=runner`. A unidade é recolhida ao final;
não há sudo, instalação, configuração persistente ou alteração de outras unidades. O wrapper de
medição não é um sandbox genérico: execute somente o binário conhecido que acabou de compilar.

## Mac M3/16 GB — validação manual do operador

Use os mesmos dois comandos. O agente não fará SSH no Mac. No macOS não há cgroup: o script
executa diretamente o probe pequeno e fixo, com timeout do runner, sem alegar hard limit de RAM.
Envie as quatro linhas JSON e a versão de `rustc` usada. Não envie environment, credenciais,
comandos de tarefas ou conteúdo de workspace. O binário nativo terá outro SHA-256/arquitetura.

`child_max_rss_bytes` é normalizado por plataforma, mas **pode incluir a memória pré-exec do
wrapper Python**; `rusage_includes_preexec=true` torna essa limitação explícita. Não o tratar como
RSS isolado do Rust. Linux reporta separadamente `probe.self_rss_peak_bytes` de seu próprio VmHWM.
No Mac esse campo é omitido; uma medição isolada adicional pode usar `/usr/bin/time -l` diretamente
sobre o binário, sem wrapper Python, antes de qualquer conclusão sobre RSS.

## Evidência de 25/10/2026

Ryzen e Tirion executaram o mesmo binário release Rust 1.99.1, identificado por SHA-256 no JSON.
Um warmup e três amostras por máquina terminaram sem tickets ou reservas restantes. O pico VmHWM
mediano do probe ficou em aproximadamente 5,2 MiB em ambas. Tempos de fsync e contenção do host
variam; os tempos menores no Tirion não significam maior capacidade do hardware. Mac permanece
pendente. A campanha com o pin antigo 1.98.1 está preservada somente como
[histórico](admission-control-plane-2026-10-25-rust-1.98.1.json); o operador determinou usar 1.99.1.
Versões exploratórias que atribuíam RSS pré-exec do wrapper ao probe não compõem este baseline.
