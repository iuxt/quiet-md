import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./index.js', import.meta.url), 'utf8');
const { collect, key } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('missing days are retried after later days succeed', async () => {
  const runs = [], calls = [];
  for (const source of ['cloudflare', 'site']) {
    for (let d = new Date('2026-10-05'); d < new Date('2026-11-01'); d.setUTCDate(d.getUTCDate() + 1)) {
      runs.push({ source, day: d.toISOString().slice(0, 10) });
    }
  }
  const DB = {
    prepare(sql) {
      const statement = (args = []) => ({
        bind(...values) { return statement(values); },
        async first() { return runs.filter((r) => r.source === args[0] && !r.error).map((r) => r.day).sort().at(-1) || null; },
        async all() {
          return { results: sql.includes('SELECT DISTINCT day') ? runs.filter((r) => r.source === args[0] && !r.error && r.day >= args[1]) : [] };
        },
        async run() {
          if (sql.startsWith('INSERT INTO runs')) runs.push({ source: args[1], day: args[2], error: sql.includes('error)') ? args[3] : null });
        },
      });
      return statement();
    },
    async batch() {},
  };
  const originalFetch = globalThis.fetch;
  let failing = true;
  globalThis.fetch = async (url, options) => {
    if (!url.includes('graphql')) return { ok: true, json: async () => url.includes('/releases?') ? [] : {} };
    const { query, variables } = JSON.parse(options.body);
    const day = variables.from.slice(0, 10);
    const source = query.includes('clientRequestPath: "/"') ? 'site' : 'cloudflare';
    calls.push({ source, day });
    if (failing && day === '2026-11-01') return { ok: false, status: 503, json: async () => ({ errors: ['temporary failure'] }) };
    return { ok: true, json: async () => ({ data: { viewer: { zones: [{ httpRequestsAdaptiveGroups: [] }] } } }) };
  };
  try {
    const now = new Date('2026-11-04T00:30:00Z');
    await collect({ DB, MAX_DAYS: 3 }, now);
    assert.deepEqual(calls.filter((c) => c.source === 'cloudflare').map((c) => c.day), ['2026-11-01', '2026-11-02', '2026-11-03']);
    calls.length = 0;
    failing = false;
    await collect({ DB, MAX_DAYS: 3 }, now);
    assert.deepEqual(calls, [{ source: 'cloudflare', day: '2026-11-01' }, { source: 'site', day: '2026-11-01' }]);
    calls.length = 0;
    await collect({ DB, MAX_DAYS: 3 }, now);
    assert.deepEqual(calls, []);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('unsigned macOS packages count without attributing older downloads to today', async () => {
  const windows = 'quiet-md-1.2.0-windows-x64-setup.exe';
  const arm = 'quiet-md-1.2.0-macos-arm64-unsigned.zip';
  const intel = 'quiet-md-1.2.0-macos-x86_64-unsigned.zip';
  const windowsDims = key({ platform: 'windows-x64', version: '1.2.0', asset: windows });
  let previousDay = '2026-11-25';
  let previous = [{ dims: windowsDims, value: 50, collected_at: '2026-11-26T00:30:00Z' }];
  let rows = [];
  const DB = {
    prepare(sql) {
      return { bind(...args) { return { sql, args,
        all: async () => ({ results: sql.includes('SELECT dims, value') ? previous : [] }),
        first: async () => previousDay,
        run: async () => {},
      }; } };
    },
    async batch(stmts) { rows.push(...stmts.filter(s => s.sql.startsWith('INSERT INTO metrics')).map(s => s.args)); },
  };
  const originalFetch = globalThis.fetch;
  let downloads = [70, 100, 20];
  globalThis.fetch = async url => ({
    ok: true,
    json: async () => url.includes('/releases?') ? [{ tag_name: 'v1.2.0', assets: [windows, arm, intel, arm + '.sha256'].map((name, i) => ({
      name, download_count: downloads[i] ?? 200, created_at: i === 2 ? '2026-11-26T12:00:00Z' : '2026-11-20T12:00:00Z',
    })) }] : { stargazers_count: 1, forks_count: 1, subscribers_count: 1, open_issues_count: 1 },
  });
  const metric = name => Object.fromEntries(rows.filter(row => row[1] === name).map(row => [JSON.parse(row[2]).platform, row[3]]));
  try {
    const env = { DB, MAX_DAYS: '0' };
    await collect(env, new Date('2026-11-27T00:30:00Z'));
    assert.deepEqual(metric('github.downloads_total'), { 'windows-x64': 70, 'macos-arm64': 100, 'macos-x86_64': 20 });
    assert.deepEqual(metric('github.downloads'), { 'windows-x64': 20, 'macos-x86_64': 20 });
    previous = rows.filter(row => row[1] === 'github.downloads_total').map(row => ({ dims: row[2], value: row[3], collected_at: row[5] }));
    previousDay = '2026-11-26';
    rows = [];
    downloads = [75, 108, 24];
    await collect(env, new Date('2026-11-28T00:30:00Z'));
    assert.deepEqual(metric('github.downloads'), { 'windows-x64': 5, 'macos-arm64': 8, 'macos-x86_64': 4 });
  } finally {
    globalThis.fetch = originalFetch;
  }
});
