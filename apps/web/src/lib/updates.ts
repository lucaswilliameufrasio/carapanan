export const updateFixtures = [
  {
    id: 'interface',
    label: 'Interface · sem reinício',
    runtime: false,
    verified: true,
    rollback: true,
    notes: [
      'Melhorias de layout e navegação.',
      'Compatível com o runtime da prévia; nenhuma tarefa precisa ser interrompida.',
    ],
  },
  {
    id: 'runtime',
    label: 'Runtime · reinício seguro',
    runtime: true,
    verified: true,
    rollback: true,
    notes: [
      'Melhorias no runtime (cenário fictício).',
      'Preparar agora; ativar somente em um ponto seguro, com retomada.',
    ],
  },
  {
    id: 'invalid',
    label: 'Assinatura inválida · bloquear',
    runtime: true,
    verified: false,
    rollback: false,
    notes: [
      'Pacote de teste com assinatura inválida.',
      'A versão atual permanece; não tentar instalar nem fazer fallback.',
    ],
  },
  {
    id: 'migration',
    label: 'Migração · sem rollback automático',
    runtime: true,
    verified: true,
    rollback: false,
    notes: [
      'Migração de estado incompatível com a versão anterior (fixture).',
      'Não oferecer rollback automático quando ele não for seguro.',
    ],
  },
] as const;
export type UpdateFixture = (typeof updateFixtures)[number]['id'];
export type UpdateStatus =
  | 'available'
  | 'scheduled'
  | 'verifying'
  | 'ready'
  | 'installed'
  | 'failed'
  | 'rolled-back';
export type UpdateState = {
  fixture: UpdateFixture;
  channel: 'stable' | 'beta' | 'nightly';
  status: UpdateStatus;
  busy: boolean;
  hidden: boolean;
};
export type UpdateAction =
  | { type: 'now' | 'schedule' | 'later' | 'idle' | 'verify' | 'activate' | 'rollback' }
  | { type: 'fixture'; fixture: UpdateFixture }
  | { type: 'channel'; channel: UpdateState['channel'] };
export const updateRelease = (state: UpdateState) => {
  const release = updateFixtures.find((f) => f.id === state.fixture);
  if (!release) throw new RangeError('Fixture de atualização desconhecida.');
  return release;
};
export const updateVersion = (state: UpdateState) => `0.0.1-${state.channel}-demo`;
export const updateLabels: Record<UpdateStatus, string> = {
  available: 'Nova versão disponível',
  scheduled: 'Atualização agendada',
  verifying: 'Verificação pendente',
  ready: 'Reinício pendente',
  installed: 'Atualização simulada concluída',
  failed: 'Atualização bloqueada',
  'rolled-back': 'Rollback simulado concluído',
};
export function initialUpdate(fixture: UpdateFixture = 'interface'): UpdateState {
  return { fixture, channel: 'stable', status: 'available', busy: true, hidden: false };
}
// A deterministic UX preview only: no fetch, download, signature verification,
// installation, timers, process control, persistence or changes to session state.
export function reduceUpdate(state: UpdateState, action: UpdateAction): UpdateState {
  if (action.type === 'fixture')
    return { ...initialUpdate(action.fixture), channel: state.channel };
  if (action.type === 'channel')
    return { ...initialUpdate(state.fixture), channel: action.channel };
  if (action.type === 'later') return { ...state, hidden: true };
  if (action.type === 'idle')
    return {
      ...state,
      busy: false,
      hidden: false,
      status: state.status === 'scheduled' ? 'verifying' : state.status,
    };
  if (action.type === 'now' && ['available', 'scheduled'].includes(state.status))
    return { ...state, status: 'verifying', hidden: false };
  if (action.type === 'schedule' && state.status === 'available')
    return { ...state, status: 'scheduled', hidden: false };
  if (action.type === 'verify' && state.status === 'verifying') {
    const release = updateRelease(state);
    return {
      ...state,
      status: !release.verified ? 'failed' : release.runtime ? 'ready' : 'installed',
      hidden: false,
    };
  }
  if (action.type === 'activate' && state.status === 'ready' && !state.busy)
    return { ...state, status: 'installed', hidden: false };
  if (action.type === 'rollback' && state.status === 'installed' && updateRelease(state).rollback)
    return { ...state, status: 'rolled-back', hidden: false };
  return state;
}
