import { configured, hostIdentity, runBackend, searchMemory, string } from './backend.js';

const providerId = 'mempalace-memory';
const fail = (message = 'Memory unavailable.') => { throw new Error(message); };
const recordId = (hit) => string(hit.id) || string(hit.drawer_id)
  || string(hit.source_file).replace(/\.md$/, '');

function mapHit(hit) {
  if (!hit || typeof hit !== 'object' || !recordId(hit) || typeof hit.text !== 'string') fail();
  const score = Number.isFinite(hit.distance) ? Math.max(0, Math.min(1, 1 - hit.distance / 2)) : undefined;
  return { reference: { providerId, id: recordId(hit) }, excerpt: hit.text,
    ...(score == null ? {} : { score }) };
}

export function createProviderRuntime(cfg) {
  return {
    async open({ agentId, context }) {
      let closed = false;
      let revoked = false;
      const check = () => {
        if (closed) fail('Memory handle closed.');
        if (revoked) fail('Memory authority unavailable.');
        try { context.assertCurrent(); } catch {
          revoked = true;
          fail('Memory authority unavailable.');
        }
      };
      const identity = context.authority?.kind === 'session'
        && context.authority.audience?.agentId === agentId
        ? hostIdentity({ memoryAudience: context.authority.audience }) : null;
      const authorize = () => {
        check();
        if (!identity) fail('Memory access denied.');
        if (!configured(cfg)) fail();
      };
      const search = async (query, limit, maxDistance) => {
        try {
          return (await searchMemory(cfg, identity, query, limit, {
            signal: context.signal, assertCurrent: check, maxDistance,
          })).results.map(mapHit);
        } catch { fail(); }
      };
      check();
      return { provider: {
        capabilities: { sources: ['memory'], pagination: false, candidates: [], projectFilter: false },
        async search(request) {
          authorize();
          if (request.cursor != null || request.sources?.some(source => source !== 'memory')
            || request.activeProjectKeys?.length) fail('Unsupported memory request.');
          const query = string(request.query);
          if (!query || Array.from(query).length > 250
            || (request.maxResults != null && (!Number.isSafeInteger(request.maxResults) || request.maxResults < 1))
            || (request.minScore != null && (!Number.isFinite(request.minScore) || request.minScore < 0 || request.minScore > 1))) {
            fail('Invalid memory request.');
          }
          const limit = Math.min(100, request.maxResults ?? 5);
          // palace treats max-distance=0 as unbounded; always enforce minScore locally too.
          const hits = (await search(query, limit, request.minScore == null ? undefined : 2 * (1 - request.minScore)))
            .filter(hit => request.minScore == null || (hit.score != null && hit.score >= request.minScore));
          check();
          return { hits };
        },
        async get(request) {
          authorize();
          const ref = request.reference;
          if (ref?.providerId !== providerId) fail('Foreign memory reference.');
          const id = string(ref.id);
          if (!id || Array.from(id).length > 250 || ref.revision != null || ref.fragment != null
            || (request.from != null && (!Number.isSafeInteger(request.from) || request.from < 1))
            || (request.lines != null && (!Number.isSafeInteger(request.lines) || request.lines < 1))) fail('Invalid memory reference.');
          const [hit] = await search(id, 1);
          let result = { status: 'not_found' };
          if (hit?.reference.id === id) {
            const lines = hit.excerpt.split('\n');
            const from = request.from ?? 1;
            const end = request.lines == null ? lines.length : from - 1 + request.lines;
            const text = lines.slice(from - 1, end).join('\n');
            result = { status: 'ok', reference: hit.reference, text,
              ...(request.from != null || request.lines != null
                ? { from, lines: Math.max(0, Math.min(end, lines.length) - from + 1), truncated: end < lines.length,
                  ...(end < lines.length ? { nextFrom: end + 1 } : {}) } : {}) };
          }
          check();
          return result;
        },
        async health() {
          check();
          let status = 'degraded';
          if (configured(cfg)) {
            try {
              await runBackend(cfg, ['--version'], { signal: context.signal, assertCurrent: check });
              status = 'ready';
            } catch { status = 'unavailable'; }
          }
          check();
          return { status };
        },
        async close() { closed = true; },
      } };
    },
  };
}
