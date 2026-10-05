import { describe, expect, it } from 'vitest';
import {
  initialConfiguration,
  newProvider,
  providerErrors,
  providerModels,
  resourceErrors,
  resourcePreview,
} from './configuration';
import { initialState, reduce } from './prototype';

describe('mock configuration', () => {
  it('Should adapt previews within operator caps without modifying the saved settings', () => {
    const { resources } = initialConfiguration();
    const original = structuredClone(resources);
    const mac = resourcePreview(resources, 'mac', false);
    const small = resourcePreview(resources, 'tirion', true);
    expect(small.agents).toBeLessThan(mac.agents);
    for (const machine of ['mac', 'tirion', 'ryzen'] as const) {
      const preview = resourcePreview(resources, machine, false);
      expect(preview.agents).toBeLessThanOrEqual(resources.agents);
      expect(preview.hard).toBeLessThanOrEqual(resources.hard);
      expect(preview.soft).toBeLessThanOrEqual(preview.hard);
    }
    expect(resources).toEqual(original);
    expect(resourcePreview({ ...resources, policy: 'manual' }, 'tirion', true).agents).toBe(
      resources.agents,
    );
  });
  it('Should reject invalid memory ordering and fractional or unbounded concurrency', () => {
    const { resources } = initialConfiguration();
    expect(resourceErrors(resources)).toEqual([]);
    for (const patch of [
      { soft: 5, hard: 4 },
      { soft: 0 },
      { hard: Infinity },
      { agents: 1.5 },
      { sessions: 0 },
      { processes: NaN },
    ]) {
      expect(resourceErrors({ ...resources, ...patch }).length).toBeGreaterThan(0);
    }
    expect(resourceErrors({ ...resources, soft: 6, hard: 12, agents: 7, sessions: 10 })).toEqual(
      [],
    );
  });
  it('Should accept a custom model catalog and preserve execution queue and provider consent', () => {
    const config = initialConfiguration();
    const custom = newProvider('custom-4');
    custom.name = 'Meu gateway';
    custom.kind = 'custom';
    custom.endpoint = 'https://gateway.example.invalid/v1';
    custom.models[0].upstreamId = 'meu-modelo';
    expect(providerErrors(custom, config.providers)).toEqual([]);
    const catalog = providerModels([...config.providers, custom]);
    const before = initialState();
    const selected = { ...before.selected, model: catalog.at(-1)!.id };
    const changed = reduce(before, { type: 'select', selection: selected }, catalog);
    expect(changed.executing).toEqual(before.executing);
    expect(changed.queue).toEqual(before.queue);
    const sent = reduce(changed, { type: 'send', text: 'Revisar' }, catalog);
    expect(sent.queue.at(-1)?.providerApproved).toBe(false);
    expect(sent.queue.at(-1)?.model).toBe(selected.model);
    const disabled = providerModels([...config.providers, { ...custom, enabled: false }]);
    expect(reduce(changed, { type: 'send', text: 'Revisar' }, disabled).queue).toEqual(
      before.queue,
    );
    const ready = { ...sent, scenario: 'running', queue: [sent.queue.at(-1)!] };
    const stopped = reduce(ready, { type: 'finish' }, disabled);
    expect(stopped.queue).toEqual(ready.queue);
    expect(stopped.executing).toEqual(before.executing);
    expect(stopped.notice).toContain('indisponível');
  });
  it('Should reject duplicate providers models credentials in URLs and secret values instead of references', () => {
    const config = initialConfiguration();
    const valid = newProvider('new-4');
    expect(providerErrors(valid, config.providers)).toEqual([]);
    const invalid = [
      { ...valid, name: config.providers[0].name },
      { ...valid, models: [] },
      { ...valid, models: [valid.models[0], valid.models[0]] },
      { ...valid, credentialRef: 'sk-secret-not-a-reference' },
      { ...valid, kind: 'custom' as const, endpoint: 'https://user:password@example.invalid/v1' },
      { ...valid, kind: 'custom' as const, endpoint: 'https://example.invalid/v1?key=secret' },
      { ...valid, kind: 'custom' as const, endpoint: 'http://remote.example.invalid/v1' },
      { ...valid, kind: 'custom' as const, endpoint: 'javascript:alert(1)' },
      { ...valid, models: [{ ...valid.models[0], variants: [] }] },
      { ...valid, models: [{ ...valid.models[0], variants: ['high'], reasoning: false }] },
    ];
    for (const candidate of invalid)
      expect(providerErrors(candidate, config.providers).length).toBeGreaterThan(0);
    expect(
      providerErrors(
        { ...valid, kind: 'custom', endpoint: 'http://127.0.0.1:11434/v1' },
        config.providers,
      ),
    ).toEqual([]);
  });
});
