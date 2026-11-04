import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

// Stub only the SDK entry wrapper; execute the repository entry's actual registration.
const source = (await fs.readFile(new URL('./index.js', import.meta.url), 'utf8'))
  .replace("import { definePluginEntry } from 'openclaw/plugin-sdk/plugin-entry';", 'const definePluginEntry = entry => entry;')
  .replaceAll("'./tools.js'", JSON.stringify(new URL('./tools.js', import.meta.url).href))
  .replaceAll("'./provider.js'", JSON.stringify(new URL('./provider.js', import.meta.url).href));
const { default: entry } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const scratch = fileURLToPath(new URL('../../target/memprov1/provider-tests/', import.meta.url));
await fs.mkdir(scratch, { recursive: true });
const owner = { kind: 'owner-private', agentId: 'invented' };
const session = audience => ({ kind: 'session', sessionKey: 'untrusted-route', sandboxed: false, audience });
const ref = id => ({ providerId: 'mempalace-memory', id });

async function setup(authority = session(owner), options = {}) {
  const dir = await fs.mkdtemp(path.join(scratch, 'fixture-'));
  const log = path.join(dir, 'calls.jsonl');
  const bin = path.join(dir, 'palace');
  await fs.writeFile(bin, `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify(args)+'\\n');
if(args.includes('--version')) { console.log('invented version'); process.exit(0); }
const query = args.find(a=>a.startsWith('--query='))?.slice(8);
if(query==='backend-error') { console.error('/invented/secret stderr'); console.log('secret stdout'); process.exit(1); }
if(query==='malformed') { console.log('secret malformed'); process.exit(0); }
if(query==='invalid-row') { console.log(JSON.stringify({results:[{text:'secret invalid row'}]})); process.exit(0); }
console.log(JSON.stringify({results: query==='unknown' ? [] : [
{id:'drawer-1', source_file:'entry-1.md', text:'Invented amber glider.', distance:0.2, secret:'/invented/secret'},
{source_file:'entry-2.md', text:'Invented cobalt glider.', distance:1.6},
{source_file:'entry-3.md', text:'Invented unscored glider.'}
].slice(0,Number(args.find(a=>a.startsWith('--limit='))?.slice(8)))}));
`, { mode: 0o700 });
  const cfg = { palaceBin: bin, palacePath: dir, storePath: path.join(dir, 'compiler.sqlite'),
    peopleMap: path.join(dir, 'people.json'), ...options.cfg };
  let checks = 0;
  const context = { authority, assertCurrent() {
    checks++;
    if (options.staleAt === checks) throw new Error('/invented/secret guard exception');
  } };
  let capability;
  entry.register({ pluginConfig: cfg, registerMemoryCapability(c) { capability = c; }, registerTool() {} });
  const open = () => capability.providerRuntime.open({ agentId: 'invented', context });
  const { provider } = await open();
  return { provider, capability, open, checks: () => checks,
    async calls() { try { return (await fs.readFile(log, 'utf8')).trim().split('\n').map(JSON.parse); }
      catch (e) { if (e.code === 'ENOENT') return []; throw e; } } };
}

test('registered runtime and caller handle match the required contract', async () => {
  const f = await setup();
  assert.equal(typeof f.capability.providerRuntime.open, 'function');
  assert.deepEqual(f.capability.recallToolNames, ['memory_search', 'memory_get']);
  for (const member of ['search', 'get', 'health', 'close']) assert.equal(typeof f.provider[member], 'function');
  assert.deepEqual(f.provider.capabilities, { sources: ['memory'], pagination: false, candidates: [], projectFilter: false });
  assert(!('candidates' in f.provider));
});

for (const audience of [owner, { kind: 'conversation', agentId: 'invented', sessionKey: 'invented:root', sessionId: 'synthetic-1' }]) {
  test(`${audience.kind} uses only its host audience and scopes references`, async () => {
    const f = await setup(session(audience));
    const page = await f.provider.search({ query: 'glider', maxResults: 3, sources: ['memory'] });
    assert.equal(page.hits.length, 3);
    assert.deepEqual(page.hits[0], { reference: ref('drawer-1'), excerpt: 'Invented amber glider.', score: 0.9 });
    assert.deepEqual(page.hits[1].reference, ref('entry-2'));
    assert(!JSON.stringify(page).includes('secret'));
    const [args] = await f.calls();
    assert.equal(args.includes('--owner'), audience.kind === 'owner-private');
    assert.equal(args.includes('--conversation=invented:root'), audience.kind === 'conversation');
    assert(!args.some(a => /untrusted-route/.test(a)));
    assert.equal(f.checks(), 5); // open, authorize, pre-exec, post-exec, release
  });
}

for (const authority of [session(undefined), { kind: 'operator', scopes: ['operator.admin'] },
  { kind: 'host', operation: 'status' }, session({ ...owner, agentId: 'other' }),
  session({ kind: 'conversation', agentId: 'invented', sessionKey: 'incomplete' })]) {
  test(`ungranted ${JSON.stringify(authority)} denies search/get without exec`, async () => {
    const f = await setup(authority);
    await assert.rejects(f.provider.search({ query: 'glider' }), /access denied/);
    await assert.rejects(f.provider.get({ reference: ref('drawer-1') }), /access denied/);
    assert.deepEqual(await f.calls(), []);
  });
}

for (const method of ['search', 'get', 'health']) {
  test(`${method} releases nothing if currency fails after execution`, async () => {
    const f = await setup(session(owner), { staleAt: 4 });
    await assert.rejects(f.provider[method](method === 'search' ? { query: 'glider' } : { reference: ref('drawer-1') }),
      error => !error.message.includes('secret'));
    assert.equal((await f.calls()).length, 1);
  });
}

test('foreign references reject before exec; get requires exact ID and distinguishes missing', async () => {
  const f = await setup();
  await assert.rejects(f.provider.get({ reference: { providerId: 'foreign', id: 'drawer-1' } }), /Foreign/);
  assert.deepEqual(await f.calls(), []);
  assert.deepEqual(await f.provider.get({ reference: ref('unknown') }), { status: 'not_found' });
  assert.deepEqual(await f.provider.get({ reference: ref('unrelated') }), { status: 'not_found' });
  assert.deepEqual(await f.provider.get({ reference: ref('drawer-1') }),
    { status: 'ok', reference: ref('drawer-1'), text: 'Invented amber glider.' });
  assert((await f.calls()).every(args => args.includes('--limit=1')));
});

for (const authority of [session(owner), { kind: 'operator', scopes: ['operator.read'] }, { kind: 'host', operation: 'status' }]) {
  test(`${authority.kind} health exposes readiness alone`, async () => {
    const f = await setup(authority);
    assert.deepEqual(await f.provider.health(), { status: 'ready' });
    assert.deepEqual(await f.calls(), [['--version']]);
  });
}

test('health reports incomplete config and missing binary without leaking paths', async () => {
  const f = await setup(undefined, { cfg: { peopleMap: '' } });
  assert.deepEqual(await f.provider.health(), { status: 'degraded' });
  assert.deepEqual(await f.calls(), []);
  const missing = await setup(undefined, { cfg: { palaceBin: path.join(scratch, 'nonexistent') } });
  assert.deepEqual(await missing.provider.health(), { status: 'unavailable' });
});

test('minScore enforced locally, max-distance mapped, unsupported capabilities reject before exec', async () => {
  const f = await setup();
  for (const extra of [{ cursor: '' }, { sources: ['sessions'] }, { activeProjectKeys: ['invented'] }, { minScore: NaN }]) {
    await assert.rejects(f.provider.search({ query: 'glider', ...extra }));
  }
  assert.deepEqual(await f.calls(), []);
  const page = await f.provider.search({ query: 'glider', minScore: 0.8 });
  assert.equal(page.hits.length, 1);
  assert((await f.calls())[0].includes('--max-distance=0.3999999999999999'));
  assert.deepEqual(await f.provider.search({ query: 'glider', minScore: 1 }), { hits: [] });
});

test('errors and malformed output never become success or disclose backend text', async () => {
  const f = await setup();
  for (const query of ['backend-error', 'malformed', 'invalid-row']) {
    for (const operation of [() => f.provider.search({ query }), () => f.provider.get({ reference: ref(query) })]) {
      await assert.rejects(operation(), error => error.message === 'Memory unavailable.');
    }
  }
});

test('closed caller does not affect another handle', async () => {
  const f = await setup();
  const other = (await f.open()).provider;
  await f.provider.close();
  await f.provider.close();
  await assert.rejects(f.provider.search({ query: 'glider' }), /closed/);
  assert.deepEqual(await other.health(), { status: 'ready' });
});

test('prompt builder accepts Set, array, absent, and fallback tool lists', async () => {
  const f = await setup();
  for (const availableTools of [new Set(['memory_search']), ['memory_search']]) {
    assert.match(f.capability.promptBuilder({ availableTools }).join('\n'), /call memory_search\./);
  }
  for (const availableTools of [undefined, [], new Set(['mempalace_search'])]) {
    assert.match(f.capability.promptBuilder({ availableTools }).join('\n'), /call mempalace_search\./);
  }
});

test('currency failure before execution invokes no backend and revokes the handle', async () => {
  const f = await setup(session(owner), { staleAt: 3 });
  await assert.rejects(f.provider.search({ query: 'glider' }));
  await assert.rejects(f.provider.health());
  assert.deepEqual(await f.calls(), []);
});

test('missing config denies reads; get bounds are honored and unsupported revisions rejected', async () => {
  const missing = await setup(session(owner), { cfg: { peopleMap: '' } });
  await assert.rejects(missing.provider.search({ query: 'glider' }));
  await assert.rejects(missing.provider.get({ reference: ref('drawer-1') }));
  assert.deepEqual(await missing.calls(), []);
  const f = await setup();
  await assert.rejects(f.provider.get({ reference: { ...ref('drawer-1'), revision: 'invented-revision' } }));
  assert.deepEqual(await f.calls(), []);
  assert.deepEqual(await f.provider.get({ reference: ref('drawer-1'), from: 1, lines: 1 }), {
    status: 'ok', reference: ref('drawer-1'), text: 'Invented amber glider.', from: 1, lines: 1, truncated: false,
  });
});
