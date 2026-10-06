import { describe, expect, it } from 'vitest';
import { initialUpdate, reduceUpdate } from './updates';

describe('update UX preview', () => {
  it('Should prepare and apply an interface fixture without restarting a busy session', () => {
    const initial = initialUpdate();
    const prepared = reduceUpdate(initial, { type: 'now' });
    expect(prepared.status).toBe('verifying');
    const applied = reduceUpdate(prepared, { type: 'verify' });
    expect(applied.status).toBe('installed');
    expect(applied.busy).toBe(true);
    expect(reduceUpdate(applied, { type: 'rollback' }).status).toBe('rolled-back');
  });
  it('Should block runtime activation until the preview is idle and require an explicit restart', () => {
    let state = reduceUpdate(initialUpdate('runtime'), { type: 'now' });
    state = reduceUpdate(state, { type: 'verify' });
    expect(state.status).toBe('ready');
    expect(reduceUpdate(state, { type: 'activate' })).toEqual(state);
    state = reduceUpdate(state, { type: 'idle' });
    expect(state.status).toBe('ready');
    expect(reduceUpdate(state, { type: 'activate' }).status).toBe('installed');
  });
  it('Should defer notification without losing scheduling and never apply an invalid fixture', () => {
    const scheduled = reduceUpdate(initialUpdate('runtime'), { type: 'schedule' });
    const reminded = reduceUpdate(scheduled, { type: 'later' });
    expect(reminded.status).toBe('scheduled');
    expect(reminded.hidden).toBe(true);
    const finished = reduceUpdate(reminded, { type: 'idle' });
    expect(finished.status).toBe('verifying');
    expect(finished.hidden).toBe(false);
    const invalid = reduceUpdate(reduceUpdate(initialUpdate('invalid'), { type: 'now' }), {
      type: 'verify',
    });
    expect(invalid.status).toBe('failed');
    expect(reduceUpdate(invalid, { type: 'activate' })).toEqual(invalid);
    expect(reduceUpdate(invalid, { type: 'rollback' })).toEqual(invalid);
  });
  it('Should refuse incompatible rollback and ignore out of order actions', () => {
    const initial = initialUpdate('migration');
    expect(reduceUpdate(initial, { type: 'verify' })).toEqual(initial);
    const ready = reduceUpdate(reduceUpdate(initial, { type: 'now' }), { type: 'verify' });
    const installed = reduceUpdate(reduceUpdate(ready, { type: 'idle' }), { type: 'activate' });
    expect(installed.status).toBe('installed');
    expect(reduceUpdate(installed, { type: 'rollback' })).toEqual(installed);
    expect(reduceUpdate(installed, { type: 'now' })).toEqual(installed);
  });
});
