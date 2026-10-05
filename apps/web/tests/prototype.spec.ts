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
async function pick(page: Page, label: string, value: string) {
  await page.getByRole('combobox', { name: label, exact: true }).click();
  const names: Record<string, string> = {
    plan: 'Planejar',
    ask: 'Perguntar',
    auto: 'Auto',
    yolo: 'Yolo',
    'gpt-mock': 'GPT · mock',
    'claude-mock': 'Claude · mock',
    'local-mock': 'Local · mock',
  };
  await page
    .getByRole('dialog', { name: label, exact: true })
    .getByRole('option', { name: names[value] ?? value, exact: true })
    .click();
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
  await pick(page, 'Perfil', 'auto');
  await expect(page.locator('.execution-strip')).toContainText('Auto');
  await page.locator('#composer').fill('Verifica também o logout.');
  await page.getByRole('button', { name: 'Enviar para a fila', exact: true }).click();
  await pick(page, 'Perfil', 'ask');
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
  await pick(page, 'Variante', 'high');
  await pick(page, 'Modelo', 'local-mock');
  await page.locator('#composer').fill('Não pode enviar ainda');
  await expect(page.getByRole('combobox', { name: 'Variante', exact: true })).toHaveAttribute(
    'data-value',
    'high',
  );
  await expect(
    page.getByRole('button', { name: 'Enviar para a fila', exact: true }),
  ).toBeDisabled();
});

test('Should confirm provider sharing and remember the choice only in this session', async ({
  page,
}) => {
  await detail(page);
  await pick(page, 'Perfil', 'plan');
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
  await expect(page.getByRole('combobox', { name: 'Perfil', exact: true })).toHaveAttribute(
    'data-value',
    'auto',
  );
});

for (const shortcut of ['Control+p', 'Meta+p']) {
  test(`Should open the palette with ${shortcut} without printing or losing the draft`, async ({
    page,
  }) => {
    await detail(page);
    await page.locator('#composer').fill('Rascunho preservado');
    await page.locator('#composer').focus();
    await page.keyboard.press(shortcut);
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('#composer')).toHaveValue('Rascunho preservado');
    await expect(page.locator('#composer')).toBeFocused();
  });
}

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

test('Should center search, find sessions without accents, and activate results with Enter', async ({
  page,
}) => {
  await detail(page);
  await page.keyboard.press('Control+p');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  expect(Math.abs(box!.x + box!.width / 2 - page.viewportSize()!.width / 2)).toBeLessThan(2);
  const search = dialog.getByRole('textbox');
  await expect(search).toBeFocused();
  await search.fill('tixnow');
  await expect(dialog.getByRole('button', { name: /tixnow-web/ })).toBeVisible();
  await search.press('Enter');
  await expect(dialog).not.toBeVisible();
  await expect(page.locator('.session-title')).toContainText('tixnow-web');
  await page.keyboard.press('Control+p');
  await page.getByRole('dialog').getByRole('textbox').fill('diagnostico');
  await expect(page.getByRole('dialog').getByRole('button', { name: /Diagnóstico/ })).toBeVisible();
});

test('Should open readable composer pickers within the viewport and preserve the draft on cancel', async ({
  page,
}) => {
  await detail(page);
  await page.locator('#composer').fill('Rascunho de revisão');
  await page.getByRole('combobox', { name: 'Perfil', exact: true }).click();
  const picker = page.getByRole('dialog', { name: 'Perfil', exact: true });
  await expect(picker).toBeVisible();
  const box = await picker.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  for (const name of ['Planejar', 'Perguntar', 'Auto', 'Yolo']) {
    const option = picker.getByRole('option', { name, exact: true });
    await expect(option).toBeVisible();
    expect((await option.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  }
  await page.keyboard.press('Escape');
  await expect(page.locator('#composer')).toHaveValue('Rascunho de revisão');
  const trigger = page.getByRole('combobox', { name: 'Perfil', exact: true });
  await trigger.focus();
  await trigger.press('ArrowDown');
  await page
    .getByRole('dialog', { name: 'Perfil', exact: true })
    .getByRole('option', { name: 'Perguntar', exact: true })
    .press('ArrowDown');
  await expect(trigger).toHaveAttribute('data-value', 'ask');
  await page.keyboard.press('Enter');
  await expect(trigger).toHaveAttribute('data-value', 'auto');
  await expect(trigger).toBeFocused();
});

test('Should change mode hue and effort intensity without recoloring the executing selection', async ({
  page,
}) => {
  await detail(page);
  const border = () => page.locator('.composer').evaluate((el) => getComputedStyle(el).borderColor);
  const executing = await page.locator('.execution-strip').getAttribute('data-profile');
  const hues = [];
  for (const mode of ['plan', 'ask', 'auto', 'yolo']) {
    await pick(page, 'Perfil', mode);
    hues.push(await border());
    await expect(page.locator('.execution-strip')).toHaveAttribute('data-profile', executing!);
  }
  expect(new Set(hues).size).toBe(4);
  await pick(page, 'Perfil', 'ask');
  const levels = [];
  for (const level of ['low', 'default', 'high']) {
    await pick(page, 'Variante', level);
    levels.push(await border());
  }
  expect(new Set(levels).size).toBe(3);
});

test('Should keep management content and controls in bounds at phone tablet and desktop widths', async ({
  page,
}) => {
  await detail(page);
  for (const width of [320, 900, 1440]) {
    await page.setViewportSize({ width, height: 956 });
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
      const content = await page.locator('.management-content').boundingBox();
      const main = await page.locator('main').boundingBox();
      expect(Math.abs(content!.width - main!.width)).toBeLessThan(2);
      expect(
        await page
          .locator('.management-content')
          .evaluate((el) => el.scrollWidth <= el.clientWidth),
      ).toBe(true);
      for (const control of await page
        .locator('.management-content button, .management-content select')
        .all()) {
        const box = await control.boundingBox();
        if (box) {
          expect(box.x).toBeGreaterThanOrEqual(0);
          expect(box.x + box.width).toBeLessThanOrEqual(width);
        }
      }
    }
  }
});
