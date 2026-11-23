// Copies site/office/status.json into the fallback block in site/office/index.html.
// Usage:  node office/sync-status.mjs          (write)
//         node office/sync-status.mjs --check  (exit 1 if out of sync; for CI)
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const served = join(here, '..', 'site', 'office');
const jsonPath = join(served, 'status.json');
const htmlPath = join(served, 'index.html');

const raw = readFileSync(jsonPath, 'utf8');
const data = JSON.parse(raw); // throws on invalid JSON, which is what we want

const statuses = new Set(['working', 'done', 'waiting', 'idle']);
const problems = [];
if (!Array.isArray(data.agents)) problems.push('agents must be an array');
for (const a of data.agents || []) {
  for (const k of ['id', 'name', 'role', 'status', 'updated_at']) if (!a[k]) problems.push(`${a.id || '?'}: missing ${k}`);
  if (a.status && !statuses.has(a.status)) problems.push(`${a.id}: status "${a.status}" is not one of ${[...statuses].join(', ')}`);
  if (a.last_result && a.last_result.url && !/^https:\/\//.test(a.last_result.url)) problems.push(`${a.id}: last_result.url must start with https://`);
  if (a.updated_at && isNaN(Date.parse(a.updated_at))) problems.push(`${a.id}: updated_at is not a date`);
}
if (!Array.isArray(data.owner_queue)) problems.push('owner_queue must be an array');
if (problems.length) {
  console.error('status.json problems:\n- ' + problems.join('\n- '));
  process.exit(1);
}

// "</" would end the script block early; escape it.
const embedded = JSON.stringify(data, null, 2).replace(/<\//g, '<\\/');
const html = readFileSync(htmlPath, 'utf8');
const re = /(<script type="application\/json" id="status-fallback">\n)([\s\S]*?)(\n\s*<\/script>)/;
if (!re.test(html)) { console.error('fallback block not found in index.html'); process.exit(1); }
const current = html.match(re)[2];

if (process.argv.includes('--check')) {
  let same = false;
  try { same = JSON.stringify(JSON.parse(current)) === JSON.stringify(data); } catch { same = false; }
  if (!same) { console.error('index.html fallback is out of date. Run: node office/sync-status.mjs'); process.exit(1); }
  console.log('fallback in sync');
} else {
  writeFileSync(htmlPath, html.replace(re, (_, a, _b, c) => a + embedded + c));
  console.log(`embedded ${data.agents.length} agents and ${data.owner_queue.length} owner items into index.html`);
}
