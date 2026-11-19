// Build relay (Cloudflare Worker): Solarbox boxes and the installer ask for an
// artifact; the Worker hands out the release asset if it is built, otherwise
// starts the on-demand GitHub build (.github/workflows/build.yml) with its own
// token and reports progress. Only combinations from the lists below are
// accepted — never arbitrary input reaches GitHub.
//
//   GET /v1/server/<arch>[?version=vX.Y.Z]      (no version = newest tag)
//   GET /v1/installer/<os-arch>[?version=vX.Y.Z]
//   → { status: "ready", version, files: { bin: {url, sig}, routeros?: {url, sig} } }
//   → { status: "building", version, run }      poll again in ~15 s
//   → { status: "failed", version, run }        the last build for it failed
//   → 400 { error } / 404 { error } / 502 { error }
//   GET /install.sh, /install.ps1   the launchers, from install/ on main

export const KINDS = {
  server: ['armv5', 'armv7', 'arm64', 'x86_64', 'mips', 'mipsel'],
  installer: ['linux-x86_64', 'linux-arm64', 'macos-arm64', 'macos-x86_64', 'windows-x86_64', 'windows-arm64', 'android-arm64'],
};
const ROUTEROS = new Set(['armv5', 'armv7', 'arm64', 'x86_64']);
const VERSION = /^v\d+\.\d+\.\d+$/;
const WORKFLOW = 'build.yml';
// A failed build is not retried for this long, so a broken combination
// does not start a new run on every poll.
const FAILED_HOLD_MS = 30 * 60 * 1000;

const json = (status, body, maxAge = 0) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': maxAge ? `public, max-age=${maxAge}` : 'no-store' },
  });

// Newest vX.Y.Z tag by semver.
export function newest(tags) {
  const key = (t) => t.slice(1).split('.').map(Number);
  return tags.filter((t) => VERSION.test(t)).sort((a, b) => {
    const x = key(a), y = key(b);
    return x[0] - y[0] || x[1] - y[1] || x[2] - y[2];
  }).pop();
}

export const title = (kind, arch, version) => `build ${kind} ${arch} ${version}`;

// Ready when the binary's signature is in the release (it is uploaded last).
export function readyFiles(assets, kind, arch) {
  const by = Object.fromEntries(assets.map((a) => [a.name, a.browser_download_url]));
  const pair = (name) => (by[name] && by[name + '.sig'] ? { url: by[name], sig: by[name + '.sig'] } : null);
  const base = `solarbox-${kind}-${arch}`;
  const bin = pair(base + (arch.startsWith('windows') ? '.exe' : ''));
  if (!bin) return null;
  const files = { bin };
  if (kind === 'server' && ROUTEROS.has(arch)) {
    const ros = pair(base + '.routeros.tar');
    if (!ros) return null;
    files.routeros = ros;
  }
  return files;
}

function github(env) {
  const api = async (path, init = {}) => {
    const r = await fetch(`https://api.github.com/repos/${env.REPO}${path}`, {
      ...init,
      headers: {
        authorization: `Bearer ${env.GH_TOKEN}`,
        accept: 'application/vnd.github+json',
        'user-agent': 'solarbox-relay',
        'x-github-api-version': '2022-11-28',
        ...(init.body ? { 'content-type': 'application/json' } : {}),
      },
    });
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`GitHub ${path}: ${r.status}`);
    return r.status === 204 ? {} : r.json();
  };
  return {
    tags: async () => ((await api('/tags?per_page=100')) || []).map((t) => t.name),
    tagExists: async (v) => !!(await api(`/git/ref/tags/${v}`)),
    assets: async (v) => (await api(`/releases/tags/${v}`))?.assets || [],
    runs: async () => (await api(`/actions/workflows/${WORKFLOW}/runs?per_page=50&event=workflow_dispatch`))?.workflow_runs || [],
    dispatch: (inputs) => api(`/actions/workflows/${WORKFLOW}/dispatches`, { method: 'POST', body: JSON.stringify({ ref: 'main', inputs }) }),
  };
}

export async function artifact(gh, kind, arch, version, now = Date.now()) {
  if (!KINDS[kind]) return json(404, { error: 'unknown path' });
  if (!KINDS[kind].includes(arch)) return json(400, { error: `unknown ${kind} target; one of ${KINDS[kind].join(', ')}` });
  if (version && !VERSION.test(version)) return json(400, { error: 'version must look like v1.2.3' });
  const asked = version;
  if (!version) {
    version = newest(await gh.tags());
    if (!version) return json(404, { error: 'no release yet' });
  } else if (!(await gh.tagExists(version))) {
    return json(404, { error: `no such version ${version}` });
  }

  const files = readyFiles(await gh.assets(version), kind, arch);
  // A built version never changes: cache it for an hour; "newest" for 5 min
  // so a new release shows up soon.
  if (files) return json(200, { status: 'ready', version, files }, asked ? 3600 : 300);

  const t = title(kind, arch, version);
  const last = (await gh.runs()).find((r) => r.display_title === t);
  // Polls within 10 s share one answer. A duplicate dispatch (the new run is not
  // listed yet) is harmless: build.yml queues it and it finds the asset built.
  if (last && last.status !== 'completed') return json(200, { status: 'building', version, run: last.html_url }, 10);
  if (last && last.conclusion === 'failure' && now - Date.parse(last.updated_at) < FAILED_HOLD_MS)
    return json(200, { status: 'failed', version, run: last.html_url });

  await gh.dispatch({ kind, arch, version });
  return json(200, { status: 'building', version, run: null }, 10);
}

export default {
  async fetch(req, env, ctx) {
    const url = new URL(req.url);
    if (req.method !== 'GET') return json(405, { error: 'GET only' });
    if (url.pathname === '/install.sh' || url.pathname === '/install.ps1') {
      const r = await fetch(`https://raw.githubusercontent.com/${env.REPO}/main/install${url.pathname}`, { cf: { cacheTtl: 300 } });
      if (!r.ok) return json(502, { error: `launcher: ${r.status}` });
      return new Response(r.body, { headers: { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'public, max-age=300' } });
    }
    const m = url.pathname.match(/^\/v1\/([a-z]+)\/([a-z0-9_-]+)$/);
    if (!m) return json(404, { error: 'unknown path' });

    // Same answer for everyone: serve repeated polls from the edge cache.
    const cache = caches.default, key = new Request(url.toString());
    const hit = await cache.match(key);
    if (hit) return hit;
    let res;
    try {
      res = await artifact(github(env), m[1], m[2], url.searchParams.get('version') || '');
    } catch (e) {
      return json(502, { error: String(e.message || e) });
    }
    if (res.headers.get('cache-control') !== 'no-store') ctx.waitUntil(cache.put(key, res.clone()));
    return res;
  },
};
