import fixtures from '../../../../fixtures/scenarios.json';

export const scenarios = fixtures;
export type Scenario = (typeof scenarios)[number];
export type Selection = { profile: string; model: string; variant: string };
export type Profile = Selection & { name: string; mode: 'plan' | 'execute' };
export type QueuedMessage = Selection & {
  id: number;
  text: string;
  origin: string;
  providerApproved: boolean;
};
export type PrototypeState = {
  scenario: string;
  profiles: Profile[];
  selected: Selection;
  executing: Selection;
  queue: QueuedMessage[];
  pending: QueuedMessage | null;
  nextId: number;
  approvalResolved: boolean;
  notice: string;
  events: string[];
};
export const models = [
  {
    id: 'gpt-mock',
    name: 'GPT · mock',
    provider: 'OpenAI · mock',
    variants: ['default', 'low', 'high'],
  },
  {
    id: 'claude-mock',
    name: 'Claude · mock',
    provider: 'Anthropic · mock',
    variants: ['default', 'low'],
  },
  { id: 'local-mock', name: 'Local · mock', provider: 'Local · mock', variants: ['default'] },
];
export const modelName = (id: string) => models.find((m) => m.id === id)?.name ?? id;
export const provider = (id: string) => models.find((m) => m.id === id)?.provider ?? 'Indisponível';
export const profileName = (state: PrototypeState, id: string) =>
  state.profiles.find((p) => p.profile === id)?.name ?? id;
export const getScenario = (state: PrototypeState) =>
  scenarios.find((s) => s.id === state.scenario) ?? scenarios[0];
export const connected = (state: PrototypeState) => state.scenario !== 'offline';
export const compatible = (selection: Selection) =>
  models.some((m) => m.id === selection.model && m.variants.includes(selection.variant));

export function initialState(scenario = 'approval'): PrototypeState {
  return {
    scenario,
    profiles: [
      { profile: 'plan', name: 'Planejar', mode: 'plan', model: 'claude-mock', variant: 'low' },
      { profile: 'ask', name: 'Perguntar', mode: 'execute', model: 'gpt-mock', variant: 'default' },
      { profile: 'auto', name: 'Auto', mode: 'execute', model: 'gpt-mock', variant: 'high' },
      { profile: 'yolo', name: 'Yolo', mode: 'execute', model: 'gpt-mock', variant: 'default' },
    ],
    selected: { profile: 'ask', model: 'gpt-mock', variant: 'default' },
    executing: { profile: 'auto', model: 'gpt-mock', variant: 'high' },
    queue: [
      {
        id: 1,
        text: 'Também verifica a detecção de reutilização do token.',
        profile: 'auto',
        model: 'gpt-mock',
        variant: 'high',
        origin: 'TUI · MacBook',
        providerApproved: true,
      },
      {
        id: 2,
        text: 'Planeja a migração dos tokens existentes, sem implementar.',
        profile: 'plan',
        model: 'claude-mock',
        variant: 'low',
        origin: 'PWA · celular',
        providerApproved: false,
      },
    ],
    nextId: 3,
    pending: null,
    approvalResolved: false,
    notice: '',
    events: ['Sessão recuperada do cenário simulado.', '183 testes passaram · evidência simulada.'],
  };
}

export type Action =
  | { type: 'scenario'; id: string }
  | { type: 'select'; selection: Selection }
  | { type: 'cycle' }
  | { type: 'profiles'; profiles: Profile[] }
  | { type: 'send'; text: string; intervene?: boolean; approved?: boolean }
  | { type: 'edit'; id: number; text: string; selection: Selection }
  | { type: 'remove'; id: number }
  | { type: 'move'; id: number; direction: -1 | 1 }
  | { type: 'safe-step' }
  | { type: 'finish' }
  | { type: 'approve'; allow: boolean }
  | { type: 'resume' }
  | { type: 'pause' }
  | { type: 'stop' }
  | { type: 'notice'; text: string };

// An in-memory UI reducer only. No timers, filesystem, processes, network or persistence.
export function reduce(state: PrototypeState, action: Action): PrototypeState {
  const event = (next: PrototypeState, text: string) => ({
    ...next,
    notice: text,
    events: [...next.events, text],
  });
  if (action.type === 'scenario') return initialState(action.id);
  if (action.type === 'notice') return { ...state, notice: action.text };
  if (action.type === 'profiles') return { ...state, profiles: action.profiles };
  if (action.type === 'select') {
    return {
      ...state,
      selected: { ...action.selection },
      profiles: state.profiles.map((p) =>
        p.profile === action.selection.profile
          ? { ...p, model: action.selection.model, variant: action.selection.variant }
          : p,
      ),
    };
  }
  if (action.type === 'cycle') {
    const index = state.profiles.findIndex((p) => p.profile === state.selected.profile);
    const p = state.profiles[(index + 1) % state.profiles.length];
    return reduce(state, {
      type: 'select',
      selection: { profile: p.profile, model: p.model, variant: p.variant },
    });
  }
  if (!connected(state)) return { ...state, notice: 'Desconectado: nenhuma ação foi enviada.' };
  if (action.type === 'send') {
    if (!action.text.trim() || !compatible(state.selected))
      return { ...state, notice: 'Selecione um modelo e uma variante compatíveis.' };
    const message = {
      ...state.selected,
      id: state.nextId,
      text: action.text.trim(),
      origin: 'Web · este dispositivo',
      providerApproved:
        action.approved ?? provider(state.selected.model) === provider(state.executing.model),
    };
    if (action.intervene)
      return event(
        { ...state, pending: message, nextId: state.nextId + 1, approvalResolved: true },
        'Intervenção pendente. Approval anterior invalidado; aguardando etapa segura.',
      );
    return event(
      { ...state, queue: [...state.queue, message], nextId: state.nextId + 1 },
      'Mensagem enfileirada com a seleção atual.',
    );
  }
  if (action.type === 'edit') {
    if (!state.queue.some((m) => m.id === action.id))
      return {
        ...state,
        notice: 'A mensagem já começou. Seu texto foi preservado para novo envio.',
      };
    if (!action.text.trim() || !compatible(action.selection))
      return { ...state, notice: 'Texto e seleção compatível são necessários.' };
    return event(
      {
        ...state,
        queue: state.queue.map((m) =>
          m.id === action.id
            ? {
                ...m,
                text: action.text.trim(),
                ...action.selection,
                providerApproved: m.model === action.selection.model && m.providerApproved,
              }
            : m,
        ),
      },
      'Mensagem da fila atualizada explicitamente.',
    );
  }
  if (action.type === 'remove')
    return { ...state, queue: state.queue.filter((m) => m.id !== action.id) };
  if (action.type === 'move') {
    const queue = [...state.queue];
    const i = queue.findIndex((m) => m.id === action.id);
    const j = i + action.direction;
    if (i < 0 || j < 0 || j >= queue.length) return state;
    [queue[i], queue[j]] = [queue[j], queue[i]];
    return { ...state, queue };
  }
  if (action.type === 'safe-step') {
    if (!state.pending) return state;
    return event(
      { ...state, executing: { ...state.pending }, pending: null, scenario: 'running' },
      'Etapa segura simulada. Intervenção aplicada; novas ações serão reavaliadas.',
    );
  }
  if (action.type === 'finish') {
    if (getScenario(state).blocking)
      return { ...state, notice: 'A fila espera a resolução do bloqueio atual.' };
    const [next, ...queue] = state.queue;
    if (next && !next.providerApproved && provider(next.model) !== provider(state.executing.model))
      return {
        ...state,
        notice:
          'Confirme o compartilhamento com o provider da próxima mensagem antes de continuar.',
      };
    return event(
      {
        ...state,
        queue,
        executing: next ? { ...next } : state.executing,
        scenario: next ? 'running' : 'completed',
      },
      next ? `Próxima mensagem iniciada: ${next.text}` : 'Concluído com evidências simuladas.',
    );
  }
  if (action.type === 'approve') {
    if (state.approvalResolved)
      return { ...state, notice: 'already_resolved: decisão já resolvida ou invalidada.' };
    return event(
      { ...state, approvalResolved: true, scenario: action.allow ? 'running' : 'recovery' },
      action.allow
        ? 'Autorização simulada registrada. Nenhum comando executado.'
        : 'Ação negada. Alterações preservadas.',
    );
  }
  if (action.type === 'resume') {
    if (
      [
        'quota',
        'auth',
        'model',
        'variant',
        'loop',
        'incomplete',
        'sandbox',
        'trust',
        'secret',
        'shared',
        'conflict',
      ].includes(state.scenario)
    )
      return { ...state, notice: 'Resolva o bloqueio específico antes de retomar.' };
    return event(
      { ...state, scenario: 'running' },
      'Retomada simulada: arquivos e permissões serão revalidados.',
    );
  }
  if (action.type === 'pause')
    return event({ ...state, scenario: 'recovery' }, 'Pausado. Fila e alterações preservadas.');
  if (action.type === 'stop')
    return event(
      { ...state, scenario: 'recovery', pending: null },
      'Parado. Processos temporários encerrados; serviços anteriores e arquivos preservados (mock).',
    );
  return state;
}
