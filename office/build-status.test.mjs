// node --test office/build-status.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseNotes, botForPr, rollup, merge, cleanText, cleanUrl, fingerprint } from './build-status.mjs';

const ids = new Set(['atlas', 'anvil', 'canvas', 'plume', 'warden', 'proof', 'frame']);
const roster = {
  agents: [...ids].map((id) => ({ id, name: id[0].toUpperCase() + id.slice(1), role: 'r', status: 'idle', task: null, last_result: null, updated_at: '2026-11-23T00:00:00Z' })),
  owner_queue: [],
};
const now = new Date('2026-11-24T12:00:00Z');
const R = 'https://github.com/o/r';

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
      { number: 12, title: 'One key', url: R + '/pull/12', headRefName: 'anvil/one-key', updatedAt: '2026-11-24T10:00:00Z', statusCheckRollup: red },
      { number: 13, title: 'Design v2', url: R + '/pull/13', headRefName: 'app-v2', updatedAt: '2026-11-24T11:00:00Z', statusCheckRollup: green },
    ],
    merged: [
      { number: 10, title: 'Promo video v1 (Frame)', url: R + '/pull/10', headRefName: 'promo-video', mergedAt: '2026-11-23T11:42:42Z' },
    ],
  };
  const s = merge({ roster, notes, issue: { updated_at: '2026-11-24T09:00:00Z', html_url: R + '/issues/1' }, prs, mainCi: { state: 'failure', url: R + '/actions/runs/5' }, repoUrl: R, now });
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
  const later = merge({ roster, notes, issue: { updated_at: '2026-11-24T09:00:00Z', html_url: R + '/issues/1' }, prs, mainCi: { state: 'failure', url: R + '/actions/runs/5' }, repoUrl: R, now: new Date('2026-11-24T13:00:00Z') });
  assert.equal(fingerprint(s), fingerprint(later)); // build time does not trigger a redeploy
});
