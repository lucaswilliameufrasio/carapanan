import { models, type Model } from './prototype';

export type ResourceSettings = {
  policy: 'adaptive' | 'manual';
  soft: number;
  hard: number;
  agents: number;
  sessions: number;
  processes: number;
};
export const machines = [
  { id: 'mac', name: 'Mac M3 · 16 GiB', available: 8, cpu: 8 },
  { id: 'tirion', name: 'Tirion · 16 GiB / 4 núcleos', available: 5, cpu: 4 },
  { id: 'ryzen', name: 'Ryzen 9 · 64 GiB / 12 núcleos', available: 42, cpu: 12 },
] as const;
export type Machine = (typeof machines)[number]['id'];
export const integrations = [
  { id: 'chatgpt', name: 'ChatGPT · assinatura', protocol: 'chatgpt', auth: 'oauth', endpoint: '' },
  {
    id: 'openai',
    name: 'OpenAI API',
    protocol: 'openai',
    auth: 'reference',
    endpoint: 'https://api.openai.com/v1',
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    protocol: 'anthropic',
    auth: 'reference',
    endpoint: 'https://api.anthropic.com',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    protocol: 'openai',
    auth: 'reference',
    endpoint: 'https://openrouter.ai/api/v1',
  },
  {
    id: 'gemini',
    name: 'Gemini',
    protocol: 'gemini',
    auth: 'reference',
    endpoint: 'https://generativelanguage.googleapis.com',
  },
  {
    id: 'local',
    name: 'Local · OpenAI-compatible',
    protocol: 'openai',
    auth: 'none',
    endpoint: 'http://127.0.0.1:11434/v1',
  },
];
export const protocols = [
  { value: 'openai', label: 'OpenAI-compatible' },
  { value: 'anthropic', label: 'Anthropic Messages' },
  { value: 'gemini', label: 'Gemini' },
];
export type ProviderModel = {
  id: string;
  upstreamId: string;
  name: string;
  variants: string[];
  streaming: boolean;
  tools: boolean;
  reasoning: boolean;
};
export type ProviderConfig = {
  id: string;
  name: string;
  kind: 'mapped' | 'custom';
  integration: string;
  protocol: string;
  endpoint: string;
  auth: string;
  credentialRef: string;
  enabled: boolean;
  models: ProviderModel[];
};
export type Configuration = { resources: ResourceSettings; providers: ProviderConfig[] };
export function newProvider(id: string): ProviderConfig {
  return {
    id,
    name: 'Meu provider',
    kind: 'mapped',
    integration: 'openai',
    protocol: 'openai',
    endpoint: 'https://api.openai.com/v1',
    auth: 'reference',
    credentialRef: 'cred:demo',
    enabled: true,
    models: [
      {
        id: `${id}-model-1`,
        upstreamId: 'modelo-demo',
        name: 'Meu modelo · mock',
        variants: ['default'],
        streaming: true,
        tools: true,
        reasoning: false,
      },
    ],
  };
}
export function initialConfiguration(): Configuration {
  return {
    resources: { policy: 'adaptive', soft: 2, hard: 4, agents: 4, sessions: 8, processes: 8 },
    providers: models.map((model, index) => ({
      ...newProvider(`builtin-${index}`),
      name: model.provider,
      integration: ['openai', 'anthropic', 'local'][index],
      protocol: index === 1 ? 'anthropic' : 'openai',
      endpoint: integrations.find((p) => p.id === ['openai', 'anthropic', 'local'][index])!
        .endpoint,
      auth: index === 2 ? 'none' : 'reference',
      models: [
        {
          id: model.id,
          upstreamId: model.id,
          name: model.name,
          variants: [...model.variants],
          streaming: true,
          tools: true,
          reasoning: model.variants.length > 1,
        },
      ],
    })),
  };
}
export function providerModels(providers: ProviderConfig[]): Model[] {
  return providers.flatMap((p) =>
    p.models.map((m) => ({
      id: m.id,
      name: m.name,
      provider: p.name,
      variants: m.variants,
      enabled: p.enabled,
    })),
  );
}
export function resourceErrors(settings: ResourceSettings): string[] {
  const errors: string[] = [];
  if (![settings.soft, settings.hard].every((v) => Number.isFinite(v) && v >= 0.25))
    errors.push('Memória deve ser um valor finito de pelo menos 0,25 GiB.');
  if (settings.soft > settings.hard)
    errors.push('O limite suave não pode superar o teto de memória.');
  if (
    ![settings.agents, settings.sessions, settings.processes].every(
      (v) => Number.isSafeInteger(v) && v >= 1,
    )
  )
    errors.push('Paralelismo, sessões e processos devem ser números inteiros positivos.');
  return errors;
}
export function resourcePreview(settings: ResourceSettings, machine: Machine, pressure: boolean) {
  const fixture = machines.find((m) => m.id === machine)!;
  const hard =
    settings.policy === 'manual'
      ? settings.hard
      : Math.min(settings.hard, fixture.available * (pressure ? 0.2 : 0.5));
  return {
    hard,
    soft: settings.policy === 'manual' ? settings.soft : Math.min(settings.soft, hard * 0.75),
    agents:
      settings.policy === 'manual'
        ? settings.agents
        : Math.min(
            settings.agents,
            pressure ? 1 : Math.max(1, Math.floor(fixture.cpu / 2)),
            Math.max(1, Math.floor(hard / 0.5)),
          ),
    available: fixture.available,
    cpu: fixture.cpu,
  };
}
export function providerErrors(candidate: ProviderConfig, others: ProviderConfig[]): string[] {
  const errors: string[] = [];
  if (!candidate.name.trim()) errors.push('Dê um nome ao provider.');
  if (
    others.some(
      (p) =>
        p.id !== candidate.id &&
        p.name.trim().toLowerCase() === candidate.name.trim().toLowerCase(),
    )
  )
    errors.push('Já existe um provider com esse nome.');
  if (
    candidate.auth === 'reference' &&
    !/^cred:[a-z0-9][a-z0-9._-]*$/i.test(candidate.credentialRef)
  )
    errors.push('Use uma referência como cred:meu-provider, nunca uma chave ou token.');
  if (candidate.kind === 'custom') {
    if (!protocols.some((p) => p.value === candidate.protocol))
      errors.push('Escolha um protocolo suportado pelo mock.');
    try {
      const url = new URL(candidate.endpoint);
      if (
        !['https:', 'http:'].includes(url.protocol) ||
        url.username ||
        url.password ||
        url.search ||
        url.hash
      )
        throw new Error('unsafe');
      if (url.protocol === 'http:' && !['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))
        throw new Error('unsafe');
    } catch {
      errors.push('Use uma URL HTTPS (ou HTTP local), sem credenciais, parâmetros ou fragmentos.');
    }
  } else if (!integrations.some((p) => p.id === candidate.integration))
    errors.push('Escolha uma integração mapeada.');
  if (!candidate.models.length) errors.push('Adicione pelo menos um modelo.');
  const ids = new Set<string>();
  for (const model of candidate.models) {
    if (!model.name.trim() || !model.upstreamId.trim() || !/^[\w./:-]+$/.test(model.upstreamId))
      errors.push('Cada modelo precisa de nome e identificador válido, sem espaços.');
    if (ids.has(model.upstreamId))
      errors.push('Identificadores de modelos não podem se repetir neste provider.');
    ids.add(model.upstreamId);
    if (
      !model.variants.length ||
      model.variants.some((v) => !['default', 'low', 'high'].includes(v))
    )
      errors.push('Selecione pelo menos uma variante válida por modelo.');
    if (!model.reasoning && model.variants.some((v) => v !== 'default'))
      errors.push('Low/high exigem a capacidade Reasoning marcada.');
  }
  return [...new Set(errors)];
}
