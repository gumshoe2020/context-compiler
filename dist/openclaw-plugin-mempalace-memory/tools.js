import { hostIdentity, searchMemory } from './backend.js';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';

const executeFile = promisify(execFile);
const textResult = (text, results = []) => ({ content: [{ type: 'text', text }], details: { results } });
const notice = 'Memory unavailable; no results returned.';
const string = (v) => typeof v === 'string' ? v.trim() : '';
const names = ['memory_search', 'mempalace_search', 'memory_get'];

function current(context) {
  // Version 2 requires this guard. A missing guard is a host incompatibility, never authority.
  context.assertInvocationCurrent();
  context.assertMemoryAudienceCurrent?.();
}

export function registerTools(api) {
  const cfg = api.pluginConfig ?? {};
  registerContinuityTools(api, cfg);
  for (const name of names) {
    api.registerTool({
      contextVersion: 2,
      create(context) {
        // Only host-prepared authority is captured. Model parameters never supply identity.
        const identity = hostIdentity(context);
        return {
          name,
          label: name === 'memory_get' ? 'Memory Get' : 'Memory Search',
          description: 'Search long-term memory visible to this authenticated conversation.',
          parameters: {
            type: 'object', additionalProperties: false,
            ...(name === 'memory_get' ? {} : { required: ['query'] }),
            properties: {
              query: { type: 'string' },
              ...(name === 'memory_get' ? { id: { type: 'string' } } : {}),
              wing: { type: 'string', description: 'Compatibility parameter; ignored.' },
              room: { type: 'string', description: 'Compatibility parameter; ignored.' },
              limit: { type: 'number' }, results: { type: 'number' },
            },
          },
          async execute(_toolCallId, params, signal) {
            current(context);
            let result = textResult(notice);
            const query = name === 'memory_get' ? string(params?.id) || string(params?.query) : string(params?.query);
            if (identity && query && Array.from(query).length <= 250 && string(cfg.storePath) && string(cfg.peopleMap) && string(cfg.palacePath)) {
              const requested = Number(params?.limit ?? params?.results ?? 5);
              const limit = name === 'memory_get' ? 1 : Number.isFinite(requested) ? Math.max(1, Math.min(20, Math.trunc(requested))) : 5;
              try {
                const data = await searchMemory(cfg, identity, query, limit, { signal });
                if (Array.isArray(data.results)) result = textResult(JSON.stringify({ results: data.results }), data.results);
              } catch {
                result = textResult(notice);
              }
            }
            // No await or other work between the final authority checks and release.
            current(context);
            return result;
          },
        };
      },
    }, { name });
  }
}

function registerContinuityTools(api, cfg) {
  for (const name of ['mempalace_diary_write', 'mempalace_status']) {
    const writing = name === 'mempalace_diary_write';
    api.registerTool({
      contextVersion: 2,
      create(context) {
        const identity = hostIdentity(context);
        if (!writing && !identity?.includes('--owner')) return null;
        return {
          name, label: writing ? 'Memory Diary Write' : 'Memory Status',
          description: writing ? 'File a continuity note in the main diary with this authenticated audience’s privacy marker.'
            : 'Owner-private palace overview; counts only.',
          parameters: {
            type: 'object', additionalProperties: false,
            ...(writing ? { required: ['text'] } : {}),
            properties: writing ? { text: { type: 'string' }, title: { type: 'string' },
              wing: { type: 'string', description: 'Compatibility parameter; ignored.' } } : {},
          },
          async execute(_toolCallId, params, signal) {
            current(context);
            let result = textResult(writing ? 'Memory unavailable; nothing written.' : notice);
            const configured = identity && string(cfg.peopleMap) && string(cfg.palacePath)
              && (writing ? string(cfg.diaryRoot) : string(cfg.storePath));
            const valid = !writing || (typeof params?.text === 'string' && params.text.trim()
              && Buffer.byteLength(params.text) <= 65536
              && (params.title == null || (typeof params.title === 'string'
                && !/[\r\n]/.test(params.title) && Array.from(params.title).length <= 200)));
            if (configured && valid) {
              const roots = [...new Set([path.dirname(path.resolve(cfg.peopleMap)), path.resolve(cfg.palacePath),
                writing ? path.resolve(cfg.diaryRoot) : path.dirname(path.resolve(cfg.storePath))])];
              const args = [...roots.map(root => `--roots=${root}`), `--people-map=${cfg.peopleMap}`,
                writing ? 'diary-write' : 'status', ...identity, `--palace=${cfg.palacePath}`];
              if (writing) {
                args.push(`--diary-root=${cfg.diaryRoot}`, `--text=${params.text}`);
                if (params.title != null) args.push(`--title=${params.title}`);
                if (string(cfg.mempalace)) args.push(`--mempalace=${cfg.mempalace}`);
              } else args.push(`--store=${cfg.storePath}`);
              try {
                const { stdout } = await executeFile(string(cfg.palaceBin) || 'palace', args, {
                  encoding: 'utf8', timeout: writing ? 30000 : 15000, maxBuffer: 1024 * 1024, signal,
                });
                const data = JSON.parse(stdout);
                if (writing && typeof data.file === 'string' && data.file.endsWith('.md')) {
                  const file = path.basename(data.file);
                  result = { content: [{ type: 'text', text: JSON.stringify({ file, mined: data.mined === true }) }],
                    details: { file, mined: data.mined === true } };
                } else if (!writing) {
                  const keys = ['entries', 'drawers', 'claims', 'findings', 'improvement_events'];
                  if (keys.every(key => Number.isSafeInteger(data.counts?.[key]) && data.counts[key] >= 0)) {
                    const counts = Object.fromEntries(keys.map(key => [key, data.counts[key]]));
                    result = { content: [{ type: 'text', text: JSON.stringify({ counts }) }], details: { counts } };
                  }
                }
              } catch { /* Never release transport output or raw backend errors. */ }
            }
            current(context);
            return result;
          },
        };
      },
    }, { name });
  }
}
