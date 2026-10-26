import { defineConfig, devices } from '@playwright/test';
import { e2eServers } from './e2e-config';

const servers = e2eServers(process.env);
export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  workers: 2,
  snapshotPathTemplate: `{testDir}/{testFilePath}-snapshots/{arg}-{projectName}-${process.env.CARAPANA_SNAPSHOT_ENV ?? '{platform}'}{ext}`,
  use: { baseURL: servers.preview.url, trace: 'retain-on-failure' },
  projects: [
    { name: 'desktop', testMatch: 'prototype.spec.ts', use: { ...devices['Desktop Chrome'] } },
    { name: 'mobile', testMatch: 'prototype.spec.ts', use: { ...devices['Pixel 7'] } },
    {
      name: 'dev',
      // These tests exercise one shared dev server and its source watcher.
      workers: 1,
      testMatch: 'dev.spec.ts',
      use: { ...devices['Desktop Chrome'], baseURL: servers.dev.url },
    },
  ],
  webServer: [
    {
      ...servers.preview,
      reuseExistingServer: false,
    },
    { ...servers.dev, reuseExistingServer: false },
  ],
});
