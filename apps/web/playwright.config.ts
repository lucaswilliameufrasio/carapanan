import { defineConfig, devices } from '@playwright/test';
export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  workers: 2,
  use: { baseURL: 'http://127.0.0.1:4173', trace: 'retain-on-failure' },
  projects: [
    { name: 'desktop', testMatch: 'prototype.spec.ts', use: { ...devices['Desktop Chrome'] } },
    { name: 'mobile', testMatch: 'prototype.spec.ts', use: { ...devices['Pixel 7'] } },
    {
      name: 'dev',
      // These tests exercise one shared dev server and its source watcher.
      workers: 1,
      testMatch: 'dev.spec.ts',
      use: { ...devices['Desktop Chrome'], baseURL: 'http://127.0.0.1:4174' },
    },
  ],
  webServer: [
    {
      command: 'pnpm preview --port 4173',
      url: 'http://127.0.0.1:4173',
      reuseExistingServer: false,
    },
    { command: 'pnpm dev --port 4174', url: 'http://127.0.0.1:4174', reuseExistingServer: false },
  ],
});
