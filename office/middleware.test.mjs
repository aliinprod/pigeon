// node --test office/middleware.test.mjs
// Proves SEC-31: the /office login gate has no credentials in code, reads only
// OFFICE_USER_SHA256 / OFFICE_PASS_SHA256, and fails closed (503) when they are
// missing or malformed. Test credentials are generated per run and never stored.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash, randomBytes } from 'node:crypto';

const src = readFileSync(new URL('../functions/_middleware.js', import.meta.url), 'utf8');
const { onRequest } = await import('data:text/javascript;base64,' + Buffer.from(src).toString('base64'));

const hex = (s) => createHash('sha256').update(s, 'utf8').digest('hex');
const USER = 'u-' + randomBytes(6).toString('hex');
const PASS = randomBytes(18).toString('base64url');
const ENV = { OFFICE_USER_SHA256: hex(USER), OFFICE_PASS_SHA256: hex(PASS) };
const basic = (u, p) => 'Basic ' + Buffer.from(`${u}:${p}`, 'utf8').toString('base64');

async function call(path, opts = {}) {
  const env = 'env' in opts ? opts.env : ENV;
  const { auth } = opts;
  const headers = auth ? { Authorization: auth } : {};
  const request = new Request('https://flypigeon.pages.dev' + path, { headers });
  let nextCalled = false;
  const next = async () => { nextCalled = true; return new Response('PAGE', { status: 200 }); };
  const res = await onRequest({ request, env, next });
  return { status: res.status, body: await res.text(), nextCalled };
}

test('no hardcoded credential hashes or defaults in the middleware source', () => {
  assert.doesNotMatch(src, /\b[0-9a-f]{64}\b/i, 'a 64-hex value (hash) is in the source');
  assert.doesNotMatch(src, /DEFAULT_(USER|PASS)/, 'a default credential constant is in the source');
});

test('fails closed with 503 when the env vars are missing, empty or malformed', async () => {
  const cases = [
    undefined,
    {},
    { OFFICE_USER_SHA256: ENV.OFFICE_USER_SHA256 },
    { OFFICE_PASS_SHA256: ENV.OFFICE_PASS_SHA256 },
    { OFFICE_USER_SHA256: '', OFFICE_PASS_SHA256: '' },
    { OFFICE_USER_SHA256: 'abc', OFFICE_PASS_SHA256: ENV.OFFICE_PASS_SHA256 },
    { OFFICE_USER_SHA256: ENV.OFFICE_USER_SHA256, OFFICE_PASS_SHA256: 'z'.repeat(64) },
  ];
  for (const env of cases) {
    for (const auth of [undefined, basic(USER, PASS), basic('', '')]) {
      const r = await call('/office/', { env, auth });
      assert.equal(r.status, 503, `env=${JSON.stringify(env)} auth=${!!auth}`);
      assert.equal(r.nextCalled, false, 'the page must never be served');
    }
  }
});

test('with env set: 401 without or with wrong credentials, page with the right ones', async () => {
  assert.equal((await call('/office/')).status, 401);
  assert.equal((await call('/office/', { auth: basic(USER, 'wrong') })).status, 401);
  assert.equal((await call('/office/', { auth: basic('wrong', PASS) })).status, 401);
  const ok = await call('/office/status.json', { auth: basic(USER, PASS) });
  assert.equal(ok.status, 200);
  assert.equal(ok.nextCalled, true);
});

test('encoded and odd paths to the office are still gated', async () => {
  for (const p of ['/office', '/OFFICE/', '/%6Fffice/', '/%256Fffice/', '/x/../office/', '//office//index.html', '/office.css']) {
    const r = await call(p);
    assert.equal(r.status, 401, p);
    assert.equal(r.nextCalled, false, p);
  }
});

test('the rest of the site is not gated, even with no env configured', async () => {
  const r = await call('/', { env: {} });
  assert.equal(r.status, 200);
  assert.equal(r.nextCalled, true);
});
