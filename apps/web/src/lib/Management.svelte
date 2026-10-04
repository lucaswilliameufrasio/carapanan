<script lang="ts">
  import {
    Check,
    Shield,
    Smartphone,
    Server,
    RotateCw,
    Plus,
    HardDrive,
    ExternalLink,
  } from '@lucide/svelte';
  import { labels as l, demo } from './content';
  import { models } from './prototype';
  let {
    view,
    degraded,
    offline,
    onmodal,
    onnotice,
  }: {
    view: string;
    degraded: boolean;
    offline: boolean;
    onmodal: (id: string) => void;
    onnotice: (text: string) => void;
  } = $props();
  let mcpStates = $state<Record<string, string>>({
    'ai-memory': 'healthy',
    playwright: 'disabled',
    postgres: 'healthy',
  });
  let skills = $state(demo.skills.slice(0, 4));
  let paired = $state(false);
  let deviceAdmin = $state(false);
  let keep = $state(true);
  let connectedProviders = $state(['OpenAI · mock']);
  let memoryLimit = $state('2 GiB');
  let configKey = $state('model.primary');
  let permissions = $state(false);
  const update = (text: string) => {
    if (!offline) onnotice(text);
  };
</script>

{#if view === 'mcp'}
  <div class="section-heading">
    <div>
      <h2>MCP</h2>
      <p>{l.newTools}</p>
    </div>
    <button disabled={offline} onclick={() => onmodal('import')}
      ><Plus size={16} />{l.import}</button
    >
  </div>
  <div class="list-table">
    {#each demo.mcp as server (server.name)}
      {@const status =
        server.name === 'ai-memory' && degraded ? 'degraded' : mcpStates[server.name]}
      <div class="management-row">
        <Server size={19} />
        <div class="grow">
          <strong>{server.name}</strong><small
            >{server.scope === 'pinned' ? l.pinned : l.onDemand}</small
          >
        </div>
        <span class:warning={status === 'degraded'} class:success={status === 'healthy'}
          >{status === 'healthy'
            ? l.healthy
            : status === 'disabled'
              ? l.disabled
              : l.degraded}</span
        ><button
          class="icon-button"
          aria-label={`${l.restart} ${server.name}`}
          disabled={offline}
          onclick={() => {
            mcpStates[server.name] = 'healthy';
            update(l.savedMock);
          }}><RotateCw size={16} /></button
        ><button
          disabled={offline}
          onclick={() => {
            mcpStates[server.name] = status === 'disabled' ? 'healthy' : 'disabled';
            update(l.savedMock);
          }}>{status === 'disabled' ? l.enable : l.disable}</button
        ><button
          disabled={offline}
          onclick={() => {
            permissions = !permissions;
          }}>{l.inspect}</button
        >
      </div>
    {/each}
  </div>
  {#if permissions}<section class="detail-block">
      <h3>ai-memory · {l.permission}</h3>
      <p>{l.readMemory}</p>
      <p>{l.writeMemory}</p>
      <p>{l.deleteMemory}</p>
      <button disabled={offline} onclick={() => onmodal('output')}>{l.fullOutput}</button>
    </section>{/if}
  <aside class="inline-note">
    <Shield size={18} />
    <p>{l.secretRule}</p>
  </aside>
{:else if view === 'skills'}
  <div class="section-heading">
    <div>
      <h2>Skills</h2>
      <p>{l.newTools}</p>
    </div>
    <button disabled={offline} onclick={() => onmodal('skill')}><Plus size={16} />{l.add}</button>
  </div>
  <div class="list-table">
    {#each demo.skills as skill (skill)}<label class="management-row"
        ><Shield size={18} /><span class="grow"
          ><strong>{skill}</strong><small
            >{skill === 'security' ? 'builtin · mandatory' : 'builtin · optional'}</small
          ></span
        ><input
          type="checkbox"
          aria-label={`${l.enable} ${skill}`}
          checked={skill === 'security' || skills.includes(skill)}
          disabled={offline || skill === 'security'}
          onchange={() => {
            skills = skills.includes(skill)
              ? skills.filter((s) => s !== skill)
              : [...skills, skill];
            update(l.savedMock);
          }}
        /></label
      >{/each}
  </div>
{:else if view === 'providers'}
  <div class="section-heading">
    <div>
      <h2>{l.provider}s</h2>
      <p>{l.noProvider}</p>
    </div>
  </div>
  <div class="list-table">
    {#each models as model (model.id)}<div class="management-row">
        <Server size={19} />
        <div class="grow">
          <strong>{model.provider}</strong><small>{model.name} · {model.variants.join(', ')}</small>
        </div>
        <span class:success={connectedProviders.includes(model.provider)}
          >{connectedProviders.includes(model.provider) ? l.healthy : l.disabled}</span
        ><button
          disabled={offline}
          onclick={() => {
            connectedProviders = connectedProviders.includes(model.provider)
              ? connectedProviders.filter((p) => p !== model.provider)
              : [...connectedProviders, model.provider];
            update(l.savedMock);
          }}>{connectedProviders.includes(model.provider) ? l.disable : l.connect}</button
        >
      </div>{/each}
  </div>
  <section class="detail-block">
    <h3>ChatGPT · Pro · mock</h3>
    <p>{l.noFallback}</p>
    <div class="meter"><span style="width: 24%"></span></div>
    <small>24% · fixture</small>
  </section>
{:else if view === 'devices'}
  <div class="section-heading">
    <div>
      <h2>{l.devices ?? 'Dispositivos'}</h2>
      <p>{l.adminDisabled}</p>
    </div>
    <button disabled={offline} onclick={() => onmodal('pair')}><Plus size={16} />{l.pair}</button>
  </div>
  <div class="management-row">
    <Smartphone size={20} />
    <div class="grow"><strong>MacBook · TUI</strong><small>Local IPC · mock</small></div>
    <span class="success">{l.healthy}</span>
  </div>
  <div class="management-row">
    <Smartphone size={20} />
    <div class="grow"><strong>PWA · celular</strong><small>{l.permissionsMobile}</small></div>
    <button
      disabled={offline}
      onclick={() => {
        paired = !paired;
        update(paired ? l.deviceAdded : l.savedMock);
      }}>{paired ? l.revoke : l.connect}</button
    >
  </div>
  <label class="setting-row"
    ><span>{l.adminDisabled}</span><input
      type="checkbox"
      checked={deviceAdmin}
      disabled={offline}
      onchange={() => {
        deviceAdmin = !deviceAdmin;
        update(l.savedMock);
      }}
    /></label
  >
  <section class="detail-block">
    <h3>Remote attach · mock</h3>
    <code>https://host.example.invalid</code>
    <p>Tailscale / NetBird · TLS · app authentication</p>
    <button disabled={offline} onclick={() => onmodal('pair')}
      ><ExternalLink size={16} />{l.connect}</button
    >
  </section>
{:else if view === 'resources'}
  <div class="section-heading">
    <div>
      <h2>{l.limits}</h2>
      <p>{l.budgets}</p>
    </div>
  </div>
  <div class="resource-grid">
    <section class="detail-block">
      <h3>{l.memory}</h3>
      <div class="large-metric">742 <small>MiB</small></div>
      <p>/ {memoryLimit} · mock</p>
      <div class="meter"><span style="width: 36%"></span></div>
      <label class="setting-row"
        ><span>{l.limits}</span><select bind:value={memoryLimit} disabled={offline}
          ><option>1 GiB</option><option>2 GiB</option><option>4 GiB</option></select
        ></label
      >
    </section>
    <section class="detail-block">
      <h3>{l.cpu}</h3>
      <div class="large-metric">1 <small>/ 2</small></div>
      <p>3 active · 7 hibernated · mock</p>
      <p>{l.budgets}</p>
    </section>
  </div>
  <section class="detail-block">
    <h3><HardDrive size={18} />{l.disk}</h3>
    {#each [['Sessions', '1,8 GB'], ['Artifacts', '3,2 GB'], ['Logs', '280 MB'], ['Cache', '640 MB']] as row (row[0])}<div
        class="metric-row"
      >
        <span>{row[0]}</span><code>{row[1]}</code>
      </div>{/each}<button disabled={offline} onclick={() => onmodal('clean')}>{l.clean}</button>
  </section>
  <section class="detail-block">
    <h3>{l.processes}</h3>
    <label class="setting-row"
      ><code>pnpm dev · mock</code><span>{l.keepRunning}</span><input
        type="checkbox"
        checked={keep}
        disabled={offline}
        onchange={() => {
          keep = !keep;
          update(l.savedMock);
        }}
      /></label
    >
    <p>{l.noExecution}</p>
  </section>
{:else if view === 'doctor'}
  <div class="section-heading">
    <div>
      <h2>{l.doctorIntro}</h2>
      <p>{l.noExecution}</p>
    </div>
  </div>
  <div class="list-table">
    {#each demo.doctor as item (item)}<div class="management-row">
        <Check size={17} class="success" /><strong class="grow">{item}</strong><small
          >mock · OK</small
        >
      </div>{/each}
  </div>
  <div class="actions">
    <button disabled={offline} onclick={() => onmodal('report')}>{l.report}</button><button
      disabled={offline}
      onclick={() => update(l.savedMock)}>{l.fix}</button
    >
  </div>
{:else if view === 'config'}
  <div class="section-heading">
    <div>
      <h2>{l.effective}</h2>
      <p>{l.noSecretConfig}</p>
    </div>
  </div>
  <label class="setting-row"
    ><span>{l.explain}</span><select bind:value={configKey}
      ><option>model.primary</option><option>agent.mode</option><option
        >resources.max_concurrent_agents</option
      ></select
    ></label
  >
  <section class="detail-block">
    <h3><code>{configKey}</code></h3>
    <div class="metric-row">
      <span>{l.effective}</span><code
        >{configKey === 'model.primary'
          ? 'gpt-mock'
          : configKey === 'agent.mode'
            ? 'ask'
            : '2'}</code
      >
    </div>
    <div class="metric-row"><span>{l.source}</span><code>user · mock</code></div>
    <pre>defaults → system → user → project → env → CLI → session</pre>
    <p>{l.sessionScope} / {l.projectScope} / {l.userScope}</p>
  </section>
{/if}
