// Builds site/office/status.json from real work, at deploy time. Read-only on GitHub.
//
// Source A: GitHub (open PRs, PRs merged in the last 7 days, CI results per PR and on main).
// Source B: Atlas's notes in the open issue titled "Office status", one line per bot:
//             anvil: working | One key per computer (both roles)
//             owner: Test pairing build on the office Mac
//
// The committed site/office/status.json is the roster (ids, names, roles, about) and the
// default when a source says nothing about a bot. Output keeps the same schema.
//
// Usage:
//   node office/build-status.mjs                    write site/office/status.json
//   node office/build-status.mjs --out FILE         write somewhere else
//   node office/build-status.mjs --print            print, write nothing
//   node office/build-status.mjs --fingerprint      also print "fingerprint=<sha256>" (content minus timestamps)
//   node office/build-status.mjs --repo OWNER/NAME  default: $GITHUB_REPOSITORY, else `gh repo view`
//   node office/build-status.mjs --owner LOGIN      only PRs by this account count (default aliinprod)
//   node office/build-status.mjs --notes FILE       local preview: read the notes from FILE instead of the issue
// Needs `gh` logged in (locally) or GH_TOKEN (in Actions). Then run office/sync-status.mjs
// to refresh the fallback copy in index.html.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const ISSUE_TITLE = 'Office status';
export const STATUSES = ['working', 'done', 'waiting', 'idle'];
const MERGED_DAYS = 7;
const MAX_TEXT = 160;

// Fallback when a PR has no `bot:<id>` label: branch prefixes seen in this repo.
// Longest match wins. A branch named `<id>/...` or `<id>-...` always maps to that bot.
export const BRANCH_PREFIXES = {
  'agent-office': 'plume', 'office-': 'plume', 'site-': 'plume', 'web-': 'plume',
  'app-': 'canvas', 'ui-': 'canvas',
  'security-': 'warden', 'threat-': 'warden',
  'verify-': 'proof', 'test-': 'proof',
  'promo-': 'frame', 'video-': 'frame',
  'devlog-': 'atlas', 'post-': 'atlas', 'posts-': 'atlas',
  'day-': 'anvil', 'week': 'anvil', 'linux-': 'anvil', 'core-': 'anvil', 'cli-': 'anvil',
  'relay-': 'anvil', 'pairing-': 'anvil', 'one-key': 'anvil',
};

// ---------- text safety ----------
// Everything from GitHub is untrusted text. The page renders it with textContent only, but we
// also strip control/bidi characters, collapse whitespace and cap the length here.
export function cleanText(s, max = MAX_TEXT) {
  if (typeof s !== 'string') return null;
  const t = s
    .replace(/[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁦-⁩﻿]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (!t) return null;
  return t.length > max ? t.slice(0, max - 1).trimEnd() + '…' : t;
}
// Only https links to github.com (PRs, runs, issues). Anything else is dropped.
export function cleanUrl(u) {
  if (typeof u !== 'string') return null;
  try {
    const x = new URL(u);
    return x.protocol === 'https:' && x.hostname === 'github.com' ? x.toString() : null;
  } catch { return null; }
}

// ---------- Source B: the issue ----------
const LINE = /^([a-z][a-z0-9-]{0,31})\s*:\s*(working|done|waiting|idle)\s*(?:\|\s*(.*))?$/;
const OWNER = /^owner\s*:\s*(.+)$/;
// Strict: one bot per line, known ids only, anything else is ignored (headings, prose, typos).
export function parseNotes(body, knownIds) {
  const bots = {}, owner = [];
  for (const raw of String(body || '').split(/\r?\n/)) {
    const line = raw.trim().replace(/^[-*]\s+/, '').replace(/^`(.*)`$/, '$1').trim();
    const o = OWNER.exec(line);
    if (o) { const t = cleanText(o[1]); if (t) owner.push(t); continue; }
    const m = LINE.exec(line);
    if (!m || !knownIds.has(m[1]) || bots[m[1]]) continue;
    bots[m[1]] = { status: m[2], task: cleanText(m[3] || '') };
  }
  return { bots, owner };
}

// ---------- Source A: PRs and CI ----------
export function botForPr(pr, knownIds) {
  for (const l of pr.labels || []) {
    const m = /^bot:([a-z][a-z0-9-]*)$/.exec((l && l.name) || '');
    if (m && knownIds.has(m[1])) return m[1];
  }
  const branch = String(pr.headRefName || '').toLowerCase();
  for (const id of knownIds) if (branch.startsWith(id + '/') || branch.startsWith(id + '-')) return id;
  let best = null;
  for (const [p, id] of Object.entries(BRANCH_PREFIXES)) {
    if (branch.startsWith(p) && knownIds.has(id) && (!best || p.length > best[0].length)) best = [p, id];
  }
  if (best) return best[1];
  // Last resort: "(Frame)" in the title, or a "Bot: frame" line in the body.
  for (const id of knownIds) {
    if (new RegExp(`\\(${id}\\)`, 'i').test(pr.title || '')) return id;
    if (new RegExp(`^\\s*bot\\s*:\\s*${id}\\s*$`, 'im').test(pr.body || '')) return id;
  }
  return null;
}

const RED = new Set(['FAILURE', 'CANCELLED', 'TIMED_OUT', 'ACTION_REQUIRED', 'STARTUP_FAILURE', 'ERROR']);
// statusCheckRollup items are CheckRun {status, conclusion, detailsUrl} or StatusContext {state, targetUrl}.
export function rollup(checks) {
  let pending = false, fail = null, any = false;
  for (const c of checks || []) {
    any = true;
    const url = c.detailsUrl || c.targetUrl || null;
    if (c.__typename === 'StatusContext' || c.state) {
      const s = String(c.state || '').toUpperCase();
      if (RED.has(s)) fail = fail || url; else if (s !== 'SUCCESS') pending = true;
      continue;
    }
    if (String(c.status || '').toUpperCase() !== 'COMPLETED') { pending = true; continue; }
    if (RED.has(String(c.conclusion || '').toUpperCase())) fail = fail || url;
  }
  if (fail !== null) return { state: 'failure', url: fail };
  if (pending) return { state: 'pending', url: null };
  return { state: any ? 'success' : 'none', url: null };
}

// ---------- trust ----------
// The repo is public: anyone can open a PR from a fork with a branch like "site-x" or a title
// like "(Plume)". Only PRs opened by the owner account from a branch of this repo count;
// everything else is ignored, so a stranger's PR never shows in the office or the owner queue.
export const OWNER_LOGIN = 'aliinprod';
export function trustedPr(pr, repo, owner = OWNER_LOGIN) {
  const [repoOwner, repoName] = String(repo || '').toLowerCase().split('/');
  const login = (o) => String((o && o.login) || '').toLowerCase();
  return !!pr && !!repoOwner && !!repoName
    && pr.isCrossRepository === false
    && login(pr.headRepositoryOwner) === repoOwner
    && String((pr.headRepository && pr.headRepository.name) || '').toLowerCase() === repoName
    && login(pr.author) === String(owner).toLowerCase();
}

// ---------- merge ----------
// roster: committed status.json. notes: parseNotes() result plus issue meta. prs: {open, merged}.
// repo: "owner/name". PRs that fail trustedPr() are dropped before anything else.
export function merge({ roster, notes, issue, prs, mainCi, repo, owner = OWNER_LOGIN, now }) {
  const ids = new Set(roster.agents.map((a) => a.id));
  const repoUrl = 'https://github.com/' + repo;
  prs = {
    open: (prs.open || []).filter((p) => trustedPr(p, repo, owner)),
    merged: (prs.merged || []).filter((p) => trustedPr(p, repo, owner)),
  };
  const byBot = {};
  for (const id of ids) byBot[id] = { open: [], merged: [] };
  for (const pr of prs.open) { const b = botForPr(pr, ids); if (b) byBot[b].open.push({ ...pr, ci: rollup(pr.statusCheckRollup) }); }
  for (const pr of prs.merged) { const b = botForPr(pr, ids); if (b) byBot[b].merged.push(pr); }
  for (const id of ids) byBot[id].merged.sort((a, b) => Date.parse(b.mergedAt) - Date.parse(a.mergedAt));

  const ownerQueue = [];
  const agents = roster.agents.map((base) => {
    const a = { ...base };
    const note = notes.bots[a.id];
    const mine = byBot[a.id];
    const red = mine.open.filter((p) => p.ci.state === 'failure');
    const green = mine.open.filter((p) => p.ci.state === 'success' && !p.isDraft);
    const pend = mine.open.filter((p) => p.ci.state === 'pending' || p.ci.state === 'none' || p.isDraft);
    const lastMerged = mine.merged[0];
    const times = [];

    // B: Atlas's notes give status and task.
    if (note) {
      a.status = note.status;
      a.task = note.task || (note.status === 'idle' ? null : a.task || null);
      if (issue && issue.updated_at) times.push(issue.updated_at);
    } else if (red.length || green.length) {
      a.status = 'waiting';
    } else if (pend.length) {
      a.status = 'working';
    } else if (lastMerged) {
      a.status = 'done';
      a.task = cleanText(lastMerged.title);
    }

    // A: links and results. Red CI always wins and sets "waiting" (the "!" in the scene).
    const pr = red[0] || green[0] || pend[0] || null;
    if (red.length) {
      a.status = 'waiting';
      const p = red[0];
      a.last_result = { text: cleanText(`PR #${p.number} CI failing: ${p.title}`), url: cleanUrl(p.ci.url) || cleanUrl(p.url) };
    } else if (green.length) {
      const p = green[0];
      a.last_result = { text: cleanText(`PR #${p.number} green, waiting for the owner: ${p.title}`), url: cleanUrl(p.url) };
    } else if (pend.length) {
      const p = pend[0];
      a.last_result = { text: cleanText(`PR #${p.number} ${p.isDraft ? 'draft' : 'open, CI running'}: ${p.title}`), url: cleanUrl(p.url) };
    } else if (lastMerged) {
      a.last_result = { text: cleanText(`PR #${lastMerged.number} merged: ${lastMerged.title}`), url: cleanUrl(lastMerged.url) };
    }
    if (pr && pr.updatedAt) times.push(pr.updatedAt);
    if (!pr && lastMerged) times.push(lastMerged.mergedAt);
    for (const p of green) ownerQueue.push({ text: cleanText(`Review/merge PR #${p.number}: ${p.title}`), from: a.id, url: cleanUrl(p.url) });

    const t = times.filter((x) => !isNaN(Date.parse(x))).sort().pop();
    if (t) a.updated_at = new Date(t).toISOString().replace(/\.\d{3}Z$/, 'Z');
    if (!STATUSES.includes(a.status)) a.status = 'idle';
    return a;
  });
  for (const text of notes.owner) ownerQueue.push({ text, from: 'atlas', url: null });

  const out = {
    version: 1,
    updated_at: now.toISOString().replace(/\.\d{3}Z$/, 'Z'),
    updated_by: 'github',
    source: 'github',
    notes_issue: issue ? cleanUrl(issue.html_url) : null,
    agents,
    owner_queue: ownerQueue,
  };
  if (mainCi) out.main_ci = { state: mainCi.state, url: cleanUrl(mainCi.url) || (repoUrl ? cleanUrl(repoUrl + '/actions') : null) };
  return out;
}

// Same content, ignoring when it was built: used to skip redundant deploys.
export function fingerprint(status) {
  const copy = JSON.parse(JSON.stringify(status));
  delete copy.updated_at;
  return createHash('sha256').update(JSON.stringify(copy)).digest('hex');
}

// ---------- fetching (gh CLI, read-only calls only) ----------
function gh(args) {
  return execFileSync('gh', args, { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, stdio: ['ignore', 'pipe', 'inherit'] });
}
const ghJson = (args) => JSON.parse(gh(args));

function fetchAll(repo, now) {
  const fields = 'number,title,body,url,headRefName,labels,isDraft,updatedAt,author,isCrossRepository,headRepository,headRepositoryOwner';
  const open = ghJson(['pr', 'list', '--repo', repo, '--state', 'open', '--limit', '100', '--json', fields + ',statusCheckRollup']);
  const since = new Date(now.getTime() - MERGED_DAYS * 864e5).toISOString().slice(0, 10);
  const merged = ghJson(['pr', 'list', '--repo', repo, '--state', 'merged', '--limit', '100', '--search', `merged:>=${since}`, '--json', fields + ',mergedAt']);

  const issues = ghJson(['api', `repos/${repo}/issues?state=open&per_page=100`]);
  const trusted = new Set(['OWNER', 'MEMBER', 'COLLABORATOR']);
  const issue = issues
    .filter((i) => !i.pull_request && i.title.trim() === ISSUE_TITLE && trusted.has(i.author_association))
    .sort((x, y) => x.number - y.number)[0] || null;

  // CI on main: latest run of every workflow for main's head commit, except this deploy workflow.
  const sha = gh(['api', `repos/${repo}/commits/main`, '--jq', '.sha']).trim();
  const runs = ghJson(['api', `repos/${repo}/actions/runs?head_sha=${sha}&per_page=100`]).workflow_runs || [];
  const latest = {};
  for (const r of runs) {
    if (r.name === 'site-deploy' || r.head_branch !== 'main' || (r.head_repository && r.head_repository.full_name !== repo)) continue;
    if (!latest[r.workflow_id] || r.run_number > latest[r.workflow_id].run_number) latest[r.workflow_id] = r;
  }
  let mainCi = { state: 'none', url: null };
  const list = Object.values(latest);
  if (list.length) {
    const bad = list.find((r) => r.status === 'completed' && RED.has(String(r.conclusion).toUpperCase()));
    const running = list.some((r) => r.status !== 'completed');
    mainCi = bad ? { state: 'failure', url: bad.html_url } : running ? { state: 'pending', url: null } : { state: 'success', url: null };
  }
  return { open, merged, issue, mainCi };
}

function main() {
  const argv = process.argv.slice(2);
  const opt = (k) => { const i = argv.indexOf(k); return i >= 0 ? argv[i + 1] : null; };
  const here = dirname(fileURLToPath(import.meta.url));
  const statusPath = join(here, '..', 'site', 'office', 'status.json');
  const out = opt('--out') || statusPath;
  const repo = opt('--repo') || process.env.GITHUB_REPOSITORY || gh(['repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner']).trim();
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error('bad repo name: ' + repo);

  const roster = JSON.parse(readFileSync(statusPath, 'utf8'));
  const now = new Date();
  let { open, merged, issue, mainCi } = fetchAll(repo, now);
  const ids = new Set(roster.agents.map((a) => a.id));
  const notesFile = opt('--notes');
  if (notesFile) issue = { body: readFileSync(notesFile, 'utf8'), updated_at: now.toISOString(), html_url: null };
  const notes = issue ? parseNotes(issue.body, ids) : { bots: {}, owner: [] };
  if (!issue) console.error(`warning: no open issue titled "${ISSUE_TITLE}" by the owner or a collaborator; using the committed statuses`);

  const status = merge({ roster, notes, issue, prs: { open, merged }, mainCi, repo, owner: opt('--owner') || OWNER_LOGIN, now });
  const text = JSON.stringify(status, null, 2) + '\n';
  if (argv.includes('--print')) process.stdout.write(text);
  else { writeFileSync(out, text); console.error(`wrote ${out}: ${status.agents.length} agents, ${status.owner_queue.length} owner items, ${open.length} open PRs, ${merged.length} merged in ${MERGED_DAYS} days, main CI ${status.main_ci.state}`); }
  if (argv.includes('--fingerprint')) console.log('fingerprint=' + fingerprint(status));
}

if (import.meta.url === pathToFileURL(process.argv[1] || '').href) main();
