import { describe, expect, it } from 'vitest';
import { serverConfig } from '../../server-config';

describe('serverConfig', () => {
  it('Should keep loopback and the default development and preview ports', () => {
    expect(serverConfig({})).toEqual({ host: '127.0.0.1', port: 5173, strictPort: true });
    expect(serverConfig({}, 4173).port).toBe(4173);
    expect(serverConfig({ HOST: '', PORT: '' }).port).toBe(5173);
    expect(serverConfig({ HOST: '', PORT: '' }).host).toBe('127.0.0.1');
  });

  it('Should accept a network host and custom port for development and preview', () => {
    for (const host of ['0.0.0.0', '::', '100.64.0.1', 'localhost']) {
      expect(serverConfig({ HOST: host, PORT: '5180' })).toEqual({
        host,
        port: 5180,
        strictPort: true,
      });
      expect(serverConfig({ HOST: host, PORT: '5180' }, 4173).port).toBe(5180);
    }
    expect(serverConfig({ PORT: '1' }).port).toBe(1);
    expect(serverConfig({ PORT: '65535' }).port).toBe(65535);
  });

  it('Should reject invalid ports rather than silently choosing a different address', () => {
    for (const port of ['0', '-1', '65536', 'abc', '5173.5', '5e3', ' 5173 ']) {
      expect(() => serverConfig({ PORT: port })).toThrow(
        'PORT deve ser um inteiro entre 1 e 65535.',
      );
    }
  });
});
