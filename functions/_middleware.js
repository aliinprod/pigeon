// Login gate for /office and everything under it (page, scripts, status.json).
// Cloudflare Pages Functions middleware, HTTP Basic Auth over HTTPS.
//
// Why this sits at the root (functions/_middleware.js) and not in functions/office/:
// a percent-encoded path such as /%6Fffice/ does not match the /office/* function route,
// but the static asset server still decodes it and serves /office/index.html. So every
// request passes through here, and the gate is applied to the decoded, normalised path.
// Requests outside /office go straight to the static site.
//
// Credentials are stored only as SHA-256 hex hashes, and ONLY in the Cloudflare Pages
// project (Settings -> Variables and Secrets): OFFICE_USER_SHA256 and OFFICE_PASS_SHA256.
// There are no defaults in code. This repository is public; a hash in it would be
// brute-forced offline in seconds. If either variable is missing or malformed, every
// /office request gets 503 (fail closed), never the page. See office/README.md.
// Never put a username, password or hash in this repo.

const REALM = 'Pigeon office';
const HEX64 = /^[0-9a-f]{64}$/;
const enc = new TextEncoder();

function plain(status, body, extra = {}) {
  return new Response(body, {
    status,
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': 'no-store',
      'X-Robots-Tag': 'noindex, nofollow',
      ...extra,
    },
  });
}
const unauthorized = () =>
  plain(401, 'Login required.', { 'WWW-Authenticate': `Basic realm="${REALM}", charset="UTF-8"` });

async function sha256(text) {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', enc.encode(text)));
}
function hexToBytes(hex) {
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}
// Constant time for equal-length inputs (both are always 32-byte digests here).
function sameBytes(a, b) {
  if (crypto.subtle.timingSafeEqual) return crypto.subtle.timingSafeEqual(a, b);
  let diff = a.length ^ b.length;
  for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
  return diff === 0;
}

function readBasic(header) {
  if (!header) return null;
  const m = /^Basic\s+([A-Za-z0-9+/=]+)\s*$/i.exec(header);
  if (!m) return null;
  let bin;
  try { bin = atob(m[1]); } catch { return null; }
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  let text;
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); } catch { return null; }
  const i = text.indexOf(':');
  if (i < 0) return null;
  return { user: text.slice(0, i), pass: text.slice(i + 1) };
}

// Decode (repeatedly, to defeat double encoding), fold case, collapse slashes and dot segments.
// Returns null when the path cannot be decoded; such requests are treated as protected.
function normalisedPath(pathname) {
  let p = pathname;
  for (let i = 0; i < 4; i++) {
    let next;
    try { next = decodeURIComponent(p); } catch { return null; }
    if (next === p) break;
    p = next;
  }
  p = p.replace(/\\/g, '/').toLowerCase();
  const out = [];
  for (const seg of p.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') out.pop(); else out.push(seg);
  }
  return '/' + out.join('/');
}
function isProtected(pathname) {
  const p = normalisedPath(pathname);
  if (p === null) return true;
  return p === '/office' || p.startsWith('/office/') || p.startsWith('/office.');
}

export async function onRequest(context) {
  const { request, env } = context;
  const url = new URL(request.url);
  if (!isProtected(url.pathname)) return context.next();

  // Basic Auth only over HTTPS (local dev on localhost is the exception).
  const local = url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]';
  if (url.protocol !== 'https:' && !local) {
    url.protocol = 'https:';
    return Response.redirect(url.toString(), 308);
  }

  const userHex = String((env && env.OFFICE_USER_SHA256) || '').trim().toLowerCase();
  const passHex = String((env && env.OFFICE_PASS_SHA256) || '').trim().toLowerCase();
  // Fail closed: a missing or malformed hash never lets anyone in.
  if (!HEX64.test(userHex) || !HEX64.test(passHex)) return plain(503, 'Office login is not configured.');

  const creds = readBasic(request.headers.get('Authorization'));
  if (!creds) return unauthorized();

  const [u, p] = await Promise.all([sha256(creds.user), sha256(creds.pass)]);
  const okUser = sameBytes(u, hexToBytes(userHex));
  const okPass = sameBytes(p, hexToBytes(passHex));   // always evaluated, no short-circuit
  if (!(okUser & okPass)) return unauthorized();

  const res = await context.next();
  const out = new Response(res.body, res);
  out.headers.set('Cache-Control', 'no-store');
  out.headers.set('X-Robots-Tag', 'noindex, nofollow');
  out.headers.set('Referrer-Policy', 'no-referrer');
  out.headers.append('Vary', 'Authorization');
  return out;
}
