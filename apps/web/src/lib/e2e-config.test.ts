import { describe, expect, it } from 'vitest';
import { e2eServers } from '../../e2e-config';

describe('e2eServers', () => {
  it('Should default to distinct strict loopback servers', () => {
    expect(e2eServers({})).toEqual({
      preview: {
        command: 'pnpm preview --host 127.0.0.1 --port 4173 --strictPort',
        url: 'http://127.0.0.1:4173',
      },
      dev: {
        command: 'pnpm dev --host 127.0.0.1 --port 4174 --strictPort',
        url: 'http://127.0.0.1:4174',
      },
    });
  });

  it('Should use explicit ports consistently without accepting an ambient host', () => {
    expect(
      e2eServers({
        CARAPANA_E2E_PREVIEW_PORT: '34173',
        CARAPANA_E2E_DEV_PORT: '34174',
        HOST: '0.0.0.0',
        PORT: '80',
      }),
    ).toEqual({
      preview: {
        command: 'pnpm preview --host 127.0.0.1 --port 34173 --strictPort',
        url: 'http://127.0.0.1:34173',
      },
      dev: {
        command: 'pnpm dev --host 127.0.0.1 --port 34174 --strictPort',
        url: 'http://127.0.0.1:34174',
      },
    });
  });

  it('Should reject equal or malformed ports before building shell commands', () => {
    expect(() => e2eServers({ CARAPANA_E2E_PREVIEW_PORT: '4174' })).toThrow('portas diferentes');
    for (const value of ['0', '65536', '-1', '1.5', 'NaN', '4173; echo unsafe']) {
      for (const key of ['CARAPANA_E2E_PREVIEW_PORT', 'CARAPANA_E2E_DEV_PORT']) {
        expect(() => e2eServers({ [key]: value })).toThrow('PORT');
      }
    }
  });
});
