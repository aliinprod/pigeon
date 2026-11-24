// node --test office/build-status.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseNotes, botForPr, rollup, merge, cleanText, cleanUrl, fingerprint, trustedPr } from './build-status.mjs';

const ids = new Set(['atlas', 'anvil', 'canvas', 'plume', 'warden', 'proof', 'frame']);
const roster = {
  agents: [...ids].map((id) => ({ id, name: id[0].toUpperCase() + id.slice(1), role: 'r', status: 'idle', task: null, last_result: null, updated_at: '2026-11-23T00:00:00Z' })),
  owner_queue: [],
};
const now = new Date('2026-11-24T12:00:00Z');
const REPO = 'o/r';
const OWNER = 'o';
const R = 'https://github.com/' + REPO;
// A PR by the owner from a branch of this repo (the only kind that counts).
const own = (pr) => ({ author: { login: OWNER }, isCrossRepository: false, headRepositoryOwner: { login: 'o' }, headRepository: { name: 'r' }, ...pr });

test('parseNotes: strict lines, known ids only, first line wins', () => {
  const body = [
    '# Office status', 'Edit one line per bot.',
    'anvil: working | One key per computer (both roles)',
    '- warden: idle | Will review one-key migration',
    '`frame: done | Promo video v1`',
    'proof: idle',
    'anvil: done | duplicate is ignored',
    'mallory: working | unknown id',
    'canvas: busy | bad status',
    'owner: Test pairing build on the office Mac',
  ].join('\r\n');
  const n = parseNotes(body, ids);
  assert.deepEqual(n.bots.anvil, { status: 'working', task: 'One key per computer (both roles)' });
  assert.equal(n.bots.warden.status, 'idle');
  assert.equal(n.bots.frame.task, 'Promo video v1');
  assert.equal(n.bots.proof.task, null);
  assert.equal(n.bots.mallory, undefined);
  assert.equal(n.bots.canvas, undefined);
  assert.deepEqual(n.owner, ['Test pairing build on the office Mac']);
});

test('text stays text: markup is kept as characters, controls stripped, length capped', () => {
  const n = parseNotes('plume: working | <img src=x onerror=alert(1)>‮\u0007 hi', ids);
  assert.equal(n.bots.plume.task, '<img src=x onerror=alert(1)> hi'); // rendered with textContent
  assert.equal(cleanText('x'.repeat(500)).length, 160);
  assert.equal(cleanUrl('javascript:alert(1)'), null);
  assert.equal(cleanUrl('https://evil.example/pr/1'), null);
  assert.equal(cleanUrl('https://github.com/o/r/pull/1'), 'https://github.com/o/r/pull/1');
});

test('botForPr: label, then id prefix, then known prefix, then title', () => {
  assert.equal(botForPr({ labels: [{ name: 'bot:warden' }], headRefName: 'site-x' }, ids), 'warden');
  assert.equal(botForPr({ labels: [{ name: 'bot:nobody' }], headRefName: 'site-x' }, ids), 'plume');
  assert.equal(botForPr({ headRefName: 'anvil/one-key' }, ids), 'anvil');
  assert.equal(botForPr({ headRefName: 'agent-office' }, ids), 'plume');
  assert.equal(botForPr({ headRefName: 'security-foundation' }, ids), 'warden');
  assert.equal(botForPr({ headRefName: 'misc', title: 'Promo video v1 (Frame)' }, ids), 'frame');
  assert.equal(botForPr({ headRefName: 'misc', title: 'x', body: 'Bot: proof' }, ids), 'proof');
  assert.equal(botForPr({ headRefName: 'misc', title: 'x' }, ids), null);
});

test('rollup: any red is failure with its link; running is pending', () => {
  const ok = { __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'SUCCESS' };
  assert.deepEqual(rollup([ok, { __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'FAILURE', detailsUrl: R + '/actions/runs/1' }]), { state: 'failure', url: R + '/actions/runs/1' });
  assert.equal(rollup([ok, { __typename: 'CheckRun', status: 'IN_PROGRESS' }]).state, 'pending');
  assert.equal(rollup([ok, { __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'SKIPPED' }]).state, 'success');
  assert.equal(rollup([{ __typename: 'StatusContext', state: 'ERROR', targetUrl: R }]).state, 'failure');
  assert.equal(rollup([]).state, 'none');
});

test('merge: notes give task, red CI forces waiting, green PR goes to the owner queue', () => {
  const notes = parseNotes('anvil: working | One key\ncanvas: working | Design v2\nplume: working | Office\nowner: Test on the Mac', ids);
  const red = [{ __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'FAILURE', detailsUrl: R + '/actions/runs/9' }];
  const green = [{ __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'SUCCESS' }];
  const prs = {
    open: [
      own({ number: 12, title: 'One key', url: R + '/pull/12', headRefName: 'anvil/one-key', updatedAt: '2026-11-24T10:00:00Z', statusCheckRollup: red }),
      own({ number: 13, title: 'Design v2', url: R + '/pull/13', headRefName: 'app-v2', updatedAt: '2026-11-24T11:00:00Z', statusCheckRollup: green }),
    ],
    merged: [
      own({ number: 10, title: 'Promo video v1 (Frame)', url: R + '/pull/10', headRefName: 'promo-video', mergedAt: '2026-11-23T11:42:42Z' }),
    ],
  };
  const s = merge({ roster, notes, issue: { updated_at: '2026-11-24T09:00:00Z', html_url: R + '/issues/1' }, prs, mainCi: { state: 'failure', url: R + '/actions/runs/5' }, repo: REPO, owner: OWNER, now });
  const get = (id) => s.agents.find((a) => a.id === id);
  assert.equal(get('anvil').status, 'waiting');
  assert.equal(get('anvil').task, 'One key');
  assert.equal(get('anvil').last_result.url, R + '/actions/runs/9');
  assert.equal(get('canvas').status, 'working'); // notes win unless CI is red
  assert.match(get('canvas').last_result.text, /green, waiting for the owner/);
  assert.equal(get('frame').status, 'done'); // no note: merged recently
  assert.equal(get('frame').last_result.url, R + '/pull/10');
  assert.equal(get('warden').status, 'idle'); // nothing anywhere: roster default
  assert.deepEqual(s.owner_queue.map((q) => q.text), ['Review/merge PR #13: Design v2', 'Test on the Mac']);
  assert.equal(s.main_ci.state, 'failure');
  assert.equal(s.source, 'github');
  const later = merge({ roster, notes, issue: { updated_at: '2026-11-24T09:00:00Z', html_url: R + '/issues/1' }, prs, mainCi: { state: 'failure', url: R + '/actions/runs/5' }, repo: REPO, owner: OWNER, now: new Date('2026-11-24T13:00:00Z') });
  assert.equal(fingerprint(s), fingerprint(later)); // build time does not trigger a redeploy
});

test('W-13: fork PRs and PRs by anyone but the owner are ignored everywhere', () => {
  const green = [{ __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'SUCCESS' }];
  const fork = { number: 20, title: 'Site tweak (Plume)', url: R + '/pull/20', headRefName: 'site-evil', updatedAt: '2026-11-24T10:00:00Z', statusCheckRollup: green,
    author: { login: OWNER }, isCrossRepository: true, headRepositoryOwner: { login: 'stranger' }, headRepository: { name: 'r' } };
  const stranger = { number: 21, title: 'Bump (Anvil)', url: R + '/pull/21', headRefName: 'anvil/x', updatedAt: '2026-11-24T10:00:00Z', statusCheckRollup: green,
    author: { login: 'stranger' }, isCrossRepository: false, headRepositoryOwner: { login: 'o' }, headRepository: { name: 'r' } };
  const lookalike = { ...fork, number: 22, isCrossRepository: false, headRepositoryOwner: { login: 'o-fake' } };
  const merged = { ...stranger, number: 23, title: 'Merged by a stranger (Frame)', headRefName: 'promo-x', mergedAt: '2026-11-24T09:00:00Z' };
  assert.equal(trustedPr(own({}), REPO, OWNER), true);
  for (const p of [fork, stranger, lookalike, merged, {}, null]) assert.equal(trustedPr(p, REPO, OWNER), false);
  assert.equal(trustedPr({ ...own({}), author: undefined }, REPO, OWNER), false);
  const s = merge({ roster, notes: parseNotes('', ids), issue: null, prs: { open: [fork, stranger, lookalike], merged: [merged] }, mainCi: null, repo: REPO, owner: OWNER, now });
  assert.deepEqual(s.owner_queue, []);
  for (const a of s.agents) {
    assert.equal(a.status, 'idle', a.id);
    assert.equal(a.last_result, null, a.id);
  }
  assert.ok(!JSON.stringify(s).includes('/pull/2'));
});
