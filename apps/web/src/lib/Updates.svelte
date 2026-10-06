<script lang="ts">
  import Download from '@lucide/svelte/icons/download';
  import ShieldCheck from '@lucide/svelte/icons/shield-check';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Check from '@lucide/svelte/icons/check';
  import Clock from '@lucide/svelte/icons/clock';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import Picker from './Picker.svelte';
  import {
    updateFixtures,
    updateLabels,
    updateVersion,
    updateRelease,
    reduceUpdate,
    type UpdateState,
    type UpdateAction,
    type UpdateFixture,
  } from './updates';
  let {
    state,
    offline,
    onchange,
  }: { state: UpdateState; offline: boolean; onchange: (state: UpdateState) => void } = $props();
  const release = $derived(updateRelease(state));
  function action(action: UpdateAction) {
    if (!offline || action.type === 'later') onchange(reduceUpdate(state, action));
  }
</script>

<div class="updates-preview grid gap-[18px]">
  <aside class="inline-note">
    <Download size={18} />
    <p>
      Prévia Delivery 0: versões fictícias. Nenhum pacote será consultado, baixado ou instalado.
    </p>
  </aside>
  <div class="update-release">
    <div>
      <small>Versão na prévia</small><strong
        >{state.status === 'installed' ? updateVersion(state) : '0.0.0-demo'}</strong
      >
    </div>
    <div><small>Nova versão · {state.channel}</small><strong>{updateVersion(state)}</strong></div>
  </div>
  <section
    class="update-status"
    class:warning={state.status === 'ready' || state.status === 'scheduled'}
    class:danger={state.status === 'failed'}
    role="status"
    aria-live="polite"
  >
    {#if state.status === 'failed'}<TriangleAlert
        size={20}
      />{:else if state.status === 'installed' || state.status === 'rolled-back'}<Check
        size={20}
      />{:else if state.status === 'scheduled'}<Clock size={20} />{:else}<Download size={20} />{/if}
    <div>
      <strong>{updateLabels[state.status]}</strong>
      <p>
        {state.status === 'scheduled'
          ? 'Aguardar tarefa e fila em um ponto seguro. A prévia não altera sua conversa.'
          : state.status === 'verifying'
            ? 'Próxima etapa: simular a verificação de manifest, assinatura e hash.'
            : state.status === 'ready'
              ? 'Pacote preparado na prévia. Trocar o runtime exige reinício; nunca durante uma tarefa.'
              : state.status === 'installed'
                ? release.runtime
                  ? 'Reinício e retomada simulados. Nenhum processo real foi reiniciado.'
                  : 'Interface aplicada na prévia, sem reiniciar o agente ou tocar na sessão.'
                : state.status === 'failed'
                  ? 'Assinatura inválida na fixture. Instalação bloqueada; versão atual preservada.'
                  : state.status === 'rolled-back'
                    ? 'Versão anterior restaurada apenas na prévia. Nenhum arquivo foi modificado.'
                    : release.runtime
                      ? 'Esta atualização requer reinício do runtime. Preparar não interrompe a sessão.'
                      : 'Pacote somente da interface: compatível com o runtime da prévia, sem reinício.'}
      </p>
    </div>
  </section>
  <section class="update-notes">
    <h3>O que muda</h3>
    <ul>
      {#each release.notes as note (note)}<li>{note}</li>{/each}
    </ul>
    <p class="update-integrity">
      <ShieldCheck size={16} />Manifest, assinatura e hash: {state.status === 'failed'
        ? 'reprovados no mock'
        : ['ready', 'installed', 'rolled-back'].includes(state.status)
          ? 'aprovados no mock'
          : 'não verificados · mock'}
    </p>
    <small
      >{release.rollback
        ? 'Rollback compatível nesta fixture.'
        : 'Esta fixture não permite rollback automático: uma migração incompatível exige recuperação planejada.'}</small
    >
  </section>
  <div class="actions flex-wrap max-[740px]:[&>button]:min-h-11">
    {#if state.status === 'available'}
      <button class="primary" disabled={offline} onclick={() => action({ type: 'now' })}
        >Atualizar agora · mock</button
      >
      <button disabled={offline} onclick={() => action({ type: 'schedule' })}
        >Ao terminar a tarefa</button
      >
    {:else if state.status === 'scheduled'}
      <button class="primary" disabled={offline} onclick={() => action({ type: 'idle' })}
        >Simular término da tarefa e fila</button
      >
      <button disabled={offline} onclick={() => action({ type: 'now' })}
        >Preparar agora · mock</button
      >
    {:else if state.status === 'verifying'}
      <button class="primary" disabled={offline} onclick={() => action({ type: 'verify' })}
        ><ShieldCheck size={16} />Simular verificação</button
      >
    {:else if state.status === 'ready'}
      {#if state.busy}<button disabled={offline} onclick={() => action({ type: 'idle' })}
          >Simular ponto seguro</button
        >{/if}
      <button
        class="primary"
        disabled={offline || state.busy}
        onclick={() => action({ type: 'activate' })}
        ><RefreshCw size={16} />Simular reinício e retomada</button
      >
    {:else if state.status === 'installed' && release.rollback}
      <button disabled={offline} onclick={() => action({ type: 'rollback' })}
        >Simular rollback</button
      >
    {/if}
    <button onclick={() => action({ type: 'later' })}>Lembrar depois</button>
  </div>
  {#if state.status === 'ready' && state.busy}<p class="muted">
      Sessão ocupada na prévia: o reinício permanece bloqueado.
    </p>{/if}
  {#if state.hidden}<p class="muted" role="status">
      Aviso oculto até o próximo estado simulado ou recarregamento. Atualizações continua acessível
      pelo menu e pela busca.
    </p>{/if}
  {#if offline}<p class="inline-note">
      Desconectado: ações de atualização bloqueadas. Sua sessão permanece intacta.
    </p>{/if}
  <details class="update-fixtures border-t border-border pt-4">
    <summary>Explorar cenários de atualização</summary>
    <p class="muted">Trocar canal ou pacote reinicia somente esta demonstração.</p>
    <div class="field-grid">
      <div class="form-field">
        <span>Canal · mock</span><Picker
          label="Canal de atualização"
          value={state.channel}
          disabled={offline}
          options={['stable', 'beta', 'nightly'].map((value) => ({ value, label: value }))}
          onchange={(channel) =>
            action({ type: 'channel', channel: channel as UpdateState['channel'] })}
        />
      </div>
      <div class="form-field">
        <span>Pacote · mock</span><Picker
          label="Pacote de demonstração"
          value={state.fixture}
          disabled={offline}
          options={updateFixtures.map((f) => ({ value: f.id, label: f.label }))}
          onchange={(fixture) => action({ type: 'fixture', fixture: fixture as UpdateFixture })}
        />
      </div>
    </div>
  </details>
  <p class="muted">
    Fila, rascunhos, seleções e aprovações da conversa não são modificados. A prévia não autoriza
    execução nem preserva aprovações que, em uma retomada real, precisariam ser revalidadas.
  </p>
</div>
