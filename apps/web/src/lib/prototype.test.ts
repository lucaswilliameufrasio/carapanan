import { describe, expect, it } from 'vitest';
import { compatible, initialState, reduce, scenarios } from './prototype';

describe('mock interaction state', () => {
  it('Should provide unique scenarios with explicit blocking states', () => {
    expect(new Set(scenarios.map((s) => s.id)).size).toBe(scenarios.length);
    expect(scenarios.length).toBeGreaterThanOrEqual(20);
  });
  it('Should preserve the executing profile when cycling the composer selection', () => {
    const state = initialState();
    const next = reduce(state, { type: 'cycle' });
    expect(next.executing).toEqual(state.executing);
    expect(next.selected.profile).toBe('auto');
    expect(next.queue).toEqual(state.queue);
  });
  it('Should remember each profile selection independently', () => {
    let s = initialState();
    s = reduce(s, {
      type: 'select',
      selection: { profile: 'ask', model: 'local-mock', variant: 'default' },
    });
    s = reduce(s, { type: 'cycle' });
    expect(s.selected.model).toBe('gpt-mock');
    s = reduce(s, { type: 'cycle' });
    s = reduce(s, { type: 'cycle' });
    s = reduce(s, { type: 'cycle' });
    expect(s.selected.model).toBe('local-mock');
  });
  it('Should preserve queued selections after later composer changes', () => {
    let s = reduce(initialState(), { type: 'send', text: 'Nova tarefa' });
    s = reduce(s, { type: 'cycle' });
    expect(s.queue.at(-1)?.profile).toBe('ask');
    expect(s.selected.profile).toBe('auto');
  });
  it('Should allow explicit editing, reordering and removing queued messages', () => {
    let s = reduce(initialState(), {
      type: 'edit',
      id: 1,
      text: 'Texto novo',
      selection: { profile: 'plan', model: 'claude-mock', variant: 'low' },
    });
    expect(s.queue[0].text).toBe('Texto novo');
    s = reduce(s, { type: 'move', id: 1, direction: 1 });
    expect(s.queue[1].id).toBe(1);
    s = reduce(s, { type: 'remove', id: 1 });
    expect(s.queue).toHaveLength(1);
  });
  it('Should invalidate approvals and apply intervention only at a safe step', () => {
    const before = initialState();
    const pending = reduce(before, { type: 'send', text: 'Pare e planeje', intervene: true });
    expect(pending.executing).toEqual(before.executing);
    expect(pending.approvalResolved).toBe(true);
    expect(reduce(pending, { type: 'approve', allow: true }).scenario).toBe('approval');
    const applied = reduce(pending, { type: 'safe-step' });
    expect(applied.executing.profile).toBe('ask');
    expect(applied.pending).toBeNull();
  });
  it('Should block offline actions without changing the queue', () => {
    const s = initialState('offline');
    expect(reduce(s, { type: 'send', text: 'não enviar' }).queue).toEqual(s.queue);
    expect(reduce(s, { type: 'approve', allow: true }).scenario).toBe('offline');
  });
  it('Should block advancement on incomplete validation, quota and doom loops', () => {
    for (const scenario of ['incomplete', 'quota', 'loop']) {
      const s = initialState(scenario);
      expect(reduce(s, { type: 'finish' }).queue).toEqual(s.queue);
      expect(reduce(s, { type: 'resume' }).scenario).toBe(scenario);
    }
  });
  it('Should preserve the queue on recovery and stop', () => {
    const s = initialState('recovery');
    expect(reduce(s, { type: 'stop' }).queue).toEqual(s.queue);
    expect(reduce(s, { type: 'resume' }).scenario).toBe('running');
  });
  it('Should reject incompatible variants without silently replacing them', () => {
    const s = reduce(initialState(), {
      type: 'select',
      selection: { profile: 'ask', model: 'local-mock', variant: 'high' },
    });
    expect(compatible(s.selected)).toBe(false);
    expect(reduce(s, { type: 'send', text: 'não enviar' }).queue).toEqual(s.queue);
    expect(s.selected.variant).toBe('high');
  });
  it('Should preserve a late edit as a recoverable notice', () => {
    const s = initialState();
    const next = reduce(s, {
      type: 'edit',
      id: 99,
      text: 'Rascunho preservado',
      selection: s.selected,
    });
    expect(next.notice).toContain('preservado');
    expect(next.queue).toEqual(s.queue);
  });
  it('Should resolve an approval only once', () => {
    const s = reduce(initialState(), { type: 'approve', allow: true });
    expect(reduce(s, { type: 'approve', allow: false }).notice).toContain('already_resolved');
  });
  it('Should advance using the queued message selection instead of the composer', () => {
    const s = reduce(initialState('running'), { type: 'finish' });
    expect(s.executing.profile).toBe('auto');
    expect(s.executing.variant).toBe('high');
    expect(s.selected.profile).toBe('ask');
    expect(s.queue).toHaveLength(1);
  });
});
