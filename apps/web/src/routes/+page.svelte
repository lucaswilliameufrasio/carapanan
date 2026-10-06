<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Activity from '@lucide/svelte/icons/activity';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import Check from '@lucide/svelte/icons/check';
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Clock from '@lucide/svelte/icons/clock';
  import Download from '@lucide/svelte/icons/download';
  import Cpu from '@lucide/svelte/icons/cpu';
  import FileDiff from '@lucide/svelte/icons/file-diff';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import ListChecks from '@lucide/svelte/icons/list-checks';
  import Menu from '@lucide/svelte/icons/menu';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import Monitor from '@lucide/svelte/icons/monitor';
  import Pause from '@lucide/svelte/icons/pause';
  import PanelLeftClose from '@lucide/svelte/icons/panel-left-close';
  import PanelLeftOpen from '@lucide/svelte/icons/panel-left-open';
  import Plus from '@lucide/svelte/icons/plus';
  import Search from '@lucide/svelte/icons/search';
  import Settings from '@lucide/svelte/icons/settings';
  import Shield from '@lucide/svelte/icons/shield';
  import Smartphone from '@lucide/svelte/icons/smartphone';
  import Square from '@lucide/svelte/icons/square';
  import Terminal from '@lucide/svelte/icons/terminal';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import WifiOff from '@lucide/svelte/icons/wifi-off';
  import X from '@lucide/svelte/icons/x';
  import Zap from '@lucide/svelte/icons/zap';
  import '@fontsource/ibm-plex-sans/latin-400.css';
  import '@fontsource/ibm-plex-sans/latin-500.css';
  import '@fontsource/ibm-plex-sans/latin-600.css';
  import '@fontsource/ibm-plex-mono/latin-400.css';
  import '@carapana/design/tokens.css';
  import '../app.css';
  import Modal from '#lib/Modal.svelte';
  import Management from '#lib/Management.svelte';
  import Picker from '#lib/Picker.svelte';
  import Updates from '#lib/Updates.svelte';
  import { initialUpdate, updateLabels, reduceUpdate } from '#lib/updates.ts';
  import { initialConfiguration, providerModels } from '#lib/configuration.ts';
  import { translate, type CopyKey } from '#lib/copy.ts';
  import { labels as l, demo } from '#lib/content.ts';
  import {
    compatible as catalogCompatible,
    connected,
    getScenario,
    initialState,
    modelName as catalogModelName,
    profileName,
    provider as catalogProvider,
    reduce,
    scenarios,
    type Action,
    type Profile,
    type PrototypeState,
    type Selection,
  } from '#lib/prototype.ts';

  let ui: PrototypeState = $state(initialState());
  let configuration = $state(initialConfiguration());
  let update = $state(initialUpdate());
  const models = $derived(providerModels(configuration.providers));
  const compatible = (selection: Selection) => catalogCompatible(selection, models);
  const modelName = (id: string) => catalogModelName(id, models);
  const provider = (id: string) => catalogProvider(id, models);
  let view = $state('session');
  let tab = $state('conversation');
  let modal = $state('');
  let locale = $state<'pt-BR' | 'en'>('pt-BR');
  let theme = $state('system');
  let systemDark = $state(true);
  let draft = $state('');
  let mobileHome = $state(false);
  let menuOpen = $state(false);
  let sidebarCollapsed = $state(false);
  let search = $state('');
  let paletteCursor = $state(0);
  const normalize = (text: string) =>
    text
      .normalize('NFD')
      .replace(/[\u0300-\u036f]/g, '')
      .toLowerCase()
      .trim();
  let fileIndex = $state(0);
  let editId = $state<number | null>(null);
  let editText = $state('');
  let editSelection = $state<Selection>({ ...initialState().selected });
  let draftProfiles = $state<Profile[]>([]);
  let providerRemember = $state(false);
  let providerWarningsOff = $state(false);
  let rememberedProviders = $state<string[]>([]);
  let sendIntervention = $state(false);
  let quotaAuto = $state(false);
  let permanentPermission = $state(false);
  let settingsScope = $state('session');
  let sessionName = $state('quintal-api');
  let eventOpen = $state(false);
  const t = (key: CopyKey) => translate(key, locale);
  const scenario = $derived(getScenario(ui));
  const online = $derived(connected(ui));
  const valid = $derived(compatible(ui.selected));
  const isDark = $derived(theme === 'dark' || (theme === 'system' && systemDark));
  const tabs = [
    { id: 'conversation', key: 'conversation' as const, icon: MessageSquare },
    { id: 'activity', key: 'activity' as const, icon: Activity },
    { id: 'plan', key: 'plan' as const, icon: ListChecks },
    { id: 'changes', key: 'changes' as const, icon: FileDiff },
    { id: 'validation', key: 'validation' as const, icon: Check },
  ];
  const navigation = [
    { id: 'mcp', key: 'mcp' as const, icon: Zap },
    { id: 'skills', key: 'skills' as const, icon: Shield },
    { id: 'providers', key: 'providers' as const, icon: Cpu },
    { id: 'devices', key: 'devices' as const, icon: Smartphone },
    { id: 'resources', key: 'resources' as const, icon: Activity },
    { id: 'doctor', key: 'doctor' as const, icon: Terminal },
    { id: 'config', key: 'config' as const, icon: ListChecks },
    { id: 'settings', key: 'settings' as const, icon: Settings },
    { id: 'updates', key: 'updates' as const, icon: Download },
  ];
  const modalTitles: Record<string, string> = {
    provider: l.providerTitle,
    queue: l.edit,
    profiles: l.customProfiles,
    output: l.fullOutput,
    checkpoint: l.checkpoint,
    pair: l.pairTitle,
    import: l.import,
    clean: l.clean,
    report: l.report,
    skill: 'Skills',
    active: l.activeSession,
    keyboard: l.keyboard,
    updates: 'Atualizações · prévia',
  };
  const paletteResults = $derived(
    [
      {
        id: 'sessions',
        label: t('sessions'),
        detail: 'Ver todas as sessões',
        icon: FolderOpen,
        run: () => openView('sessions'),
      },
      ...demo.sessions.map((s) => ({
        id: `session-${s.id}`,
        label: s.name,
        detail: `${s.title} · ${s.status}`,
        icon: MessageSquare,
        run: () => chooseSession(s.id),
      })),
      ...navigation.map((item) => ({
        id: item.id,
        label: t(item.key),
        detail: 'Abrir gerenciamento',
        icon: item.icon,
        run: () => openView(item.id),
      })),
      ...tabs.map((item) => ({
        id: `tab-${item.id}`,
        label: t(item.key),
        detail: 'Abrir na sessão atual',
        icon: item.icon,
        run: () => {
          openView('session');
          tab = item.id;
        },
      })),
      {
        id: 'pause',
        label: t('pause'),
        detail: 'Pausar a sessão simulada',
        icon: Pause,
        run: () => dispatch({ type: 'pause' }),
      },
      {
        id: 'resume',
        label: t('resume'),
        detail: 'Retomar a sessão simulada',
        icon: Activity,
        run: () => dispatch({ type: 'resume' }),
      },
      {
        id: 'profiles',
        label: l.customProfiles,
        detail: 'Editar modelos e esforços dos perfis',
        icon: Settings,
        run: openProfiles,
      },
      {
        id: 'keyboard',
        label: l.keyboard,
        detail: 'Consultar atalhos',
        icon: Terminal,
        run: () => (modal = 'keyboard'),
      },
    ].filter((item) =>
      normalize(`${item.label} ${item.detail} ${item.id}`).includes(normalize(search)),
    ),
  );
  function openPalette() {
    search = '';
    paletteCursor = 0;
    modal = 'palette';
  }
  function paletteKey(event: KeyboardEvent) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      paletteCursor =
        (paletteCursor + (event.key === 'ArrowDown' ? 1 : -1) + paletteResults.length) %
        Math.max(1, paletteResults.length);
      document
        .querySelector(`[data-palette-index="${paletteCursor}"]`)
        ?.scrollIntoView({ block: 'nearest' });
    } else if (event.key === 'Enter') {
      event.preventDefault();
      activatePalette(paletteCursor);
    }
  }
  function activatePalette(index: number) {
    const item = paletteResults[index];
    if (!item) return;
    modal = '';
    item.run();
  }

  function dispatch(action: Action) {
    ui = reduce(ui, action, models);
  }
  function selectProfile(id: string) {
    const p = ui.profiles.find((p) => p.profile === id);
    if (p)
      dispatch({
        type: 'select',
        selection: { profile: p.profile, model: p.model, variant: p.variant },
      });
  }
  function changeSelection(key: 'model' | 'variant', value: string) {
    dispatch({ type: 'select', selection: { ...ui.selected, [key]: value } });
  }
  function submit(intervene = false, approved = false) {
    if (!draft.trim() || !online || !valid) return;
    sendIntervention = intervene;
    const target = provider(ui.selected.model);
    if (
      !approved &&
      !providerWarningsOff &&
      target !== provider(ui.executing.model) &&
      !rememberedProviders.includes(target)
    ) {
      providerRemember = false;
      modal = 'provider';
      return;
    }
    dispatch({
      type: 'send',
      text: draft,
      intervene,
      approved:
        approved ||
        providerWarningsOff ||
        rememberedProviders.includes(target) ||
        target === provider(ui.executing.model),
    });
    draft = '';
  }
  function openEdit(id: number) {
    const m = ui.queue.find((m) => m.id === id);
    if (!m) return;
    editId = id;
    editText = m.text;
    editSelection = { profile: m.profile, model: m.model, variant: m.variant };
    modal = 'queue';
  }
  function openProfiles() {
    draftProfiles = structuredClone($state.snapshot(ui.profiles));
    modal = 'profiles';
  }
  function openView(id: string) {
    if (id === 'updates') {
      modal = 'updates';
      menuOpen = false;
      return;
    }
    view = id;
    mobileHome = id === 'sessions';
    menuOpen = false;
  }
  function chooseSession(id: string) {
    sessionName = demo.sessions.find((s) => s.id === id)?.name ?? 'quintal-api';
    dispatch({
      type: 'scenario',
      id: id === 'tixnow' ? 'running' : id === 'carapana' ? 'recovery' : 'approval',
    });
    view = 'session';
    mobileHome = false;
  }
  function keydown(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && ['p', 'k'].includes(event.key.toLowerCase())) {
      event.preventDefault();
      if (modal === 'palette') modal = '';
      else openPalette();
    }
    if (
      (event.altKey && event.key.toLowerCase() === 'p') ||
      (event.shiftKey && event.key === 'Tab' && (event.target as HTMLElement)?.id === 'composer')
    ) {
      event.preventDefault();
      dispatch({ type: 'cycle' });
    }
    if (
      (event.ctrlKey || event.metaKey) &&
      event.key === 'Enter' &&
      (event.target as HTMLElement)?.id === 'composer'
    ) {
      event.preventDefault();
      submit();
    }
  }
  onMount(() => {
    mobileHome = window.matchMedia('(max-width: 740px)').matches;
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    systemDark = query.matches;
    const handler = () => {
      systemDark = query.matches;
    };
    query.addEventListener('change', handler);
    return () => query.removeEventListener('change', handler);
  });
  $effect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.lang = locale;
    document.documentElement.style.colorScheme = isDark ? 'dark' : 'light';
    // Read the semantic token after the data-theme attribute has been patched;
    // otherwise browser chrome can lag one theme switch behind the interface.
    void tick().then(() => {
      const app = document.querySelector('.app');
      if (app)
        document
          .querySelector('meta[name="theme-color"]')
          ?.setAttribute('content', getComputedStyle(app).getPropertyValue('--surface').trim());
    });
  });
</script>

<svelte:head
  ><title>Carapanã — Delivery 0</title><meta
    name="description"
    content="Protótipo interativo local do Carapanã. Sem runtime ou execução real."
  /></svelte:head
>
<svelte:window onkeydown={keydown} />

<div class="app" data-theme={isDark ? 'dark' : 'light'}>
  <a class="skip-link" href="#main">{t('conversation')}</a>
  <header class="app-header">
    <button
      class="icon-button mobile-only"
      aria-label={t('sessions')}
      aria-controls="session-sidebar"
      aria-expanded={menuOpen}
      onclick={() => {
        menuOpen = !menuOpen;
      }}><Menu size={20} /></button
    >
    <button
      class="icon-button desktop-only"
      aria-label={t(sidebarCollapsed ? 'showSidebar' : 'hideSidebar')}
      title={t(sidebarCollapsed ? 'showSidebar' : 'hideSidebar')}
      aria-controls="session-sidebar"
      aria-expanded={!sidebarCollapsed}
      onclick={() => (sidebarCollapsed = !sidebarCollapsed)}
    >
      {#if sidebarCollapsed}<PanelLeftOpen size={20} aria-hidden="true" />{:else}<PanelLeftClose
          size={20}
          aria-hidden="true"
        />{/if}
    </button>
    <a class="brand" href="/" aria-label="Carapanã"
      ><img src="/mosquito.svg" alt="" width="38" height="38" aria-hidden="true" /><strong
        >carapanã</strong
      ></a
    >
    <span class="prototype-badge"
      >Delivery 0 <span class="desktop-only">/ {t('prototype')}</span></span
    >
    <div class="header-right">
      <button class="icon-button mobile-only" aria-label={t('command')} onclick={openPalette}
        ><Search size={18} /></button
      >
      <button
        class="search-button desktop-only"
        onclick={() => {
          openPalette();
        }}><Search size={15} />{t('command')}<kbd>Ctrl/⌘ P</kbd></button
      ><Picker
        class="theme-control"
        label={t('theme')}
        value={theme}
        options={[
          { value: 'system', label: t('system') },
          { value: 'dark', label: t('dark') },
          { value: 'light', label: t('light') },
        ]}
        onchange={(value) => (theme = value)}
      />
    </div>
  </header>

  {#if !update.hidden}<div class="update-banner">
      <button class="plain-button" onclick={() => (modal = 'updates')}
        ><Download size={15} /><span>{updateLabels[update.status]}</span><small>mock</small
        ><ChevronRight size={15} /></button
      >
      <button
        class="icon-button"
        aria-label="Lembrar atualização depois"
        title="Lembrar depois"
        onclick={() => (update = reduceUpdate(update, { type: 'later' }))}><X size={15} /></button
      >
    </div>{/if}

  <div class="app-body">
    <aside
      id="session-sidebar"
      class:open={menuOpen}
      class:collapsed={sidebarCollapsed}
      class="sidebar"
    >
      <div class="sidebar-heading">
        <button class="plain-button" onclick={() => openView('sessions')}
          ><FolderOpen size={16} />{t('sessions')}</button
        ><button
          class="icon-button"
          aria-label={l.newSession}
          onclick={() => {
            modal = 'active';
          }}><Plus size={18} /></button
        >
      </div>
      <div class="sidebar-section-label">
        <span class="attention-dot"></span>{t('attention')}<span class="count">1</span>
      </div>
      {#each demo.sessions.filter((s) => s.attention) as s (s.id)}<button
          class:active={view === 'session' && sessionName === s.name}
          class="session-button"
          onclick={() => chooseSession(s.id)}
          ><span class="session-line"
            ><strong>{s.name}</strong><span class="attention-symbol">!</span></span
          ><span>{s.title}</span><small>{s.status}</small></button
        >{/each}
      <div class="sidebar-section-label">{t('running')}</div>
      {#each demo.sessions.filter((s) => !s.attention) as s (s.id)}<button
          class:active={view === 'session' && sessionName === s.name}
          class="session-button"
          onclick={() => chooseSession(s.id)}
          ><span class="session-line"
            ><strong>{s.name}</strong><span class="status-dot"></span></span
          ><span>{s.title}</span><small>{s.status} · {s.time}</small></button
        >{/each}
      <nav class="navigation" aria-label={t('settings')}>
        {#each navigation as item (item.id)}<button
            aria-label={t(item.key)}
            class:active={view === item.id}
            onclick={() => openView(item.id)}
            ><item.icon size={17} />{t(item.key)}{#if item.id === 'mcp'}<small>3</small
              >{/if}</button
          >{/each}
      </nav>
      <div class="sidebar-footer">
        <Monitor size={15} /><span>Local · mock</span><button
          class="plain-button"
          onclick={() => {
            modal = 'keyboard';
          }}>?</button
        >
      </div>
    </aside>

    <main id="main">
      <div class="review-bar">
        <div class="scenario-control">
          <span>{t('scenario')}</span><Picker
            label={t('scenario')}
            value={ui.scenario}
            options={scenarios.map((s) => ({ value: s.id, label: s.label }))}
            onchange={(value) => {
              dispatch({ type: 'scenario', id: value });
              if (value === 'empty') openView('sessions');
            }}
          />
        </div>
        <button class="plain-button" onclick={() => dispatch({ type: 'scenario', id: ui.scenario })}
          >{l.reset}</button
        ><span class="desktop-only">{l.noExecution}</span>
      </div>

      {#if view === 'sessions' || (mobileHome && view === 'session')}
        <section class="session-dashboard">
          <div class="section-heading">
            <div>
              <p class="eyebrow">{t('sessions')}</p>
              <h1>{ui.scenario === 'empty' ? l.emptyTitle : t('attention')}</h1>
              <p>{ui.scenario === 'empty' ? l.emptyBody : scenario.summary}</p>
            </div>
            <button
              onclick={() => {
                modal = 'active';
              }}><Plus size={17} />{l.newSession}</button
            >
          </div>
          {#if ui.scenario === 'empty'}<div class="empty-state">
              <FolderOpen size={36} />
              <p>{l.noExecution}</p>
              <button class="primary" onclick={() => chooseSession('quintal')}>{l.openMock}</button>
            </div>{:else}
            <button
              class="attention-session"
              onclick={() => {
                mobileHome = false;
                view = 'session';
              }}
              ><span class="attention-symbol">!</span><span class="grow"
                ><strong>{sessionName}</strong><span>{scenario.summary}</span><small
                  >Auto · GPT · mock</small
                ></span
              ><ChevronRight size={20} /></button
            >
            <h2 class="dashboard-group">{t('running')}</h2>
            {#each demo.sessions.filter((s) => !s.attention) as s (s.id)}<button
                class="dashboard-session"
                onclick={() => chooseSession(s.id)}
                ><span class="status-dot"></span><span class="grow"
                  ><strong>{s.name}</strong><span>{s.title}</span><small
                    >{s.status} · {s.time}</small
                  ></span
                ><ChevronRight size={20} /></button
              >{/each}
          {/if}
        </section>
      {:else if view === 'session'}
        <header class="session-header">
          <div class="session-title">
            <button
              class="icon-button mobile-only"
              aria-label={l.backSessions}
              onclick={() => {
                mobileHome = true;
              }}><ChevronDown size={18} /></button
            ><FolderOpen size={18} />
            <h1>{sessionName}</h1>
            <span class="branch"><GitBranch size={14} />feature/refresh-token</span>
          </div>
          <div class="session-actions">
            <span class:warning={scenario.attention} class="status-label"
              ><span class="status-dot"></span>{scenario.status}</span
            ><button
              class="icon-button"
              aria-label={t('pause')}
              disabled={!online}
              onclick={() => dispatch({ type: 'pause' })}><Pause size={17} /></button
            ><button
              class="icon-button danger"
              aria-label={t('stop')}
              disabled={!online}
              onclick={() => dispatch({ type: 'stop' })}><Square size={15} /></button
            >
          </div>
        </header>
        <div
          class="execution-strip selection-theme"
          data-profile={ui.executing.profile}
          data-effort={ui.executing.variant}
        >
          <span>{t('executing')} <strong>{profileName(ui, ui.executing.profile)}</strong></span
          ><span>{modelName(ui.executing.model)} <code>{ui.executing.variant}</code></span><span
            class="grow"
          ></span><button class="plain-button" onclick={() => openView('resources')}
            ><Cpu size={14} />742 MiB</button
          ><span>{l.context} <strong>31%</strong></span><span
            class:warning={ui.scenario === 'sandbox'}
            ><Shield size={14} />{ui.scenario === 'sandbox'
              ? 'Sandbox unavailable · mock'
              : 'Sandbox · mock'}</span
          >
        </div>
        <nav class="session-tabs" aria-label={t('conversation')}>
          {#each tabs as item (item.id)}<button
              class:active={tab === item.id}
              aria-current={tab === item.id ? 'page' : undefined}
              onclick={() => {
                tab = item.id;
              }}
              ><item.icon size={16} />{t(item.key)}{#if item.id === 'changes'}<span
                  class="diff-count">+124 −38</span
                >{/if}</button
            >{/each}
        </nav>
        <div class="session-workspace">
          <section class="session-content">
            {#if !online}<div class="state-banner danger">
                <WifiOff size={20} />
                <div class="grow">
                  <strong>{t('offline')}</strong>
                  <p>{l.noExecution}</p>
                </div>
                <button onclick={() => dispatch({ type: 'scenario', id: 'recovery' })}
                  >{l.reconnect}</button
                >
              </div>{/if}
            {#if ui.pending}<div class="state-banner attention">
                <Clock size={19} />
                <div class="grow">
                  <strong>{l.pending}</strong>
                  <p>{ui.pending.text}</p>
                </div>
                <button disabled={!online} onclick={() => dispatch({ type: 'safe-step' })}
                  >{l.safeStep}</button
                >
              </div>{/if}
            {#if tab === 'conversation'}
              <div class="conversation-heading">
                <span class="eyebrow">{l.conversationTitle}</span><small>12:04 · mock</small>
              </div>
              <article class="user-message">
                <div class="avatar">L</div>
                <div>
                  <strong>Você</strong>
                  <p>{demo.user}</p>
                </div>
              </article>
              <article class="agent-message">
                <div class="agent-mark"><Terminal size={18} /></div>
                <div class="grow">
                  <strong>Carapanã</strong>
                  <div class="tool-summary">
                    {#each demo.tools as tool (tool.name)}<button
                        onclick={() => {
                          modal = 'output';
                        }}
                        ><Check size={15} class="success" /><span class="grow"
                          >{tool.name}<small>{tool.detail}</small></span
                        ><ChevronRight size={14} /></button
                      >{/each}
                  </div>
                  <p class="agent-response">{demo.response}</p>
                </div>
              </article>
            {:else if tab === 'activity'}
              <h2>{t('activity')}</h2>
              <div class="activity-list">
                {#each ui.events as entry, index (index)}<div>
                    <span class="event-index">{index + 1}</span><span>{entry}</span><small
                      >mock</small
                    >
                  </div>{/each}
              </div>
              <button
                onclick={() => {
                  modal = 'output';
                }}>{l.fullOutput}</button
              >
            {:else if tab === 'plan'}
              <h2>{t('plan')}</h2>
              <ol class="plan-list">
                {#each demo.plan as step, index (step)}<li>
                    <Check size={18} class="success" /><span>{step}</span><small
                      >{index === 4 && scenario.blocking ? 'blocked' : 'done'}</small
                    >
                  </li>{/each}
              </ol>
              <div class="inline-note">
                <ListChecks size={18} />
                <p>{l.noExecution}</p>
              </div>
            {:else if tab === 'changes'}
              <div class="section-heading">
                <div>
                  <h2>{t('changes')}</h2>
                  <p>{l.changesSummary}</p>
                </div>
                <button
                  disabled={!online}
                  onclick={() => {
                    modal = 'checkpoint';
                  }}>{l.checkpoint}</button
                >
              </div>
              <div class="file-list">
                {#each demo.files as file, index (file.path)}<button
                    class:active={index === fileIndex}
                    onclick={() => {
                      fileIndex = index;
                    }}
                    ><FileDiff size={15} /><code class="grow">{file.path}</code
                    >{#if file.previous}<small>{l.originalChanges}</small>{:else}<span
                        class="success">+{file.plus}</span
                      ><span class="danger">−{file.minus}</span>{/if}</button
                  >{/each}
              </div>
              <div class="diff-toolbar">
                <code>{demo.files[fileIndex].path}</code><span class="grow"></span><button
                  class="plain-button"
                  disabled={fileIndex === 0}
                  onclick={() => {
                    fileIndex--;
                  }}>{l.previous}</button
                ><span>{fileIndex + 1} / 4</span><button
                  class="plain-button"
                  disabled={fileIndex === 3}
                  onclick={() => {
                    fileIndex++;
                  }}>{l.next}</button
                >
              </div>
              <pre class="diff-code">{#if demo.files[fileIndex].previous}<span
                    >{l.originalChanges}</span
                  >{:else}{#each demo.diff as line, i (i)}<span
                      class:added={line.startsWith('+')}
                      class:removed={line.startsWith('-')}
                      ><span class="line-number">{i + 41}</span>{line}</span
                    >{/each}{/if}</pre>
            {:else if tab === 'validation'}
              <h2>
                {ui.scenario === 'incomplete'
                  ? scenarios.find((s) => s.id === 'incomplete')?.label
                  : l.completedTitle}
              </h2>
              <p>{l.changed}: refresh token rotation · reuse detection · integration coverage</p>
              <h3>{l.evidence}</h3>
              {#each demo.validations as command (command)}<button
                  class="validation-row"
                  onclick={() => {
                    modal = 'output';
                  }}
                  ><Check size={16} class="success" /><code class="grow">{command}</code><span
                    >0 · 4,8 s</span
                  ><ChevronRight size={15} /></button
                >{/each}
              <div class="state-banner warning">
                <Shield size={19} />
                <div>
                  <strong>{l.notValidated}</strong>
                  <p>Google OAuth callback · external provider required · mock</p>
                </div>
              </div>
            {/if}

            {#if scenario.blocking && online && !ui.pending && tab === 'conversation'}
              <section class="approval-panel" aria-label={scenario.label}>
                <div class="approval-heading">
                  <span class="attention-symbol">!</span>
                  <div>
                    <strong>{scenario.label}</strong>
                    <p>{scenario.summary}</p>
                  </div>
                  <small>mock</small>
                </div>
                {#if ui.scenario === 'approval'}<code class="approval-command"
                    >{demo.approval.command}</code
                  >
                  <dl class="approval-facts">
                    <div>
                      <dt>{l.workspace}</dt>
                      <dd>quintal-api</dd>
                    </div>
                    <div>
                      <dt>Remote</dt>
                      <dd>{demo.approval.remote}</dd>
                    </div>
                    <div>
                      <dt>{l.branch}</dt>
                      <dd>{demo.approval.branch}</dd>
                    </div>
                    <div>
                      <dt>{l.risk}</dt>
                      <dd>{demo.approval.risk}</dd>
                    </div>
                    <div>
                      <dt>{l.reason}</dt>
                      <dd>{demo.approval.reason}</dd>
                    </div>
                  </dl>
                  <div class="actions">
                    <button
                      class="primary"
                      onclick={() => dispatch({ type: 'approve', allow: true })}>{l.once}</button
                    ><button onclick={() => dispatch({ type: 'approve', allow: true })}
                      >{l.task}</button
                    ><button
                      class="plain-button danger"
                      onclick={() => dispatch({ type: 'approve', allow: false })}>{l.deny}</button
                    >
                  </div>
                {:else if ui.scenario === 'quota'}<p>{l.noFallback}</p>
                  <label class="setting-row"
                    ><input
                      type="checkbox"
                      checked={quotaAuto || ui.executing.profile === 'yolo'}
                      disabled={ui.executing.profile === 'yolo'}
                      onchange={() => {
                        quotaAuto = !quotaAuto;
                      }}
                    /><span>{l.quotaChoice}</span></label
                  >{#if quotaAuto || ui.executing.profile === 'yolo'}<small>{l.quotaWaiting}</small
                    >{/if}
                {:else if ui.scenario === 'sandbox'}<p>{l.sudoRule}</p>
                  <button class="primary" onclick={() => dispatch({ type: 'approve', allow: true })}
                    >{l.allowNoSandbox}</button
                  ><button onclick={() => dispatch({ type: 'approve', allow: false })}
                    >{l.deny}</button
                  >
                {:else if ui.scenario === 'trust'}<pre>+ MCP: postgres · project\n+ Hook: validation.finish\n  sudo: blocked</pre>
                  <button class="primary" onclick={() => dispatch({ type: 'approve', allow: true })}
                    >{l.trustWorkspace}</button
                  >
                {:else if ui.scenario === 'shared'}<p>{l.scopePath}</p>
                  <strong>Corrigir login · Auto · 12 min · mock</strong>
                  <div class="actions">
                    <button onclick={() => dispatch({ type: 'approve', allow: true })}
                      >{l.useWorktree}</button
                    ><button onclick={() => dispatch({ type: 'approve', allow: true })}
                      >{l.shareDirectory}</button
                    >
                  </div>
                {:else if ui.scenario === 'secret'}<p>{l.secretRule}</p>
                  <code>.env.example</code>
                  <div class="actions">
                    <button onclick={() => dispatch({ type: 'approve', allow: true })}
                      >{l.localUse}</button
                    ><button onclick={() => dispatch({ type: 'approve', allow: false })}
                      >{l.deny}</button
                    >
                  </div>
                {:else if ui.scenario === 'model' || ui.scenario === 'variant'}<p>
                    {l.selectRequired}
                  </p>
                  <button onclick={() => openView('providers')}>{t('providers')}</button>
                {:else if ui.scenario === 'auth'}<button onclick={() => openView('providers')}
                    >{l.connect}</button
                  >
                {:else if ui.scenario === 'conflict'}<button
                    onclick={() => {
                      tab = 'changes';
                    }}>{l.reviewChanges}</button
                  >
                {:else if ui.scenario === 'incomplete'}<button
                    onclick={() => {
                      tab = 'validation';
                    }}>{t('validation')}</button
                  ><button onclick={() => dispatch({ type: 'scenario', id: 'running' })}
                    >{l.confirm}</button
                  >
                {:else if ui.scenario === 'loop'}<p>3 + 3 · Yolo stops · mock</p>
                  <button
                    onclick={() => {
                      tab = 'activity';
                    }}>{t('activity')}</button
                  >
                {:else if ui.scenario === 'recovery'}<button
                    class="primary"
                    onclick={() => dispatch({ type: 'resume' })}>{t('resume')}</button
                  >
                {:else}<p>{l.noProvider}</p>{/if}
              </section>
            {:else if ['mcp', 'resources', 'compaction'].includes(ui.scenario)}<div
                class="state-banner warning"
              >
                <Activity size={18} />
                <p>{scenario.summary}</p>
                <button
                  class="plain-button"
                  onclick={() => openView(ui.scenario === 'mcp' ? 'mcp' : 'resources')}
                  >{l.manage}</button
                >
              </div>{/if}
            <div class="simulation-controls">
              <span>Mock</span><button
                class="plain-button"
                disabled={!online || scenario.blocking}
                onclick={() => dispatch({ type: 'finish' })}>{t('simulateFinish')}</button
              ><button
                class="plain-button"
                onclick={() => {
                  eventOpen = !eventOpen;
                }}>{t('activity')}</button
              >
            </div>
            {#if eventOpen}<pre>{ui.events.join('\n')}</pre>{/if}
          </section>
          <aside class="task-rail">
            <div class="rail-heading"><ListChecks size={16} />{t('plan')}<span>5 / 5</span></div>
            <ol>
              {#each demo.plan as step (step)}<li>
                  <Check size={14} class="success" /><span>{step}</span>
                </li>{/each}
            </ol>
            <div class="rail-heading">
              <FileDiff size={16} />{t('changes')}<span>+124 −38</span>
            </div>
            {#each demo.files as file (file.path)}<button
                class="rail-file"
                onclick={() => {
                  tab = 'changes';
                  fileIndex = demo.files.indexOf(file);
                }}
                ><code>{file.path.split('/').at(-1)}</code><span
                  >{file.previous ? 'prévio' : `+${file.plus}`}</span
                ></button
              >{/each}
            <div class="rail-note">
              <Shield size={17} />
              <p>{l.originalChanges}</p>
            </div>
          </aside>
        </div>

        <section class="composer-area">
          {#if ui.queue.length}<details class="queue">
              <summary
                ><span class="queue-count">{ui.queue.length}</span>{t('queue')}<span class="grow"
                ></span><span class="desktop-only">{ui.queue[0].text.slice(0, 48)}</span
                ><ChevronDown size={15} /></summary
              >
              <div class="queue-items">
                {#each ui.queue as message, index (message.id)}<div class="queue-item">
                    <span>{index + 1}</span>
                    <div class="grow">
                      <p>{message.text}</p>
                      <small
                        class="selection-theme queue-selection"
                        data-profile={message.profile}
                        data-effort={message.variant}
                        >{profileName(ui, message.profile)} · {modelName(message.model)} · {message.variant}
                        · {message.origin}</small
                      >
                    </div>
                    <div class="queue-actions">
                      <button
                        class="icon-button"
                        aria-label={`${l.up} ${index + 1}`}
                        disabled={!online || index === 0}
                        onclick={() => dispatch({ type: 'move', id: message.id, direction: -1 })}
                        ><ArrowUp size={14} /></button
                      ><button
                        class="icon-button"
                        aria-label={`${l.down} ${index + 1}`}
                        disabled={!online || index === ui.queue.length - 1}
                        onclick={() => dispatch({ type: 'move', id: message.id, direction: 1 })}
                        ><ArrowDown size={14} /></button
                      ><button disabled={!online} onclick={() => openEdit(message.id)}
                        >{l.edit}</button
                      ><button
                        class="icon-button"
                        aria-label={`${l.remove} ${index + 1}`}
                        disabled={!online}
                        onclick={() => dispatch({ type: 'remove', id: message.id })}
                        ><Trash2 size={14} /></button
                      >
                    </div>
                  </div>{/each}
              </div>
            </details>{/if}
          <div
            class="composer selection-theme"
            data-profile={ui.selected.profile}
            data-effort={ui.selected.variant}
          >
            <label class="sr-only" for="composer">{t('next')}</label><textarea
              id="composer"
              bind:value={draft}
              placeholder={t('placeholder')}
              rows="2"></textarea>
            <div class="composer-toolbar">
              <div class="composer-selects">
                <Picker
                  label={l.profile}
                  value={ui.selected.profile}
                  options={ui.profiles.map((p) => ({
                    value: p.profile,
                    label: p.name,
                    detail:
                      p.mode === 'plan'
                        ? 'Planejar sem implementar'
                        : p.profile === 'ask'
                          ? 'Perguntar antes de agir'
                          : p.profile === 'yolo'
                            ? 'Maior autonomia; limites de segurança permanecem'
                            : 'Executar conforme as permissões',
                  }))}
                  onchange={selectProfile}
                />
                <Picker
                  label={l.model}
                  value={ui.selected.model}
                  options={models.map((m) => ({
                    value: m.id,
                    label: m.name,
                    detail: `${m.provider}${m.enabled === false ? ' · desabilitado' : ''}`,
                  }))}
                  onchange={(value) => changeSelection('model', value)}
                />
                <Picker
                  label={l.variant}
                  value={ui.selected.variant}
                  options={['default', 'low', 'high'].map((v) => ({
                    value: v,
                    label: v,
                    detail: models.find((m) => m.id === ui.selected.model)?.variants.includes(v)
                      ? 'Compatível com o modelo selecionado'
                      : 'Incompatível; escolha outro modelo ou esforço',
                  }))}
                  onchange={(value) => changeSelection('variant', value)}
                />
                <button class="icon-button" aria-label={l.customProfiles} onclick={openProfiles}
                  ><Settings size={15} /></button
                >
              </div>
              <div class="composer-send">
                <button disabled={!online || !draft.trim() || !valid} onclick={() => submit(true)}
                  >{t('intervene')}</button
                ><button
                  class="primary send-button"
                  disabled={!online || !draft.trim() || !valid}
                  onclick={() => submit()}
                  aria-label={t('send')}><ArrowUp size={18} /><span>{t('send')}</span></button
                >
              </div>
            </div>
          </div>
          <div class="composer-footnote">
            <span
              >{t('next')}: {profileName(ui, ui.selected.profile)} · {modelName(ui.selected.model)} ·
              {ui.selected.variant}</span
            ><span>{valid ? 'Ctrl/⌘ Enter · Alt P' : l.selectRequired}</span>
          </div>
        </section>
      {:else}
        <section class="management-content">
          {#if view === 'settings'}<div class="section-heading">
              <div>
                <h1>{t('settings')}</h1>
                <p>{l.savedMock}</p>
              </div>
            </div>
            <div class="setting-row">
              <span>{t('theme')}</span><Picker
                label={t('theme')}
                value={theme}
                options={[
                  { value: 'system', label: t('system') },
                  { value: 'dark', label: t('dark') },
                  { value: 'light', label: t('light') },
                ]}
                onchange={(value) => (theme = value)}
              />
            </div>
            <div class="setting-row">
              <span>{l.language}</span><Picker
                label={l.language}
                value={locale}
                options={[
                  { value: 'pt-BR', label: 'Português brasileiro' },
                  { value: 'en', label: 'English · partial' },
                ]}
                onchange={(value) => (locale = value as 'pt-BR' | 'en')}
              />
            </div>
            {#if locale === 'en'}<p>{l.englishPartial}</p>{/if}<label class="setting-row"
              ><span>{l.globalProvider}</span><input
                type="checkbox"
                disabled={!online}
                bind:checked={providerWarningsOff}
              /></label
            >
            <div class="setting-row">
              <span>{l.scope}</span><Picker
                label={l.scope}
                value={settingsScope}
                options={[
                  { value: 'session', label: l.sessionScope },
                  { value: 'project', label: l.projectScope },
                  { value: 'user', label: l.userScope },
                ]}
                onchange={(value) => (settingsScope = value)}
              />
            </div>
            <div class="actions">
              <button
                disabled={!online}
                onclick={() => dispatch({ type: 'notice', text: l.savedMock })}>{t('save')}</button
              ><button onclick={openProfiles}>{l.customProfiles}</button>
            </div>
            <section class="detail-block">
              <h3>{l.permission}</h3>
              <p>{l.secretRule}</p>
              <p>{l.sudoRule}</p>
              <label class="setting-row"
                ><input
                  type="checkbox"
                  disabled={!online}
                  bind:checked={permanentPermission}
                /><span>delete · /tmp/carapana-demo/**</span></label
              ><button
                disabled={!online || !permanentPermission}
                onclick={() => {
                  permanentPermission = false;
                  dispatch({ type: 'notice', text: l.savedMock });
                }}>{l.revoke}</button
              ><button
                disabled={!online}
                onclick={() => {
                  permanentPermission = true;
                  dispatch({ type: 'notice', text: l.savedMock });
                }}>{l.always}</button
              >
            </section>{:else}<Management
              {view}
              {configuration}
              onconfigure={(value) => (configuration = value)}
              degraded={ui.scenario === 'mcp'}
              offline={!online}
              onmodal={(id) => {
                modal = id;
              }}
              onnotice={(text) => dispatch({ type: 'notice', text })}
            />{/if}
        </section>
      {/if}
      <div class="notice" role="status" aria-live="polite">{ui.notice}</div>
    </main>
  </div>

  <Modal
    open={modal !== ''}
    title={modal === 'palette' ? t('command') : (modalTitles[modal] ?? l.confirmation)}
    closeLabel={t('close')}
    onclose={() => {
      modal = '';
    }}
  >
    {#if modal === 'palette'}<input
        class="palette-search"
        aria-label={t('command')}
        bind:value={search}
        data-dialog-focus
        name="command-search"
        autocomplete="off"
        placeholder="Buscar sessão ou ação…"
        oninput={() => (paletteCursor = 0)}
        onkeydown={paletteKey}
      />
      <div class="palette-results">
        {#each paletteResults as item, index (item.id)}<button
            class:palette-active={index === paletteCursor}
            data-palette-index={index}
            onclick={() => {
              activatePalette(index);
            }}
            ><item.icon size={17} /><span class="grow"
              ><strong>{item.label}</strong><small>{item.detail}</small></span
            ><ChevronRight size={15} /></button
          >{/each}
        {#if !paletteResults.length}<p class="palette-empty">
            Nenhum resultado para “{search}”. Tente o nome da sessão ou da ação.
          </p>{/if}
      </div>
      <p class="palette-hint">↑/↓ navegar · Enter abrir · Esc fechar</p>
    {:else if modal === 'updates'}<Updates
        state={update}
        offline={!online}
        onchange={(value) => (update = value)}
      />
    {:else if modal === 'provider'}<p>{l.providerBody}</p>
      <code>{provider(ui.executing.model)} → {provider(ui.selected.model)}</code><label
        class="setting-row"
        ><input type="checkbox" bind:checked={providerRemember} /><span>{l.rememberProvider}</span
        ></label
      >
      <p class="muted">{l.noProvider}</p>
      <div class="actions">
        <button
          class="primary"
          disabled={!online}
          onclick={() => {
            if (providerRemember)
              rememberedProviders = [...rememberedProviders, provider(ui.selected.model)];
            modal = '';
            submit(sendIntervention, true);
          }}>{l.confirm}</button
        ><button
          onclick={() => {
            modal = '';
          }}>{t('cancel')}</button
        >
      </div>
    {:else if modal === 'queue'}<textarea aria-label={t('next')} bind:value={editText} rows="4"
      ></textarea>
      <div class="edit-selects">
        <div>
          <span>{l.profile}</span><Picker
            label={l.profile}
            value={editSelection.profile}
            options={ui.profiles.map((p) => ({ value: p.profile, label: p.name }))}
            onchange={(value) => (editSelection.profile = value)}
          />
        </div>
        <div>
          <span>{l.model}</span><Picker
            label={l.model}
            value={editSelection.model}
            options={models.map((m) => ({ value: m.id, label: m.name }))}
            onchange={(value) => (editSelection.model = value)}
          />
        </div>
        <div>
          <span>{l.variant}</span><Picker
            label={l.variant}
            value={editSelection.variant}
            options={['default', 'low', 'high'].map((v) => ({ value: v, label: v }))}
            onchange={(value) => (editSelection.variant = value)}
          />
        </div>
      </div>
      <button
        class="primary"
        disabled={!online || !compatible(editSelection)}
        onclick={() => {
          if (editId !== null) {
            const exists = ui.queue.some((m) => m.id === editId);
            dispatch({ type: 'edit', id: editId, text: editText, selection: editSelection });
            if (!exists) draft = editText;
          }
          modal = '';
        }}>{t('save')}</button
      >
    {:else if modal === 'profiles'}<div>
        {#each draftProfiles as p, index (p.profile)}<div class="profile-editor">
            <label>{l.profileName}<input bind:value={p.name} /></label>
            <div>
              <span>{l.model}</span><Picker
                label={`${l.model} · ${p.name}`}
                value={p.model}
                options={models.map((m) => ({ value: m.id, label: m.name }))}
                onchange={(value) => (p.model = value)}
              />
            </div>
            <div>
              <span>{l.variant}</span><Picker
                label={`${l.variant} · ${p.name}`}
                value={p.variant}
                options={['default', 'low', 'high'].map((v) => ({ value: v, label: v }))}
                onchange={(value) => (p.variant = value)}
              />
            </div>
            <button
              class="icon-button"
              aria-label={`${l.up} ${p.name}`}
              disabled={index === 0}
              onclick={() => {
                const reordered = [...draftProfiles];
                [reordered[index - 1], reordered[index]] = [reordered[index], reordered[index - 1]];
                draftProfiles = reordered;
              }}><ArrowUp size={15} /></button
            ><button
              class="icon-button"
              aria-label={`${l.remove} ${p.name}`}
              disabled={draftProfiles.length === 1}
              onclick={() => {
                draftProfiles = draftProfiles.filter((item) => item.profile !== p.profile);
              }}><X size={15} /></button
            >
          </div>{/each}
      </div>
      <div class="actions">
        <button
          onclick={() => {
            draftProfiles = [
              ...draftProfiles,
              {
                profile: `custom-${ui.nextId}-${draftProfiles.length}`,
                name: l.newProfile,
                model: 'gpt-mock',
                variant: 'default',
                mode: 'execute',
              },
            ];
          }}><Plus size={15} />{l.add}</button
        ><button
          class="primary"
          onclick={() => {
            dispatch({ type: 'profiles', profiles: draftProfiles });
            if (!ui.profiles.some((p) => p.profile === ui.selected.profile))
              selectProfile(ui.profiles[0].profile);
            modal = '';
            dispatch({ type: 'notice', text: l.savedMock });
          }}>{t('save')}</button
        ><button onclick={() => dispatch({ type: 'notice', text: l.defaultsMock })}
          >{l.saveDefault}</button
        >
      </div>
    {:else if modal === 'output'}<pre>{demo.output}</pre>
    {:else if modal === 'checkpoint'}<p>{l.originalChanges}</p>
      <pre class="diff-code">{#each demo.diff as line (line)}<span
            class:added={line.startsWith('+')}
            class:removed={line.startsWith('-')}>{line}</span
          >{/each}</pre>
      <p>{l.noExecution}</p>
      <button
        class="primary"
        disabled={!online}
        onclick={() => {
          modal = '';
          dispatch({ type: 'notice', text: l.restored });
        }}>{l.confirm}</button
      >
    {:else if modal === 'pair'}<div class="pair-code">
        <Smartphone size={42} /><code>{l.deviceCode}</code>
      </div>
      <p>{l.pairBody}</p>
      <p>{l.permissionsMobile}</p>
      <p class="muted">{l.adminDisabled}</p>
      <button
        class="primary"
        disabled={!online}
        onclick={() => {
          modal = '';
          dispatch({ type: 'notice', text: l.deviceAdded });
        }}>{l.connect}</button
      >
    {:else if modal === 'active'}<p>{l.activeBody}</p>
      <div class="actions">
        <button
          class="primary"
          onclick={() => {
            chooseSession('quintal');
            modal = '';
          }}>{l.attach}</button
        ><button
          onclick={() => {
            chooseSession('carapana');
            modal = '';
          }}>{l.startSeparate}</button
        ><button
          onclick={() => {
            openView('sessions');
            modal = '';
          }}>{t('sessions')}</button
        >
      </div>
    {:else if modal === 'keyboard'}<p>{l.keyboardBody}</p>
    {:else}<p>
        {modal === 'import'
          ? l.importBody
          : modal === 'clean'
            ? l.cleanBody
            : modal === 'report'
              ? l.reportBody
              : l.newTools}
      </p>
      <p class="muted">{l.noExecution}</p>
      <button
        disabled={!online}
        onclick={() => {
          modal = '';
          dispatch({ type: 'notice', text: l.savedMock });
        }}>{l.confirm}</button
      >{/if}
  </Modal>
</div>
