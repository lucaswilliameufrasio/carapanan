import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { loadEnv } from 'vite';
import { defineConfig } from 'vitest/config';
import { serverConfig } from './server-config.ts';
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, '.', ['HOST', 'PORT']);
  return {
    plugins: [tailwindcss(), sveltekit({ adapter: adapter() })],
    server: serverConfig(env),
    preview: serverConfig(env, 4173),
    test: { include: ['src/**/*.test.ts'] },
  };
});
