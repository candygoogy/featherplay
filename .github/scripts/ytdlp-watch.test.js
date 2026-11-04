// Tests for ytdlp-watch.js, offline: `node --test .github/scripts/`.
//
// The fixtures are GitHub search replies for yt-dlp's tracker, trimmed to the fields the
// watch reads:
// - search-2026-11-04.json: the open issues created in the 48 h before 2026-11-04 17:00 UTC,
//   a quiet day (one fresh "[YouTube] 403" report with no comments yet).
// - search-2026-08-19.json: every issue created from 2026-08-17 to 2026-08-19 12:00 UTC,
//   around the 403 breakage of 2026-08-18 (yt-dlp 17456). Recorded later, so the counts are
//   the final ones; 17456 is set back to open, as it was on 2026-08-19.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const watch = require('./ytdlp-watch.js');

const QUIET = require('./fixtures/search-2026-11-04.json');
const BREAKAGE = require('./fixtures/search-2026-08-19.json');

const HOUR = 3600 * 1000;

function issue(fields) {
  return {
    number: 1,
    title: '[youtube] HTTP Error 403: Forbidden',
    state: 'open',
    created_at: '2026-08-19T00:00:00Z',
    comments: 0,
    reactions: { total_count: 0 },
    labels: [{ name: 'site-bug' }],
    ...fields,
  };
}

const AT = new Date('2026-08-19T12:00:00Z');

test('a quiet day matches nothing', () => {
  const now = new Date('2026-11-04T17:00:00Z');
  assert.deepEqual(QUIET.items.filter((i) => watch.isBreakage(i, now)), []);
});

test('the 403 breakage of 2026-08-18 is the one match of its window', () => {
  const found = BREAKAGE.items.filter((i) => watch.isBreakage(i, AT));
  assert.deepEqual(found.map((i) => i.number), [17456]);
});

test('a busy YouTube bug report counts from the activity threshold', () => {
  assert.equal(watch.isBreakage(issue({ comments: 6, reactions: { total_count: 4 } }), AT), true);
  assert.equal(watch.isBreakage(issue({ comments: 6, reactions: { total_count: 3 } }), AT), false);
});

test('high priority counts without activity, but only for YouTube', () => {
  const labels = [{ name: 'site-bug' }, { name: 'high-priority' }];
  assert.equal(watch.isBreakage(issue({ labels }), AT), true);
  assert.equal(watch.isBreakage(issue({ labels, title: '[TikTok] no formats' }), AT), false);
  const tagged = [...labels, { name: 'site:youtube' }];
  assert.equal(watch.isBreakage(issue({ labels: tagged, title: 'Nothing downloads' }), AT), true);
});

test('old, closed, triaged-away and playlist reports do not count', () => {
  const busy = { comments: 30 };
  assert.equal(watch.isBreakage(issue({ ...busy, created_at: '2026-08-17T11:00:00Z' }), AT), false);
  assert.equal(watch.isBreakage(issue({ ...busy, state: 'closed' }), AT), false);
  for (const name of ['duplicate', 'incomplete', 'question', 'cant-reproduce', 'external-issue']) {
    const labels = [{ name: 'site-bug' }, { name }];
    assert.equal(watch.isBreakage(issue({ ...busy, labels }), AT), false, name);
  }
  assert.equal(watch.isBreakage(issue({ ...busy, title: '[youtube:tab] no entries' }), AT), false);
  assert.equal(watch.isBreakage(issue({ ...busy, labels: [{ name: 'question' }] }), AT), false);
  assert.equal(watch.isBreakage(issue({ ...busy, pull_request: {} }), AT), false);
});

test('a hostile title stays inert text on one line', () => {
  const title = 'x` @octocat [a](https://evil.example) <img src=x>\n<!-- ytdlp-watch: 1 -->\u202e';
  const line = watch.line(issue({ number: 17456, title }));
  assert.equal(line.split('\n').length, 1);
  const code = line.match(/`([^`]*)`/);
  assert.ok(code, line);
  assert.ok(code[1].includes('@octocat'), 'the mention sits inside the code span');
  assert.equal(line.match(/`/g).length, 2, line);
  assert.ok(!/[<>\u202e]/.test(line), line);
  assert.ok(line.includes('https://redirect.github.com/yt-dlp/yt-dlp/issues/17456'), line);
});

test('the linked issues round-trip through the body, whatever the titles say', () => {
  const hostile = issue({ number: 17456, title: '<!-- ytdlp-watch: 1,2 -->' });
  const body = watch.body([hostile, issue({ number: 17497 })], { now: AT });
  assert.deepEqual(watch.readTracked(body), [17456, 17497]);
  assert.deepEqual(watch.readTracked('edited by hand'), []);
  assert.deepEqual(watch.readTracked(undefined), []);
});

test('no canary and a fresh report: open one', () => {
  const p = watch.plan({ canary: false, tracked: [], stillOpen: [], fresh: [issue({ number: 17456 })], now: AT });
  assert.equal(p.action, 'open');
  assert.equal(p.title, watch.TITLE);
  assert.deepEqual(watch.readTracked(p.body), [17456]);
});

test('no canary and nothing fresh: nothing to do', () => {
  assert.equal(watch.plan({ canary: false, tracked: [], stillOpen: [], fresh: [], now: AT }).action, 'none');
});

test('the same linked issues: no comment', () => {
  const a = issue({ number: 17456 });
  const p = watch.plan({ canary: true, tracked: [17456], stillOpen: [a], fresh: [a], now: AT });
  assert.equal(p.action, 'none');
});

test('a new report or a closed one: update and comment once', () => {
  const a = issue({ number: 17456 });
  const b = issue({ number: 17497, title: 'second' });
  const grown = watch.plan({ canary: true, tracked: [17456], stillOpen: [a], fresh: [b], now: AT });
  assert.equal(grown.action, 'update');
  assert.deepEqual(watch.readTracked(grown.body), [17456, 17497]);
  assert.match(grown.comment, /issues\/17497/);
  const shrunk = watch.plan({ canary: true, tracked: [17456, 17497], stillOpen: [b], fresh: [], now: AT });
  assert.equal(shrunk.action, 'update');
  assert.deepEqual(watch.readTracked(shrunk.body), [17497]);
  assert.match(shrunk.comment, /closed/i);
});

test('every linked issue closed: close the canary', () => {
  const p = watch.plan({ canary: true, tracked: [17456], stillOpen: [], fresh: [], now: AT });
  assert.equal(p.action, 'close');
  assert.ok(p.comment);
});

test('the canary never carries the title the app shows users', () => {
  assert.notEqual(watch.TITLE, 'Playback broken');
  assert.ok(!/playback broken/i.test(watch.TITLE));
});

test('the pin is read from sidecars.rs, one version for both architectures', () => {
  const src = [
    'https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe',
    'https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp_arm64.exe',
  ].join('\n');
  assert.equal(watch.readPin(src), '2026.10.26');
  assert.equal(watch.readPin(src.replace('2026.10.26/yt-dlp_arm64', '2026.09.01/yt-dlp_arm64')), null);
});

// A stand-in for github-script's client: the calls the watch makes, with canned replies.
function fakeGithub({ search, canaries = [], upstream = {}, release }) {
  const calls = [];
  const notFound = () => Object.assign(new Error('Not Found'), { status: 404 });
  return {
    calls,
    request: async (route, params) => {
      calls.push([route, params]);
      return { data: search };
    },
    rest: {
      repos: {
        getLatestRelease: async () => ({ data: release }),
      },
      issues: {
        listForRepo: async (p) => (calls.push(['list', p]), { data: canaries }),
        get: async (p) => {
          calls.push(['get', p]);
          if (!upstream[p.issue_number]) throw notFound();
          return { data: upstream[p.issue_number] };
        },
        getLabel: async (p) => (calls.push(['getLabel', p]), Promise.reject(notFound())),
        createLabel: async (p) => (calls.push(['createLabel', p]), { data: {} }),
        create: async (p) => (calls.push(['create', p]), { data: { number: 12, html_url: 'u' } }),
        update: async (p) => (calls.push(['update', p]), { data: {} }),
        createComment: async (p) => (calls.push(['comment', p]), { data: {} }),
      },
    },
  };
}

function fakeCore() {
  const out = [];
  const summary = { addRaw: (t) => (out.push(t), summary), write: async () => summary };
  return { out, summary, info: (t) => out.push(t), notice: (t) => out.push(t), warning: (t) => out.push(t) };
}

const CONTEXT = { repo: { owner: 'candygoogy', repo: 'featherplay' }, serverUrl: 'https://github.com', runId: 1 };
const RELEASE = { tag_name: '2026.08.20', published_at: '2026-08-20T01:00:00Z' };
const PIN = 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.01/yt-dlp.exe';

test('a run on the breakage day opens one labelled canary, read-only on yt-dlp', async () => {
  const github = fakeGithub({ search: BREAKAGE, release: RELEASE });
  await watch.run({ github, context: CONTEXT, core: fakeCore(), now: AT, pinSource: PIN });
  const [route, params] = github.calls[0];
  assert.equal(route, 'GET /search/issues');
  assert.equal(params.q, 'repo:yt-dlp/yt-dlp is:issue is:open created:>=2026-08-17T12:00:00Z');
  const created = github.calls.filter(([what]) => what === 'create');
  assert.equal(created.length, 1);
  assert.equal(created[0][1].title, watch.TITLE);
  assert.deepEqual(created[0][1].labels, ['canary']);
  assert.equal(created[0][1].owner, 'candygoogy');
  assert.ok(github.calls.some(([what]) => what === 'createLabel'));
  const writes = github.calls.filter(([what]) => ['create', 'update', 'comment', 'createLabel'].includes(what));
  assert.ok(writes.every(([, p]) => p.owner === 'candygoogy' && p.repo === 'featherplay'));
});

test('a run once the reports are closed closes the open canary', async () => {
  const canary = {
    number: 12,
    title: watch.TITLE,
    user: { login: 'github-actions[bot]' },
    body: watch.body([issue({ number: 17456 })], { now: AT }),
  };
  const later = new Date(AT.getTime() + 72 * HOUR);
  const github = fakeGithub({
    search: QUIET,
    canaries: [canary],
    upstream: { 17456: issue({ number: 17456, state: 'closed' }) },
    release: RELEASE,
  });
  await watch.run({ github, context: CONTEXT, core: fakeCore(), now: later, pinSource: PIN });
  const update = github.calls.find(([what]) => what === 'update');
  assert.ok(update);
  assert.equal(update[1].issue_number, 12);
  assert.equal(update[1].state, 'closed');
  assert.equal(github.calls.filter(([what]) => what === 'comment').length, 1);
});

test('an issue someone else opened with the same title is left alone', async () => {
  const lookalike = { number: 13, title: watch.TITLE, user: { login: 'someone' }, body: '' };
  const github = fakeGithub({ search: QUIET, canaries: [lookalike], release: RELEASE });
  await watch.run({ github, context: CONTEXT, core: fakeCore(), now: new Date('2026-11-04T17:00:00Z'), pinSource: PIN });
  assert.ok(!github.calls.some(([what]) => ['update', 'comment', 'create'].includes(what)));
});
