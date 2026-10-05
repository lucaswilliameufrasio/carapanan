export function serverConfig(env: Record<string, string | undefined>, defaultPort = 5173) {
  const value = env.PORT || String(defaultPort);
  const port = Number(value);
  if (!/^\d+$/.test(value) || !Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error('PORT deve ser um inteiro entre 1 e 65535.');
  }
  return { host: env.HOST || '127.0.0.1', port, strictPort: true };
}
