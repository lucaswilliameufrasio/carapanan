<script lang="ts">
  import { untrack } from 'svelte';
  import { Cpu, MemoryStick } from '@lucide/svelte';
  import Picker from './Picker.svelte';
  import {
    machines,
    resourceErrors,
    resourcePreview,
    type ResourceSettings,
    type Machine,
  } from './configuration';
  let {
    settings,
    offline,
    onchange,
    onnotice,
  }: {
    settings: ResourceSettings;
    offline: boolean;
    onchange: (settings: ResourceSettings) => void;
    onnotice: (text: string) => void;
  } = $props();
  let draft = $state<ResourceSettings>(untrack(() => structuredClone($state.snapshot(settings))));
  let machine = $state<Machine>('mac');
  let pressure = $state(false);
  let errors = $state<string[]>([]);
  const preview = $derived(
    resourceErrors(draft).length ? null : resourcePreview(draft, machine, pressure),
  );
  function apply() {
    if (offline) return;
    errors = resourceErrors(draft);
    if (errors.length) return;
    onchange(structuredClone($state.snapshot(draft)));
    onnotice('Limites salvos no mock. Nenhum processo, scheduler ou medição real foi alterado.');
  }
</script>

<div class="section-heading">
  <div>
    <h2>Recursos e paralelismo</h2>
    <p>Defina o orçamento. A adaptação nunca ultrapassa os tetos que você autorizou.</p>
  </div>
  <span class="configuration-badge">Simulação</span>
</div>
<form
  class="resource-form"
  novalidate
  onsubmit={(event) => {
    event.preventDefault();
    apply();
  }}
>
  <div class="resource-grid">
    <section class="detail-block resource-budget">
      <h3>Política de recursos</h3>
      <div class="choice-group" role="group" aria-label="Política de recursos">
        <button
          type="button"
          disabled={offline}
          aria-pressed={draft.policy === 'adaptive'}
          onclick={() => (draft.policy = 'adaptive')}
          >Adaptativo<small>Ajusta dentro dos seus tetos</small></button
        >
        <button
          type="button"
          disabled={offline}
          aria-pressed={draft.policy === 'manual'}
          onclick={() => (draft.policy = 'manual')}
          >Manual<small>Usa os limites informados</small></button
        >
      </div>
      <h3><MemoryStick size={18} />Orçamento de memória</h3>
      <div class="field-grid">
        <label
          >Limite suave (GiB)<input
            type="number"
            min="0.25"
            step="0.25"
            disabled={offline}
            bind:value={draft.soft}
          /><small>Ao atingir: reduzir novas tarefas.</small></label
        >
        <label
          >Teto de memória (GiB)<input
            type="number"
            min="0.25"
            step="0.25"
            disabled={offline}
            bind:value={draft.hard}
          /><small>Não aumentar automaticamente.</small></label
        >
      </div>
      <h3><Cpu size={18} />Paralelismo máximo</h3>
      <div class="field-grid">
        <label
          >Agentes simultâneos<input
            type="number"
            min="1"
            step="1"
            disabled={offline}
            bind:value={draft.agents}
          /></label
        >
        <label
          >Sessões ativas<input
            type="number"
            min="1"
            step="1"
            disabled={offline}
            bind:value={draft.sessions}
          /></label
        >
        <label
          >Processos por sessão<input
            type="number"
            min="1"
            step="1"
            disabled={offline}
            bind:value={draft.processes}
          /></label
        >
      </div>
      <p class="muted">
        Sem capacidade para subagent: continuar sozinho ou aguardar. Pausar só quando não houver
        alternativa segura dentro do orçamento.
      </p>
    </section>
    <section class="detail-block resource-preview">
      <h3>Prévia de adaptação</h3>
      <p>Fixtures, não leituras da sua máquina. A fórmula é ilustrativa, não benchmark.</p>
      <div class="form-field">
        <span>Máquina de referência</span><Picker
          label="Máquina de referência"
          value={machine}
          options={machines.map((m) => ({ value: m.id, label: m.name }))}
          onchange={(value) => (machine = value as Machine)}
        />
      </div>
      <label class="setting-row"
        ><span>Simular pressão de memória</span><input
          type="checkbox"
          bind:checked={pressure}
        /></label
      >
      {#if preview}<div class="preview-metrics" aria-live="polite">
          <div><strong>{preview.agents}</strong><span>agentes · teto {draft.agents}</span></div>
          <div>
            <strong>{Number(preview.hard.toFixed(2))} GiB</strong><span
              >memória · teto {draft.hard} GiB</span
            >
          </div>
        </div>
        <p>
          {Number(preview.soft.toFixed(2))} GiB suave · {preview.available} GiB disponíveis · {preview.cpu}
          núcleos (mock)
        </p>
        <p class="inline-note">
          {draft.policy === 'manual'
            ? 'Modo manual: seus limites permanecem; pressão não autoriza ultrapassá-los.'
            : pressure
              ? 'Pressão simulada: menos agentes e menos memória, sem aumentar o orçamento.'
              : 'Capacidade ajustada à referência, sempre dentro dos tetos.'}
        </p>
      {:else}<p class="inline-note">Preencha limites válidos para ver a prévia.</p>{/if}
      <div class="metric-row"><span>Harness</span><code>96 MiB · mock</code></div>
      <div class="metric-row"><span>Builds / testes / MCPs</span><code>646 MiB · mock</code></div>
      <div class="metric-row"><strong>Total supervisionado</strong><code>742 MiB · mock</code></div>
      <p class="muted">RAM e CPU são separados da quota e dos tokens do provider.</p>
    </section>
  </div>
  {#if errors.length}<div class="form-errors" role="alert">
      <ul>
        {#each errors as error (error)}<li>{error}</li>{/each}
      </ul>
    </div>{/if}
  <div class="actions">
    <button class="primary" type="submit" disabled={offline}>Aplicar no mock</button><button
      type="button"
      disabled={offline}
      onclick={() => {
        draft = structuredClone($state.snapshot(settings));
        errors = [];
      }}>Descartar ajustes</button
    ><small>Somente nesta página; nada é persistido em disco.</small>
  </div>
</form>
