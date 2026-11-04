import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';

const executeFile = promisify(execFile);
export const string = (value) => typeof value === 'string' ? value.trim() : '';
export const configured = (cfg) => ['storePath', 'peopleMap', 'palacePath'].every(key => string(cfg[key]));

export function hostIdentity(context) {
  const audience = context.memoryAudience;
  return !string(audience?.agentId) ? null
    : audience.kind === 'owner-private' ? ['--owner']
      : audience.kind === 'conversation' && string(audience.sessionKey) && string(audience.sessionId)
        ? [`--conversation=${audience.sessionKey}`] : null;
}

// Shared marker-aware command for tools and native provider. Identity is host-only.
export async function searchMemory(cfg, identity, query, limit, { signal, assertCurrent, maxDistance } = {}) {
  const roots = [...new Set([path.dirname(path.resolve(cfg.storePath)), path.dirname(path.resolve(cfg.peopleMap)), path.resolve(cfg.palacePath)])];
  const args = [...roots.map(root => `--roots=${root}`), `--people-map=${cfg.peopleMap}`,
    'search', ...identity, `--store=${cfg.storePath}`, `--palace=${cfg.palacePath}`, `--query=${query}`, `--limit=${limit}`];
  if (string(cfg.mempalaceMcp)) args.push(`--mempalace-mcp=${cfg.mempalaceMcp}`);
  if (maxDistance != null) args.push(`--max-distance=${maxDistance}`);
  const { stdout } = await runBackend(cfg, args, { signal, assertCurrent });
  const data = JSON.parse(stdout);
  if (!Array.isArray(data?.results)) throw new Error('Memory unavailable.');
  return data;
}

export async function runBackend(cfg, args, { signal, assertCurrent } = {}) {
  assertCurrent?.();
  try {
    return await executeFile(string(cfg.palaceBin) || 'palace', args, {
      encoding: 'utf8', timeout: 15000, maxBuffer: 8 * 1024 * 1024, signal,
    });
  } finally {
    assertCurrent?.();
  }
}
