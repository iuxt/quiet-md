import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./index.js', import.meta.url), 'utf8');
const { collect } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

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
