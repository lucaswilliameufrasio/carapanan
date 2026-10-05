import { sveltekit } from '@sveltejs/kit/vite';
import adapter from '@sveltejs/adapter-static';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import { loadEnv } from 'vite';
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
