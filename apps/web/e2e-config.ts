import { serverConfig } from './server-config';

export function e2eServers(env: Record<string, string | undefined>) {
  const preview = serverConfig({ PORT: env.CARAPANA_E2E_PREVIEW_PORT }, 4173);
  const dev = serverConfig({ PORT: env.CARAPANA_E2E_DEV_PORT }, 4174);
  if (preview.port === dev.port) {
    throw new Error('E2E preview e dev devem usar portas diferentes.');
  }
  return {
    preview: {
      command: `pnpm preview --host 127.0.0.1 --port ${preview.port} --strictPort`,
      url: `http://127.0.0.1:${preview.port}`,
    },
    dev: {
      command: `pnpm dev --host 127.0.0.1 --port ${dev.port} --strictPort`,
      url: `http://127.0.0.1:${dev.port}`,
    },
  };
}
