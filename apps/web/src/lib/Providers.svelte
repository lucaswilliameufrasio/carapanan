<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import Server from '@lucide/svelte/icons/server';
  import Pencil from '@lucide/svelte/icons/pencil';
  import X from '@lucide/svelte/icons/x';
  import Modal from './Modal.svelte';
  import Picker from './Picker.svelte';
  import {
    integrations,
    protocols,
    newProvider,
    providerErrors,
    type ProviderConfig,
  } from './configuration';
  let {
    providers,
    offline,
    onchange,
    onnotice,
  }: {
    providers: ProviderConfig[];
    offline: boolean;
    onchange: (providers: ProviderConfig[]) => void;
    onnotice: (text: string) => void;
  } = $props();
  let open = $state(false);
  let editing = $state(false);
  let draft = $state(newProvider('provider-4'));
  let errors = $state<string[]>([]);
  let modelSequence = 2;
  const formId = $props.id();
  function edit(provider?: ProviderConfig) {
    const next =
      1 + Math.max(3, ...providers.map((p) => Number(p.id.match(/^provider-(\d+)$/)?.[1] ?? 0)));
    draft = provider ? structuredClone($state.snapshot(provider)) : newProvider(`provider-${next}`);
    editing = !!provider;
    modelSequence =
      1 + Math.max(0, ...draft.models.map((m) => Number(m.id.match(/-model-(\d+)$/)?.[1] ?? 0)));
    errors = [];
    open = true;
  }
  function integration(id: string) {
    const preset = integrations.find((p) => p.id === id)!;
    draft.integration = id;
    draft.protocol = preset.protocol;
    draft.endpoint = preset.endpoint;
    draft.auth = preset.auth;
  }
  function save() {
    if (offline) return;
    errors = providerErrors(draft, providers);
    if (errors.length) return;
    const saved = structuredClone($state.snapshot(draft));
    saved.name = saved.name.trim();
    if (saved.auth !== 'reference') saved.credentialRef = '';
    onchange(
      editing ? providers.map((p) => (p.id === saved.id ? saved : p)) : [...providers, saved],
    );
    open = false;
    onnotice('Cadastro salvo apenas neste protótipo. Nenhuma conexão ou autenticação realizada.');
  }
  function variant(index: number, value: string, checked: boolean) {
    const model = draft.models[index];
    model.variants = checked
      ? [...model.variants, value]
      : model.variants.filter((v) => v !== value);
  }
</script>

<div class="section-heading">
  <div>
    <h2>Providers</h2>
    <p>Integrações e modelos disponíveis neste protótipo.</p>
  </div>
  <button class="primary" disabled={offline} onclick={() => edit()}
    ><Plus size={16} />Adicionar provider</button
  >
</div>
<aside class="inline-note">
  <Server size={18} />
  <p>Cadastro em memória, sem rede, tokens ou login. Recarregar a página restaura os exemplos.</p>
</aside>
<div class="provider-list">
  {#each providers as provider (provider.id)}
    <section class="provider-card" data-provider-id={provider.id}>
      <div class="management-row">
        <Server size={20} />
        <div class="grow">
          <h3>{provider.name}</h3>
          <small
            >{provider.kind === 'mapped' ? 'Mapeado' : 'Custom'} · {provider.protocol === 'chatgpt'
              ? 'ChatGPT'
              : protocols.find((p) => p.value === provider.protocol)?.label} · mock</small
          >
        </div>
        <span class:success={provider.enabled}
          >{provider.enabled ? 'Disponível no mock' : 'Desabilitado'}</span
        >
        <button
          disabled={offline}
          aria-label={`Editar ${provider.name}`}
          onclick={() => edit(provider)}><Pencil size={15} />Editar</button
        >
        <button
          disabled={offline}
          aria-label={`${provider.enabled ? 'Desabilitar' : 'Habilitar'} ${provider.name}`}
          onclick={() => {
            onchange(
              providers.map((p) => (p.id === provider.id ? { ...p, enabled: !p.enabled } : p)),
            );
            onnotice(
              'Disponibilidade alterada no mock; seleções em execução e na fila permanecem.',
            );
          }}>{provider.enabled ? 'Desabilitar' : 'Habilitar'}</button
        >
      </div>
      <div class="provider-models">
        {#each provider.models as model (model.id)}
          <div class="provider-model">
            <div><strong>{model.name}</strong><code>{model.upstreamId}</code></div>
            <span>{model.variants.join(' / ')}</span><small
              >{[
                model.streaming ? 'Streaming' : '',
                model.tools ? 'Tools' : '',
                model.reasoning ? 'Reasoning' : '',
              ]
                .filter(Boolean)
                .join(' · ') || 'Sem capacidades adicionais'}</small
            >
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>
<section class="detail-block">
  <h3>ChatGPT · assinatura</h3>
  <p>
    A assinatura é a prioridade do plano. Este cadastro não verifica acesso, quota nem
    compatibilidade real.
  </p>
  <p>
    Nenhum fallback pago ou troca silenciosa de modelo. Compartilhar contexto com outro provider
    exige confirmação.
  </p>
</section>

<Modal
  {open}
  title={editing ? 'Editar provider' : 'Adicionar provider'}
  closeLabel="Fechar"
  onclose={() => (open = false)}
>
  <form
    class="configuration-form"
    novalidate
    onsubmit={(event) => {
      event.preventDefault();
      save();
    }}
  >
    <p class="muted">
      Explore o cadastro. Não cole chaves ou tokens: somente referências fictícias.
    </p>
    <div class="choice-group" role="group" aria-label="Tipo de provider">
      <button
        type="button"
        aria-pressed={draft.kind === 'mapped'}
        onclick={() => {
          draft.kind = 'mapped';
          integration(draft.integration);
        }}>Mapeado<small>Integração conhecida</small></button
      >
      <button
        type="button"
        aria-pressed={draft.kind === 'custom'}
        onclick={() => {
          draft.kind = 'custom';
          if (draft.protocol === 'chatgpt') {
            draft.protocol = 'openai';
            draft.auth = 'reference';
          }
          if (!draft.endpoint) draft.endpoint = 'https://gateway.example.invalid/v1';
        }}>Custom<small>Seu endpoint e protocolo</small></button
      >
    </div>
    <label
      >Nome do provider<input
        data-dialog-focus
        bind:value={draft.name}
        maxlength="100"
        autocomplete="off"
      /></label
    >
    {#if draft.kind === 'mapped'}
      <div class="form-field">
        <span>Integração</span><Picker
          label="Integração"
          value={draft.integration}
          options={integrations.map((p) => ({ value: p.id, label: p.name }))}
          onchange={integration}
        />
      </div>
      <div class="integration-summary">
        <strong>Endpoint da integração</strong><code
          >{draft.endpoint || 'Gerenciado pela integração ChatGPT'}</code
        ><small>Configuração de exemplo; nenhum adapter ou login é executado.</small>
      </div>
    {:else}
      <div class="form-field">
        <span>Protocolo</span><Picker
          label="Protocolo"
          value={draft.protocol}
          options={protocols}
          onchange={(value) => (draft.protocol = value)}
        />
      </div>
      <label
        >URL base<input
          aria-label="URL base"
          aria-describedby={`${formId}-endpoint-note`}
          type="url"
          bind:value={draft.endpoint}
          placeholder="https://gateway.example.invalid/v1"
          autocomplete="off"
        /><small id={`${formId}-endpoint-note`}>HTTPS, ou HTTP local. Sem credenciais na URL.</small
        ></label
      >
    {/if}
    {#if draft.kind === 'custom'}<div class="form-field">
        <span>Autenticação</span><Picker
          label="Autenticação"
          value={draft.auth}
          options={[
            { value: 'reference', label: 'Referência de credencial' },
            { value: 'none', label: 'Sem autenticação · local/mock' },
          ]}
          onchange={(value) => (draft.auth = value)}
        />
      </div>{/if}
    {#if draft.auth === 'reference'}<label
        >Referência da credencial<input
          aria-label="Referência da credencial"
          aria-describedby={`${formId}-credential-note`}
          bind:value={draft.credentialRef}
          placeholder="cred:meu-provider"
          autocomplete="off"
          spellcheck="false"
        /><small id={`${formId}-credential-note`}
          >É um nome, não o valor secreto. Não acessa nenhum cofre.</small
        ></label
      >
    {:else if draft.auth === 'oauth'}<p class="inline-note">
        Login oficial via OAuth, quando implementado. Neste mock não há autenticação.
      </p>{/if}
    <div class="section-heading">
      <div>
        <h3>Modelos</h3>
        <p>Identificadores e capacidades declarados por você; não verificados.</p>
      </div>
      <button
        type="button"
        onclick={() =>
          (draft.models = [
            ...draft.models,
            {
              id: `${draft.id}-model-${modelSequence++}`,
              upstreamId: '',
              name: '',
              variants: ['default'],
              streaming: true,
              tools: true,
              reasoning: false,
            },
          ])}><Plus size={15} />Adicionar modelo</button
      >
    </div>
    {#each draft.models as model, index (model.id)}
      <fieldset class="model-editor">
        <legend>Modelo {index + 1}</legend>
        <div class="field-grid">
          <label
            >Nome de exibição<input
              aria-label={`Nome do modelo ${index + 1}`}
              bind:value={model.name}
              maxlength="100"
            /></label
          >
          <label
            >Identificador no provider<input
              aria-label={`Identificador do modelo ${index + 1}`}
              bind:value={model.upstreamId}
              placeholder="org/modelo"
              maxlength="200"
              spellcheck="false"
            /></label
          >
        </div>
        <div class="check-group" role="group" aria-label={`Capacidades do modelo ${index + 1}`}>
          <label><input type="checkbox" bind:checked={model.streaming} />Streaming</label>
          <label><input type="checkbox" bind:checked={model.tools} />Tool calls</label>
          <label><input type="checkbox" bind:checked={model.reasoning} />Reasoning</label>
        </div>
        <div class="check-group" role="group" aria-label={`Variantes do modelo ${index + 1}`}>
          <span>Variantes</span>
          {#each ['default', 'low', 'high'] as value (value)}<label
              ><input
                type="checkbox"
                checked={model.variants.includes(value)}
                onchange={(event) => variant(index, value, event.currentTarget.checked)}
              />{value}</label
            >{/each}
        </div>
        <button
          type="button"
          class="plain-button"
          disabled={draft.models.length === 1}
          aria-label={`Remover modelo ${index + 1}`}
          onclick={() => (draft.models = draft.models.filter((m) => m.id !== model.id))}
          ><X size={15} />Remover modelo</button
        >
      </fieldset>
    {/each}
    {#if errors.length}<div class="form-errors" role="alert">
        <strong>Revise o cadastro</strong>
        <ul>
          {#each errors as error (error)}<li>{error}</li>{/each}
        </ul>
      </div>{/if}
    <p class="muted">
      Salvar não muda a seleção em execução ou na fila. Os modelos entram nos pickers da próxima
      mensagem.
    </p>
    <div class="actions">
      <button class="primary" type="submit" disabled={offline}>Salvar provider no mock</button
      ><button type="button" onclick={() => (open = false)}>Cancelar</button>
    </div>
  </form>
</Modal>
