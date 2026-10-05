import { expect, type Page, test } from '@playwright/test';

async function detail(page: Page) {
  await page.goto('/');
  if (await page.locator('.attention-session').isVisible())
    await page.locator('.attention-session').click();
  await expect(page.locator('#composer')).toBeVisible();
}
async function scenario(page: Page, id: string) {
  await pick(page, 'Cenário de revisão', id);
}
async function pick(page: Page, label: string, value: string) {
  const dialogs = page.getByRole('dialog');
  const scope = (await dialogs.count()) ? dialogs.last() : page;
  await scope.getByRole('combobox', { name: label, exact: true }).last().click();
  await page
    .getByRole('dialog', { name: label, exact: true })
    .locator(`[role="option"][data-value="${value}"]`)
    .click();
}
async function nav(page: Page, name: string) {
  if (await page.locator('.mobile-only').first().isVisible())
    await page.locator('.app-header').getByRole('button', { name: 'Sessões', exact: true }).click();
  await page.locator('.navigation').getByRole('button', { name, exact: true }).click();
}

test('Should preview an interface update and rollback without changing the conversation queue draft or approval', async ({
  page,
}) => {
  await detail(page);
  await page.locator('#composer').fill('Preserve minha revisão');
  await page.locator('.queue summary').click();
  const queue = await page.locator('.queue-items').innerText();
  const executing = await page.locator('.execution-strip').innerText();
  await page
    .locator('.update-banner')
    .getByRole('button', { name: /Nova versão disponível/ })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Atualizações · prévia', exact: true });
  await expect(dialog).toContainText('Nenhum pacote será consultado, baixado ou instalado');
  await dialog.getByRole('button', { name: 'Atualizar agora · mock', exact: true }).click();
  await expect(dialog.locator('.update-status')).toContainText('Verificação pendente');
  await dialog.getByRole('button', { name: 'Simular verificação', exact: true }).click();
  await expect(dialog.locator('.update-status')).toContainText('sem reiniciar o agente');
  await dialog.getByRole('button', { name: 'Simular rollback', exact: true }).click();
  await expect(dialog.locator('.update-status')).toContainText('Rollback simulado concluído');
  await page.keyboard.press('Escape');
  await expect(page.locator('#composer')).toHaveValue('Preserve minha revisão');
  expect(await page.locator('.execution-strip').innerText()).toBe(executing);
  expect(await page.locator('.queue-items').innerText()).toBe(queue);
  await expect(
    page.getByRole('combobox', { name: 'Cenário de revisão', exact: true }),
  ).toHaveAttribute('data-value', 'approval');
  await expect(page.locator('.approval-panel')).toBeAttached();
});

test('Should schedule runtime updates and require an idle preview before explicit restart', async ({
  page,
}) => {
  await detail(page);
  await nav(page, 'Atualizações');
  const dialog = page.getByRole('dialog', { name: 'Atualizações · prévia', exact: true });
  await dialog.getByText('Explorar cenários de atualização', { exact: true }).click();
  await pick(page, 'Pacote de demonstração', 'runtime');
  await pick(page, 'Canal de atualização', 'beta');
  await dialog.getByRole('button', { name: 'Ao terminar a tarefa', exact: true }).click();
  await expect(dialog.locator('.update-status')).toContainText('Atualização agendada');
  await dialog.getByRole('button', { name: 'Preparar agora · mock', exact: true }).click();
  await dialog.getByRole('button', { name: 'Simular verificação', exact: true }).click();
  const restart = dialog.getByRole('button', { name: 'Simular reinício e retomada', exact: true });
  await expect(restart).toBeDisabled();
  await dialog.getByRole('button', { name: 'Simular ponto seguro', exact: true }).click();
  await expect(restart).toBeEnabled();
  await expect(dialog.locator('.update-status')).toContainText('Reinício pendente');
  await restart.click();
  await expect(dialog.locator('.update-release')).toContainText('0.0.1-beta-demo');
  await expect(dialog.locator('.update-status')).toContainText(
    'Nenhum processo real foi reiniciado',
  );
});

test('Should keep deferred updates discoverable block invalid packages and refuse incompatible rollback', async ({
  page,
}) => {
  await detail(page);
  await page.getByRole('button', { name: 'Lembrar atualização depois', exact: true }).click();
  await expect(page.locator('.update-banner')).toHaveCount(0);
  await page.keyboard.press('Control+p');
  await page.getByRole('dialog').getByRole('textbox').fill('atualizacoes');
  await page.getByRole('dialog').getByRole('textbox').press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Atualizações · prévia', exact: true });
  await dialog.getByText('Explorar cenários de atualização', { exact: true }).click();
  await pick(page, 'Pacote de demonstração', 'invalid');
  await dialog.getByRole('button', { name: 'Atualizar agora · mock', exact: true }).click();
  await dialog.getByRole('button', { name: 'Simular verificação', exact: true }).click();
  await expect(dialog.locator('.update-status')).toContainText('Instalação bloqueada');
  await expect(
    dialog.getByRole('button', { name: 'Simular reinício e retomada', exact: true }),
  ).toHaveCount(0);
  await pick(page, 'Pacote de demonstração', 'migration');
  await dialog.getByRole('button', { name: 'Ao terminar a tarefa', exact: true }).click();
  await dialog
    .getByRole('button', { name: 'Simular término da tarefa e fila', exact: true })
    .click();
  await dialog.getByRole('button', { name: 'Simular verificação', exact: true }).click();
  await dialog.getByRole('button', { name: 'Simular reinício e retomada', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Simular rollback', exact: true })).toHaveCount(
    0,
  );
  await expect(dialog).toContainText('não permite rollback automático');
  await page.keyboard.press('Escape');
  await scenario(page, 'offline');
  await nav(page, 'Atualizações');
  await expect(dialog).toContainText('Desconectado: ações de atualização bloqueadas');
});

test('Should hide and restore the desktop sidebar without losing the draft or mobile navigation', async ({
  page,
}) => {
  await detail(page);
  await page.setViewportSize({ width: 1440, height: 956 });
  await page.locator('#composer').fill('Rascunho intacto');
  const sidebar = page.locator('#session-sidebar');
  const toggle = page.getByRole('button', { name: 'Esconder barra lateral', exact: true });
  await expect(sidebar).toBeVisible();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const width = (await page.locator('main').boundingBox())!.width;
  await toggle.focus();
  await page.keyboard.press('Enter');
  await expect(sidebar).not.toBeVisible();
  const show = page.getByRole('button', { name: 'Mostrar barra lateral', exact: true });
  await expect(show).toHaveAttribute('aria-expanded', 'false');
  await expect(show).toBeFocused();
  expect((await page.locator('main').boundingBox())!.width).toBeGreaterThan(width);
  await expect(page.locator('#composer')).toHaveValue('Rascunho intacto');
  await page.keyboard.press('Space');
  await expect(sidebar).toBeVisible();
  await toggle.click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(sidebar).not.toBeVisible();
  await page.locator('.app-header').getByRole('button', { name: 'Sessões', exact: true }).click();
  await expect(sidebar).toBeVisible();
  await page.locator('.app-header').getByRole('button', { name: 'Sessões', exact: true }).click();
  await expect(sidebar).not.toBeVisible();
  await page.setViewportSize({ width: 1440, height: 956 });
  await expect(sidebar).not.toBeVisible();
  await show.click();
  await expect(sidebar).toBeVisible();
  await expect(page.locator('#composer')).toHaveValue('Rascunho intacto');
});

test('Should use touch sized theme and scenario pickers instead of native mobile popups', async ({
  page,
  isMobile,
}) => {
  await detail(page);
  await expect(page.locator('select')).toHaveCount(0);
  if (isMobile)
    await page.locator('.app-header').getByRole('button', { name: 'Sessões', exact: true }).tap();
  for (const theme of ['dark', 'light', 'system']) {
    if (isMobile) {
      await page.getByRole('combobox', { name: 'Tema', exact: true }).tap();
      const dialog = page.getByRole('dialog', { name: 'Tema', exact: true });
      const box = await dialog.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
      await dialog.locator(`[role="option"][data-value="${theme}"]`).tap();
    } else await pick(page, 'Tema', theme);
    await expect(page.locator('.theme-control')).toHaveAttribute('data-value', theme);
    if (theme !== 'system') await expect(page.locator('.app')).toHaveAttribute('data-theme', theme);
  }
  if (isMobile)
    await page.locator('.app-header').getByRole('button', { name: 'Sessões', exact: true }).tap();
  await page.getByRole('combobox', { name: 'Cenário de revisão' }).click();
  const dialog = page.getByRole('dialog', { name: 'Cenário de revisão', exact: true });
  const last = dialog.getByRole('option').last();
  await last.scrollIntoViewIfNeeded();
  expect((await last.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await last.click();
  await expect(dialog).not.toBeVisible();
});

test('Should keep nested queue pickers cancelable without losing the edit dialog or draft', async ({
  page,
}) => {
  await detail(page);
  await page.locator('.queue summary').click();
  await page
    .locator('.queue-items')
    .getByRole('button', { name: 'Editar', exact: true })
    .first()
    .click();
  const editor = page.getByRole('dialog');
  await editor.getByRole('textbox').fill('Rascunho preservado');
  const trigger = editor.getByRole('combobox', { name: 'Modelo', exact: true });
  await trigger.click();
  await expect(page.getByRole('dialog', { name: 'Modelo', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(editor.getByRole('textbox')).toHaveValue('Rascunho preservado');
  await expect(trigger).toBeFocused();
});

test('Should theme scrollbars and preserve accessible forced color defaults', async ({ page }) => {
  await detail(page);
  const colors = [];
  for (const theme of ['light', 'dark']) {
    await pick(page, 'Tema', theme);
    const scroll = page.locator('.session-content');
    colors.push(await scroll.evaluate((el) => getComputedStyle(el).scrollbarColor));
    await expect(scroll).toHaveCSS('scrollbar-width', 'thin');
  }
  expect(colors[0]).not.toBe(colors[1]);
  expect(colors).not.toContain('auto');
  await page.emulateMedia({ forcedColors: 'active' });
  await expect(page.locator('.session-content')).toHaveCSS('scrollbar-color', 'auto');
});

test('Should select settings resources config and nested profile fields with the same picker', async ({
  page,
}) => {
  await detail(page);
  await nav(page, 'Configurações');
  await expect(page.locator('select')).toHaveCount(0);
  await pick(page, 'Tema', 'dark');
  await expect(page.locator('.app')).toHaveAttribute('data-theme', 'dark');
  await pick(page, 'Idioma', 'pt-BR');
  await pick(page, 'Escopo', 'project');
  await expect(page.getByRole('combobox', { name: 'Escopo', exact: true })).toHaveAttribute(
    'data-value',
    'project',
  );
  await page.getByRole('button', { name: 'Editar perfis', exact: true }).click();
  await expect(page.locator('select')).toHaveCount(0);
  await pick(page, 'Modelo · Perguntar', 'claude-mock');
  await pick(page, 'Variante · Perguntar', 'low');
  await page.getByRole('dialog').getByRole('button', { name: 'Salvar', exact: true }).click();
  await nav(page, 'Recursos');
  await expect(page.locator('select')).toHaveCount(0);
  await page.getByRole('spinbutton', { name: 'Teto de memória (GiB)' }).fill('6');
  await page.getByRole('button', { name: 'Aplicar no mock', exact: true }).click();
  await expect(page.getByRole('spinbutton', { name: 'Teto de memória (GiB)' })).toHaveValue('6');
  await nav(page, 'Configuração efetiva');
  await expect(page.locator('select')).toHaveCount(0);
  await pick(page, 'Explicar precedência', 'agent.mode');
  await expect(page.locator('.detail-block h3')).toHaveText('agent.mode');
  await scenario(page, 'offline');
  await nav(page, 'Recursos');
  await expect(page.getByRole('spinbutton', { name: 'Teto de memória (GiB)' })).toBeDisabled();
});

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

test('Should edit arbitrary resource caps validate them and preview adaptation without runtime changes', async ({
  page,
}) => {
  await detail(page);
  await nav(page, 'Recursos');
  await page.getByRole('spinbutton', { name: 'Limite suave (GiB)' }).fill('7');
  await page.getByRole('button', { name: 'Aplicar no mock', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('limite suave não pode superar');
  await page.getByRole('spinbutton', { name: 'Teto de memória (GiB)' }).fill('12');
  await page.getByRole('spinbutton', { name: 'Agentes simultâneos' }).fill('6');
  await page.getByRole('button', { name: /^Manual/ }).click();
  await expect(page.locator('.preview-metrics')).toContainText('6');
  await page.getByRole('button', { name: 'Aplicar no mock', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await nav(page, 'Diagnóstico');
  await nav(page, 'Recursos');
  await expect(page.getByRole('spinbutton', { name: 'Limite suave (GiB)' })).toHaveValue('7');
  await expect(page.getByRole('spinbutton', { name: 'Agentes simultâneos' })).toHaveValue('6');
  await page.getByRole('button', { name: /^Adaptativo/ }).click();
  await pick(page, 'Máquina de referência', 'tirion');
  await page.getByRole('checkbox', { name: 'Simular pressão de memória' }).check();
  await expect(page.locator('.preview-metrics strong').first()).toHaveText('1');
  await expect(page.getByRole('spinbutton', { name: 'Agentes simultâneos' })).toHaveValue('6');
  await page.getByRole('button', { name: 'Descartar ajustes', exact: true }).click();
  await expect(page.getByRole('button', { name: /^Manual/ })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await page.reload();
  await nav(page, 'Recursos');
  await expect(page.getByRole('spinbutton', { name: 'Limite suave (GiB)' })).toHaveValue('2');
  await expect(page.getByRole('spinbutton', { name: 'Agentes simultâneos' })).toHaveValue('4');
});

test('Should add a custom provider and queue its model with explicit sharing confirmation without network', async ({
  page,
}) => {
  await detail(page);
  const external: string[] = [];
  page.on('request', (request) => {
    if (new URL(request.url()).hostname === 'gateway.example.invalid') external.push(request.url());
  });
  const executing = await page.locator('.execution-strip').getAttribute('data-profile');
  await nav(page, 'Providers');
  await page.getByRole('button', { name: 'Adicionar provider', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Adicionar provider', exact: true });
  await dialog.getByRole('button', { name: /^Custom/ }).click();
  await dialog.getByLabel('Nome do provider', { exact: true }).fill('Meu gateway');
  await dialog.getByLabel('URL base', { exact: true }).fill('https://gateway.example.invalid/v1');
  await dialog.getByLabel('Referência da credencial', { exact: true }).fill('cred:gateway-demo');
  await dialog.getByLabel('Nome do modelo 1', { exact: true }).fill('Modelo da casa');
  await dialog.getByLabel('Identificador do modelo 1', { exact: true }).fill('org/modelo-casa');
  await dialog
    .getByRole('group', { name: 'Capacidades do modelo 1' })
    .getByRole('checkbox', { name: 'Reasoning' })
    .check();
  await dialog
    .getByRole('group', { name: 'Variantes do modelo 1' })
    .getByRole('checkbox', { name: 'high' })
    .check();
  await dialog.getByRole('button', { name: 'Adicionar modelo', exact: true }).click();
  await dialog.getByLabel('Nome do modelo 2', { exact: true }).fill('Modelo rápido');
  await dialog.getByLabel('Identificador do modelo 2', { exact: true }).fill('org/rapido');
  await dialog.getByRole('button', { name: 'Salvar provider no mock', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.locator('.provider-list')).toContainText('Modelo rápido');
  await page.keyboard.press('Control+p');
  await page.getByRole('dialog').getByRole('textbox').fill('Conversa');
  await page.getByRole('dialog').getByRole('textbox').press('Enter');
  await expect(page.locator('.execution-strip')).toHaveAttribute('data-profile', executing!);
  await pick(page, 'Modelo', 'provider-4-model-1');
  await pick(page, 'Variante', 'high');
  await page.locator('#composer').fill('Testar a seleção, sem execução real');
  await page.getByRole('button', { name: 'Enviar para a fila', exact: true }).click();
  await expect(page.getByRole('dialog')).toContainText('Meu gateway');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Confirmar no protótipo', exact: true })
    .click();
  await page.locator('.queue summary').click();
  await expect(page.locator('.queue-item').last()).toContainText('Modelo da casa');
  await expect(page.locator('.queue-item').last()).toContainText('high');
  await nav(page, 'Providers');
  await page.getByRole('button', { name: 'Desabilitar Meu gateway', exact: true }).click();
  await page.keyboard.press('Control+p');
  await page.getByRole('dialog').getByRole('textbox').fill('Conversa');
  await page.getByRole('dialog').getByRole('textbox').press('Enter');
  await expect(page.getByRole('combobox', { name: 'Modelo', exact: true })).toHaveAttribute(
    'data-value',
    'provider-4-model-1',
  );
  await page.locator('#composer').fill('Não trocar silenciosamente');
  await expect(
    page.getByRole('button', { name: 'Enviar para a fila', exact: true }),
  ).toBeDisabled();
  await expect(page.locator('.execution-strip')).toHaveAttribute('data-profile', executing!);
  expect(external).toEqual([]);
});

test('Should validate cancel and edit a mapped provider without accepting raw credentials', async ({
  page,
}) => {
  await detail(page);
  await nav(page, 'Providers');
  await page.getByRole('button', { name: 'Adicionar provider', exact: true }).click();
  let dialog = page.getByRole('dialog', { name: 'Adicionar provider', exact: true });
  await pick(page, 'Integração', 'openrouter');
  await dialog.getByLabel('Nome do provider', { exact: true }).fill('Router pessoal');
  await dialog
    .getByLabel('Referência da credencial', { exact: true })
    .fill('chave-nao-e-referencia');
  await dialog.getByRole('button', { name: 'Salvar provider no mock', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('nunca uma chave ou token');
  await dialog.getByLabel('Referência da credencial', { exact: true }).fill('cred:router');
  await dialog.getByRole('button', { name: 'Salvar provider no mock', exact: true }).click();
  const card = page
    .locator('.provider-card')
    .filter({ has: page.getByRole('heading', { name: 'Router pessoal', exact: true }) });
  await card.getByRole('button', { name: 'Editar Router pessoal', exact: true }).click();
  dialog = page.getByRole('dialog', { name: 'Editar provider', exact: true });
  await expect(dialog.getByRole('combobox', { name: 'Integração', exact: true })).toHaveAttribute(
    'data-value',
    'openrouter',
  );
  await dialog.getByLabel('Nome do provider', { exact: true }).fill('Não salvar');
  await dialog.getByRole('button', { name: 'Cancelar', exact: true }).click();
  await expect(card).toContainText('Router pessoal');
  await scenario(page, 'offline');
  await expect(
    page.getByRole('button', { name: 'Adicionar provider', exact: true }),
  ).toBeDisabled();
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
  await pick(page, 'Perfil', 'plan');
  await pick(page, 'Modelo', 'claude-mock');
  await pick(page, 'Variante', 'low');
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
    await pick(page, 'Tema', theme);
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
    await nav(page, 'Recursos');
    await expect(page).toHaveScreenshot(`resources-${theme}.png`, { animations: 'disabled' });
    await nav(page, 'Providers');
    await page.locator('.management-content').evaluate((el) => (el.scrollTop = 0));
    await expect(page).toHaveScreenshot(`providers-${theme}.png`, { animations: 'disabled' });
    await page.getByRole('button', { name: 'Adicionar provider', exact: true }).click();
    await expect(page).toHaveScreenshot(`provider-mapped-${theme}.png`, { animations: 'disabled' });
    await page
      .getByRole('dialog')
      .getByRole('button', { name: /^Custom/ })
      .click();
    await expect(page).toHaveScreenshot(`provider-custom-${theme}.png`, { animations: 'disabled' });
    await page.keyboard.press('Escape');
    await nav(page, 'Atualizações');
    await expect(page).toHaveScreenshot(`updates-available-${theme}.png`, {
      animations: 'disabled',
    });
    await page
      .getByRole('dialog')
      .getByRole('button', { name: 'Atualizar agora · mock', exact: true })
      .click();
    await page
      .getByRole('dialog')
      .getByRole('button', { name: 'Simular verificação', exact: true })
      .click();
    await expect(page).toHaveScreenshot(`updates-installed-${theme}.png`, {
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
