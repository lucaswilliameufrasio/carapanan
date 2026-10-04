import { expect, test, type Page } from '@playwright/test';

async function detail(page: Page) {
  await page.goto('/');
  if (await page.locator('.attention-session').isVisible())
    await page.locator('.attention-session').click();
  await expect(page.locator('#composer')).toBeVisible();
}
async function scenario(page: Page, id: string) {
  await page.getByLabel('Cenário de revisão').selectOption(id);
}
async function nav(page: Page, name: string) {
  if (await page.locator('.mobile-only').first().isVisible())
    await page.locator('.app-header').getByRole('button', { name: 'Sessões', exact: true }).click();
  await page.locator('.navigation').getByRole('button', { name, exact: true }).click();
}

test('Should open mobile in attention and desktop in the active session without console errors', async ({
  page,
  isMobile,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByText('Delivery 0', { exact: false }).first()).toBeVisible();
  if (isMobile)
    await expect(page.getByRole('heading', { name: 'Precisa de você', exact: true })).toBeVisible();
  else await expect(page.locator('.execution-strip')).toContainText('Executando Auto');
  expect(errors).toEqual([]);
});

test('Should keep the execution unchanged and snapshot the queued profile', async ({ page }) => {
  await detail(page);
  await page.getByLabel('Perfil', { exact: true }).selectOption('auto');
  await expect(page.locator('.execution-strip')).toContainText('Auto');
  await page.locator('#composer').fill('Verifica também o logout.');
  await page.getByRole('button', { name: 'Enviar para a fila', exact: true }).click();
  await page.getByLabel('Perfil', { exact: true }).selectOption('ask');
  await page.locator('.queue summary').click();
  await expect(page.locator('.queue-item').last()).toContainText('Auto');
  await expect(page.locator('.queue-item').last()).toContainText('high');
  await expect(page.locator('.composer-footnote')).toContainText('Perguntar');
});

test('Should edit text and model selection on a queued message explicitly', async ({ page }) => {
  await detail(page);
  await page.locator('.queue summary').click();
  await page
    .locator('.queue-items')
    .getByRole('button', { name: 'Editar', exact: true })
    .first()
    .click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('Só planeja a migração.');
  await dialog.getByLabel('Perfil', { exact: true }).selectOption('plan');
  await dialog.getByLabel('Modelo', { exact: true }).selectOption('claude-mock');
  await dialog.getByLabel('Variante', { exact: true }).selectOption('low');
  await dialog.getByRole('button', { name: 'Salvar', exact: true }).click();
  await expect(page.locator('.queue-item').first()).toContainText('Só planeja a migração.');
  await expect(page.locator('.queue-item').first()).toContainText('Claude');
});

test('Should apply intervention only at the simulated safe step', async ({ page }) => {
  await detail(page);
  await page.locator('#composer').fill('Pare e esclareça a mudança.');
  await page.getByRole('button', { name: 'Intervir agora', exact: true }).click();
  await expect(page.getByText('Intervenção pendente', { exact: true })).toBeVisible();
  await expect(page.locator('.execution-strip')).toContainText('Executando Auto');
  await expect(page.getByRole('button', { name: 'Permitir uma vez', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Aplicar na etapa segura', exact: true }).click();
  await expect(page.locator('.execution-strip')).toContainText('Executando Perguntar');
});

test('Should preserve drafts and disable actions while disconnected', async ({ page }) => {
  await detail(page);
  await page.locator('#composer').fill('Meu rascunho local');
  await scenario(page, 'offline');
  await expect(
    page.getByRole('button', { name: 'Enviar para a fila', exact: true }),
  ).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Parar', exact: true })).toBeDisabled();
  await expect(page.locator('#composer')).toHaveValue('Meu rascunho local');
  await page.getByRole('button', { name: 'Simular snapshot atualizado', exact: true }).click();
  await expect(page.locator('#composer')).toHaveValue('Meu rascunho local');
  await expect(page.getByRole('button', { name: 'Retomar', exact: true })).toBeVisible();
});

test('Should reject an incompatible variant without silently falling back', async ({ page }) => {
  await detail(page);
  await page.getByLabel('Variante', { exact: true }).selectOption('high');
  await page.getByLabel('Modelo', { exact: true }).selectOption('local-mock');
  await page.locator('#composer').fill('Não pode enviar ainda');
  await expect(page.getByLabel('Variante', { exact: true })).toHaveValue('high');
  await expect(
    page.getByRole('button', { name: 'Enviar para a fila', exact: true }),
  ).toBeDisabled();
});

test('Should confirm provider sharing and remember the choice only in this session', async ({
  page,
}) => {
  await detail(page);
  await page.getByLabel('Perfil', { exact: true }).selectOption('plan');
  await page.locator('#composer').fill('Planeja a migração');
  await page.getByRole('button', { name: 'Enviar para a fila', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Compartilhar contexto');
  await dialog.getByLabel('Não perguntar novamente para este provider nesta sessão').check();
  await dialog.getByRole('button', { name: 'Confirmar no protótipo', exact: true }).click();
  await page.locator('#composer').fill('Outra pergunta');
  await page.getByRole('button', { name: 'Enviar para a fila', exact: true }).click();
  await expect(dialog).not.toBeVisible();
});

test('Should expose every scenario and block progression where needed', async ({ page }) => {
  await detail(page);
  for (const id of [
    'quota',
    'incomplete',
    'loop',
    'recovery',
    'sandbox',
    'trust',
    'shared',
    'secret',
    'conflict',
    'model',
    'variant',
    'auth',
    'retry',
  ]) {
    await scenario(page, id);
    await expect(
      page.getByRole('button', { name: 'Simular conclusão', exact: true }),
    ).toBeDisabled();
  }
  await scenario(page, 'running');
  await expect(page.getByRole('button', { name: 'Simular conclusão', exact: true })).toBeEnabled();
});

test('Should offer actionable management and configuration screens', async ({ page }) => {
  await detail(page);
  for (const name of [
    'MCP',
    'Skills',
    'Providers',
    'Dispositivos',
    'Recursos',
    'Diagnóstico',
    'Configuração efetiva',
    'Configurações',
  ]) {
    await nav(page, name);
    await expect(page.locator('.management-content')).toBeVisible();
  }
  await page.getByRole('button', { name: 'Editar perfis', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'Adicionar', exact: true }).click();
  await expect(dialog.getByLabel('Nome do perfil')).toHaveCount(5);
  await dialog.getByRole('button', { name: 'Salvar', exact: true }).click();
});

test('Should review diff and checkpoint without executing or discarding files', async ({
  page,
}) => {
  await detail(page);
  await page
    .locator('.session-tabs')
    .getByRole('button', { name: /Alterações/ })
    .click();
  await expect(page.locator('.diff-code')).toContainText('tx.commit');
  await page.getByRole('button', { name: 'Restaurar checkpoint', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('Nenhum comando será executado');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Confirmar no protótipo', exact: true })
    .click();
  await expect(page.getByRole('status')).toContainText('Suas mudanças não foram descartadas');
});

test('Should navigate the command palette with keyboard and return focus', async ({ page }) => {
  await detail(page);
  await page.locator('#composer').focus();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).not.toBeVisible();
  await page.keyboard.press('Alt+p');
  await expect(page.getByLabel('Perfil', { exact: true })).toHaveValue('auto');
});

for (const theme of ['light', 'dark']) {
  test(`Should render ${theme} review golden states without overflow`, async ({ page }) => {
    await page.goto('/');
    await page.getByLabel('Tema', { exact: true }).selectOption(theme);
    await page.evaluate(() => document.fonts.ready);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
    ).toBe(true);
    await expect(page).toHaveScreenshot(`home-${theme}.png`, {
      fullPage: true,
      animations: 'disabled',
    });
    if (await page.locator('.attention-session').isVisible())
      await page.locator('.attention-session').click();
    await page.locator('.approval-panel').scrollIntoViewIfNeeded();
    await expect(page).toHaveScreenshot(`approval-${theme}.png`, {
      fullPage: true,
      animations: 'disabled',
    });
  });
}
