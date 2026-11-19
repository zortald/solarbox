// node --test relay/test.mjs — the relay logic against a fake GitHub.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { server, newest, readyFiles } from './src/index.js';

const asset = (name) => ({ name, browser_download_url: `https://dl/${name}` });
const built = (arch) => [`solarbox-server-${arch}`, `solarbox-server-${arch}.sig`, `solarbox-server-${arch}.routeros.tar`, `solarbox-server-${arch}.routeros.tar.sig`].map(asset);
function fake({ tags = ['v0.1.0'], assets = [], runs = [] } = {}) {
  const gh = {
    dispatched: [],
    tags: async () => tags,
    tagExists: async (v) => tags.includes(v),
    assets: async () => assets,
    runs: async () => runs,
    dispatch: async (i) => { gh.dispatched.push(i); return {}; },
  };
  return gh;
}
const body = async (r) => ({ code: r.status, cache: r.headers.get('cache-control'), ...(await r.json()) });

test('newest tag by semver', () => {
  assert.equal(newest(['v0.9.0', 'v0.10.0', 'v0.2.1', 'junk']), 'v0.10.0');
  assert.equal(newest(['junk']), undefined);
});

test('ready only with every file and its signature', () => {
  assert.ok(readyFiles(built('armv5'), 'server', 'armv5'));
  assert.equal(readyFiles(built('armv5').slice(0, 3), 'server', 'armv5'), null); // tar signature missing
  assert.ok(readyFiles(['solarbox-server-mips', 'solarbox-server-mips.sig'].map(asset), 'server', 'mips')); // no RouterOS image for mips
});

test('built → links, cached; nothing dispatched', async () => {
  const gh = fake({ assets: built('armv5') });
  const r = await body(await server(gh, 'armv5', 'v0.1.0'));
  assert.equal(r.status, 'ready');
  assert.equal(r.files.routeros.sig, 'https://dl/solarbox-server-armv5.routeros.tar.sig');
  assert.match(r.cache, /max-age=3600/);
  assert.equal(gh.dispatched.length, 0);
  assert.match((await body(await server(gh, 'armv5', ''))).cache, /max-age=300/); // newest: short cache
});

test('not built, nothing running → one dispatch', async () => {
  const gh = fake();
  const r = await body(await server(gh, 'x86_64', ''));
  assert.equal(r.status, 'building');
  assert.equal(r.version, 'v0.1.0');
  assert.deepEqual(gh.dispatched, [{ kind: 'server', arch: 'x86_64', version: 'v0.1.0' }]);
});

test('already running → no second dispatch', async () => {
  const gh = fake({ runs: [{ display_title: 'build server arm64 v0.1.0', status: 'in_progress', html_url: 'run/1' }] });
  const r = await body(await server(gh, 'arm64', 'v0.1.0'));
  assert.equal(r.status, 'building');
  assert.equal(r.run, 'run/1');
  assert.equal(gh.dispatched.length, 0);
});

test('recent failure is reported, not retried; an old one is retried', async () => {
  const now = Date.parse('2026-11-19T12:00:00Z');
  const run = (at) => [{ display_title: 'build server mips v0.1.0', status: 'completed', conclusion: 'failure', updated_at: at, html_url: 'run/2' }];
  const gh = fake({ runs: run('2026-11-19T11:50:00Z') });
  assert.equal((await body(await server(gh, 'mips', 'v0.1.0', now))).status, 'failed');
  assert.equal(gh.dispatched.length, 0);
  const old = fake({ runs: run('2026-11-19T10:00:00Z') });
  assert.equal((await body(await server(old, 'mips', 'v0.1.0', now))).status, 'building');
  assert.equal(old.dispatched.length, 1);
});

test('bad input never reaches GitHub', async () => {
  const gh = fake();
  assert.equal((await server(gh, 'riscv', '')).status, 400);
  assert.equal((await server(gh, 'armv5', 'main')).status, 400);
  assert.equal((await server(gh, 'armv5', 'v9.9.9')).status, 404);
  assert.equal(gh.dispatched.length, 0);
});
