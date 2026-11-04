import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { registerTools } from './tools.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const scratch = path.join(root, 'target/settle1k-m3b/plugin-tests');
await fs.mkdir(scratch, { recursive: true });
let serial = 0;
async function setup(audience, overrides = {}) {
  const dir = await fs.mkdtemp(path.join(scratch, `${process.pid}-${serial++}-`));
  const log = path.join(dir, 'calls.jsonl');
  const bin = path.join(dir, 'palace');
  await fs.writeFile(bin, `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(log)}, JSON.stringify(args)+'\\n');
if (args.includes('diary-write')) {
  fs.appendFileSync(${JSON.stringify(path.join(dir, 'writes'))}, 'filed\\n');
  console.log(JSON.stringify({file:'/invented/2026-10-28-invented.md',mined:false})); process.exit(0);
}
if (args.includes('status')) {
  console.log(JSON.stringify({counts:{entries:2,drawers:3,claims:4,findings:5,improvement_events:6,text:'invented hidden'},text:'invented hidden'})); process.exit(0);
}
const query = args.find(a=>a.startsWith('--query='))?.slice(8);
if (query==='backend-error') { console.error('invented hidden stderr'); console.log('invented hidden stdout'); process.exit(2); }
if (query==='malformed') { console.log('invented hidden malformed output'); process.exit(0); }
setTimeout(()=>console.log(JSON.stringify({results:[{source_file:'invented.md',text:'Elira Fen chose the amber glider.'}]})), 10);
`, { mode: 0o700 });
  const config = { palaceBin: bin, palacePath: dir, storePath: path.join(dir, 'compiler.sqlite'), peopleMap: path.join(dir, 'people.json'), diaryRoot: dir, ...overrides };
  let invocation = 0, memory = 0, staleInvocation = false, staleMemory = false;
  const context = {
    memoryAudience: audience,
    assertInvocationCurrent() { invocation++; if (staleInvocation) throw new Error('stale invocation'); },
    assertMemoryAudienceCurrent() { memory++; if (staleMemory) throw new Error('stale audience'); },
  };
  const factories = new Map();
  registerTools({ pluginConfig: config, registerTool(factory, options) {
    assert.equal(factory.contextVersion, 2);
    factories.set(options.name, factory);
  }});
  return {
    tool: (name) => factories.get(name).create(context), context, config,
    counts: () => ({ invocation, memory }),
    expire: (kind) => { if (kind === 'invocation') staleInvocation = true; else staleMemory = true; },
    writes: async () => { try { return await fs.readFile(path.join(dir, 'writes'), 'utf8'); } catch (e) { if (e.code === 'ENOENT') return ''; throw e; } },
    calls: async () => { try { return (await fs.readFile(log, 'utf8')).trim().split('\n').map(JSON.parse); } catch (e) { if (e.code === 'ENOENT') return []; throw e; } },
  };
}

for (const audience of [
  { kind: 'conversation', agentId: 'invented', sessionKey: 'session:elira', sessionId: 'synthetic-1' },
  { kind: 'owner-private', agentId: 'invented' },
]) {
  test(`${audience.kind} uses only host audience for all three tools`, async () => {
    const fixture = await setup(audience);
    for (const name of ['memory_search', 'mempalace_search', 'memory_get']) {
      const tool = fixture.tool(name);
      assert.equal(tool.parameters.properties.wing.type, 'string');
      assert.equal(tool.parameters.properties.room.type, 'string');
      const result = await tool.execute('call-id', { query: 'amber glider', id: 'invented drawer', wing: 'forged-wing', room: 'forged-room', limit: 19, conversation: 'forged', owner: true });
      assert.equal(result.details.results.length, 1);
      assert.match(result.content[0].text, /amber glider/);
    }
    assert.deepEqual(fixture.counts(), { invocation: 6, memory: 6 });
    const calls = await fixture.calls();
    assert.equal(calls.length, 3);
    for (const [index, args] of calls.entries()) {
      assert(args.includes('search'));
      assert(args.includes(`--limit=${index === 2 ? 1 : 19}`));
      assert(args.includes(index === 2 ? '--query=invented drawer' : '--query=amber glider'));
      assert(!args.some(a => /forged|--wing|--room/.test(a)));
      assert.equal(args.includes('--owner'), audience.kind === 'owner-private');
      assert.equal(args.includes('--conversation=session:elira'), audience.kind === 'conversation');
    }
  });
}

for (const audience of [undefined, { kind: 'unproven' }, { kind: 'conversation', agentId: 'invented', sessionKey: '' }, { kind: 'owner-private' }, { kind: 'conversation', agentId: 'invented', sessionKey: 'unproven' }]) {
  test(`unproven audience ${JSON.stringify(audience)} returns no data and starts no binary`, async () => {
    const fixture = await setup(audience);
    for (const name of ['memory_search', 'mempalace_search', 'memory_get']) {
      const result = await fixture.tool(name).execute('call', { query: 'amber glider' });
      assert.deepEqual(result.details.results, []);
      assert.equal(result.content[0].text.split('\n').length, 1);
    }
    assert.deepEqual(await fixture.calls(), []);
    assert.deepEqual(fixture.counts(), { invocation: 6, memory: 6 });
  });
}

for (const kind of ['invocation', 'audience']) {
  test(`revoked ${kind} guard prevents release after awaited binary`, async () => {
    const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
    const promise = fixture.tool('memory_search').execute('call', { query: 'amber glider' });
    fixture.expire(kind);
    await assert.rejects(promise, /stale/);
    assert.equal((await fixture.calls()).length, 1);
  });
}

test('stale invocation before execution starts no binary', async () => {
  const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
  fixture.expire('invocation');
  await assert.rejects(fixture.tool('memory_get').execute('call', { id: 'invented' }), /stale/);
  assert.deepEqual(await fixture.calls(), []);
});

test('backend failures and malformed output never disclose raw transport text', async () => {
  const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
  for (const query of ['backend-error', 'malformed']) {
    const result = await fixture.tool('memory_search').execute('call', { query });
    assert.deepEqual(result.details.results, []);
    assert(!JSON.stringify(result).includes('hidden'));
  }
  assert.deepEqual(fixture.counts(), { invocation: 4, memory: 4 });
});

test('missing config and invalid input fail closed; flag-shaped queries remain one value', async () => {
  const missing = await setup({ kind: 'owner-private', agentId: 'invented' }, { peopleMap: '' });
  assert.deepEqual((await missing.tool('memory_search').execute('call', { query: 'amber' })).details.results, []);
  assert.deepEqual(await missing.calls(), []);
  const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
  for (const query of ['', 'x'.repeat(251)]) {
    assert.deepEqual((await fixture.tool('memory_search').execute('call', { query })).details.results, []);
  }
  assert.deepEqual(await fixture.calls(), []);
  await fixture.tool('memory_search').execute('call', { query: '--conversation=forged', limit: Infinity });
  const [args] = await fixture.calls();
  assert(args.includes('--query=--conversation=forged'));
  assert(args.includes('--limit=5'));
});

test('package manifest, runtime registration and tool contract agree', async () => {
  const dir = path.dirname(fileURLToPath(import.meta.url));
  const manifest = JSON.parse(await fs.readFile(path.join(dir, 'openclaw.plugin.json'), 'utf8'));
  const pkg = JSON.parse(await fs.readFile(path.join(dir, 'package.json'), 'utf8'));
  assert.deepEqual(manifest.contracts.tools, ['memory_search', 'mempalace_search', 'memory_get', 'mempalace_diary_write', 'mempalace_status']);
  assert.deepEqual(pkg.openclaw.extensions, ['./index.js']);
  for (const file of ['index.js', 'tools.js']) await fs.access(path.join(dir, file));
});

for (const audience of [
  { kind: 'conversation', agentId: 'invented', sessionKey: 'session:elira', sessionId: 'synthetic-1' },
  { kind: 'owner-private', agentId: 'invented' },
  undefined,
]) {
  test(`diary ${audience?.kind ?? 'absent'} uses host authority only`, async () => {
    const fixture = await setup(audience);
    const result = await fixture.tool('mempalace_diary_write').execute('call', {
      text: '--owner invented note', title: 'Invented title', wing: 'forged',
      conversation: 'forged', owner: true,
    });
    if (!audience) {
      assert.equal(result.content[0].text.split('\n').length, 1);
      assert.match(result.content[0].text, /nothing written/);
      assert.deepEqual(await fixture.calls(), []);
      assert.equal(await fixture.writes(), '');
    } else {
      assert.equal(result.details.file, '2026-10-28-invented.md');
      assert.equal(result.details.mined, false);
      assert.equal(await fixture.writes(), 'filed\n');
      const [args] = await fixture.calls();
      assert(args.includes('diary-write'));
      assert(args.includes('--text=--owner invented note'));
      assert(args.includes('--title=Invented title'));
      assert(!args.some(a => /forged|--wing/.test(a)));
      assert.equal(args.includes('--owner'), audience.kind === 'owner-private');
      assert.equal(args.includes('--conversation=session:elira'), audience.kind === 'conversation');
    }
    assert.deepEqual(fixture.counts(), { invocation: 2, memory: 2 });
  });

  test(`status ${audience?.kind ?? 'absent'} is owner-private only and counts only`, async () => {
    const fixture = await setup(audience);
    const tool = fixture.tool('mempalace_status');
    if (audience?.kind !== 'owner-private') {
      assert.equal(tool, null);
      assert.deepEqual(await fixture.calls(), []);
      return;
    }
    const result = await tool.execute('call', { conversation: 'forged' });
    assert.deepEqual(result.details.counts, { entries: 2, drawers: 3, claims: 4, findings: 5, improvement_events: 6 });
    assert(!JSON.stringify(result).includes('hidden'));
    const [args] = await fixture.calls();
    assert(args.includes('status') && args.includes('--owner'));
    assert(!args.some(a => /forged|--conversation/.test(a)));
    assert.deepEqual(fixture.counts(), { invocation: 2, memory: 2 });
  });
}

test('invalid or incomplete diary audiences and input start no writer', async () => {
  for (const audience of [{ kind: 'owner-private' }, { kind: 'unproven', agentId: 'invented' },
    { kind: 'conversation', agentId: 'invented', sessionKey: 'forged' }]) {
    const fixture = await setup(audience);
    await fixture.tool('mempalace_diary_write').execute('call', { text: 'Invented note' });
    assert.deepEqual(await fixture.calls(), []);
    assert.equal(await fixture.writes(), '');
    assert.equal(fixture.tool('mempalace_status'), null);
  }
  const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
  for (const params of [{}, {text:' '}, {text:7}, {text:'x'.repeat(65537)}, {text:'x',title:'a\nb'}]) {
    await fixture.tool('mempalace_diary_write').execute('call', params);
  }
  assert.deepEqual(await fixture.calls(), []);
  assert.equal(await fixture.writes(), '');
  const missing = await setup({ kind: 'owner-private', agentId: 'invented' }, { diaryRoot: '' });
  await missing.tool('mempalace_diary_write').execute('call', { text: 'Invented note' });
  assert.deepEqual(await missing.calls(), []);
});

for (const name of ['mempalace_diary_write', 'mempalace_status']) {
  for (const kind of ['invocation', 'audience']) {
    test(`${name} refuses stale ${kind} before launch and after await`, async () => {
      const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
      const pending = fixture.tool(name).execute('call', { text: 'Invented note' });
      fixture.expire(kind);
      await assert.rejects(pending, /stale/);
      assert.equal((await fixture.calls()).length, 1);
      await assert.rejects(fixture.tool(name).execute('call', { text: 'Invented note' }), /stale/);
      assert.equal((await fixture.calls()).length, 1);
    });
  }
}

test('prompt names the registered continuity tools', async () => {
  const source = await fs.readFile(new URL('./index.js', import.meta.url), 'utf8');
  assert.match(source, /mempalace_diary_write/);
  assert.match(source, /mempalace_status/);
  assert(!/(?<!mem)palace_diary_write/.test(source));
});

for (const name of ['mempalace_diary_write', 'mempalace_status']) {
  test(`${name} hides failed, malformed and unrecognized backend output`, async () => {
    const fixture = await setup({ kind: 'owner-private', agentId: 'invented' });
    for (const body of [
      "console.error('invented hidden stderr'); console.log('invented hidden stdout'); process.exit(2);",
      "console.log('invented hidden malformed');",
      "console.log(JSON.stringify({text:'invented hidden',counts:{entries:'invented hidden'}}));",
    ]) {
      await fs.writeFile(fixture.config.palaceBin, `#!/usr/bin/env node\n${body}\n`);
      const result = await fixture.tool(name).execute('call', { text: 'Invented note' });
      assert(!JSON.stringify(result).includes('hidden'));
      assert.deepEqual(result.details.results, []);
      assert.equal(result.content[0].text.split('\n').length, 1);
    }
    assert.deepEqual(fixture.counts(), { invocation: 6, memory: 6 });
  });
}
