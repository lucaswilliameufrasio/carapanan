import { utimes } from 'node:fs/promises';
import { expect, test } from '@playwright/test';

test.describe.configure({ mode: 'serial' });

test('Should load the dev interface without downloading the complete icon catalog', async ({
  page,
}) => {
  const icons: { url: string; bytes: number }[] = [];
  const pending: Promise<void>[] = [];
  page.on('response', (response) => {
    if (response.url().includes('lucide'))
      pending.push(
        (async () => {
          icons.push({ url: response.url(), bytes: (await response.body()).length });
        })(),
      );
  });
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  await expect(page.locator('#composer')).toBeVisible();
  await page.getByRole('button', { name: /^Buscar ação ou sessão/ }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await Promise.all(pending);
  expect(icons.length).toBeGreaterThan(0);
  expect(icons.some((icon) => /\/deps\/@lucide_svelte\.js/.test(icon.url))).toBe(false);
  expect(icons.reduce((bytes, icon) => bytes + icon.bytes, 0)).toBeLessThan(512 * 1024);
});

test('Should apply a real dev HMR update without reloading or losing the composer draft', async ({
  page,
}) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  await page.locator('#composer').fill('Preservar durante atualização em dev');
  // Listen via Vite's public HMR context. No component or session changes.
  await page.evaluate(async () => {
    const path = '/@vite/client';
    const { createHotContext } = await import(path);
    const hot = createHotContext('/__carapana_dev_test');
    document.documentElement.dataset.hmrProbe = 'waiting';
    hot.on('vite:afterUpdate', () => {
      document.documentElement.dataset.hmrProbe = 'updated';
    });
  });
  let navigations = 0;
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) navigations += 1;
  });
  // Touching the source exercises the watcher/transform/HMR pipeline without
  // overwriting user edits. Production review snapshots remain independent.
  const now = new Date();
  await utimes(new URL('../src/lib/Picker.svelte', import.meta.url), now, now);
  await expect(page.locator('html')).toHaveAttribute('data-hmr-probe', 'updated');
  await expect(page.locator('#composer')).toHaveValue('Preservar durante atualização em dev');
  expect(navigations).toBe(0);
});
