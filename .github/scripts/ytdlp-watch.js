// The daily yt-dlp watch (.github/workflows/ytdlp-watch.yml), run by actions/github-script.
//
// featherplay plays through yt-dlp, and when YouTube changes something that breaks yt-dlp,
// busy reports show up on yt-dlp's tracker within hours. GitHub's runners can't check
// playback themselves (YouTube asks them to prove they aren't bots), so this reads those
// reports instead, through the API and read-only on yt-dlp/yt-dlp:
// - one search for the open issues created in the last WINDOW_HOURS, kept when isBreakage()
//   says so;
// - yt-dlp's latest release, set against the pin in crates/streams/src/sidecars.rs.
//
// It keeps one issue in this repository, TITLE with the `canary` label: opened when a
// report matches, updated with one comment when the set of linked reports changes, closed
// when they are all closed. That issue is for the maintainer only. The app never reads it:
// the app's notice follows an issue titled "Playback broken" that the maintainer opens by
// hand after checking playback (RELEASING.md, "When playback breaks").
//
// yt-dlp's issue titles are written by anyone, so they are only ever data: matched with
// fixed patterns and written as inline code, stripped of backticks, angle brackets, control
// and direction characters, never passed to a shell. Links go through redirect.github.com,
// which keeps the reports free of "mentioned this issue" lines from this repository.

'use strict';

/** The canary's title. Never the app's ISSUE_TITLE ("Playback broken"). */
const TITLE = 'yt-dlp reports YouTube breakage';
const LABEL = 'canary';
/** The canary is the one this workflow opened, not a lookalike someone else opened. */
const BOT = 'github-actions[bot]';
const UPSTREAM = { owner: 'yt-dlp', repo: 'yt-dlp' };
/** How far back a report may have been created: two daily runs see each one. */
const WINDOW_HOURS = 48;
/** Comments plus reactions that make a fresh report a breakage rather than one user's trouble. */
const MIN_ACTIVITY = 10;
/** Labels yt-dlp's maintainers give reports that aren't (or aren't yet) a YouTube breakage. */
const NOT_BREAKAGE = new Set([
  'duplicate',
  'invalid',
  'incomplete',
  'question',
  'cant-reproduce',
  'external-issue',
  'wontfix',
  'spam',
  'site-request',
  'site-enhancement',
  'enhancement',
  'ai-policy-violation',
  'discussion/announcement',
]);
/** The canary's first line: the linked reports, by number. */
const MARKER = /^<!-- ytdlp-watch: ([0-9]+(?:,[0-9]+)*)? -->/;
const VERSION = /^[0-9]{4}\.[0-9]{2}\.[0-9]{2}(\.[0-9]+)?$/;

function labels(issue) {
  return (issue.labels || []).map((l) => (typeof l === 'string' ? l : l.name));
}

function validNumber(n) {
  return Number.isSafeInteger(n) && n > 0;
}

/** Whether a yt-dlp issue, as GitHub's search returns it, is a fresh YouTube breakage report. */
function isBreakage(issue, now) {
  if (issue.pull_request || issue.state !== 'open' || !validNumber(issue.number)) return false;
  const created = Date.parse(issue.created_at);
  if (!(created >= now.getTime() - WINDOW_HOURS * 3600 * 1000)) return false;
  const names = labels(issue);
  const title = String(issue.title || '');
  // New reports carry only the bug template's `site-bug, triage` until a maintainer adds
  // `site:youtube`, so the title counts too.
  const youtube = names.includes('site:youtube') || /\byoutube\b/i.test(title);
  // Playlist, channel and search pages: featherplay doesn't use yt-dlp for those.
  const pages = /\[youtube:[a-z_]+\]/i.test(title);
  const bug = names.includes('site-bug') || names.includes('high-priority');
  if (!youtube || pages || !bug || names.some((n) => NOT_BREAKAGE.has(n))) return false;
  const activity = (issue.comments || 0) + ((issue.reactions && issue.reactions.total_count) || 0);
  return names.includes('high-priority') || activity >= MIN_ACTIVITY;
}

/** An untrusted title as inline code on one line. */
function inert(title) {
  const text = String(title || '')
    .replace(/[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u2028-\u202e\u2066-\u2069\ufeff]/g, ' ')
    .replace(/[`<>]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  const short = text.length > 120 ? `${text.slice(0, 119)}\u2026` : text;
  return '`' + (short || 'no title') + '`';
}

function link(n) {
  return `[yt-dlp ${n}](https://redirect.github.com/yt-dlp/yt-dlp/issues/${n})`;
}

/** One report as a list item. */
function line(issue) {
  const facts = [];
  if (labels(issue).includes('high-priority')) facts.push('high priority');
  facts.push(`${issue.comments || 0} comments`);
  facts.push(`${(issue.reactions && issue.reactions.total_count) || 0} reactions`);
  return `- ${link(issue.number)}: ${inert(issue.title)} (${facts.join(', ')})`;
}

/** The reports a canary body links, from its marker line. */
function readTracked(body) {
  const m = MARKER.exec(String(body || ''));
  if (!m || !m[1]) return [];
  return [...new Set(m[1].split(',').map(Number))].filter(validNumber).sort((a, b) => a - b);
}

/** The yt-dlp version pinned in sidecars.rs, or null if the architectures disagree. */
function readPin(source) {
  const found = new Set();
  for (const m of String(source).matchAll(/yt-dlp\/releases\/download\/([0-9][0-9.]*)\//g)) found.add(m[1]);
  return found.size === 1 ? [...found][0] : null;
}

function releaseLine(release, pin) {
  if (!release) return '';
  const tag = VERSION.test(String(release.tag)) ? release.tag : 'unknown';
  const day = /^\d{4}-\d{2}-\d{2}/.exec(String(release.published || ''));
  const pinned = pin && VERSION.test(pin) ? pin : 'unknown';
  return `Latest yt-dlp: ${tag}${day ? `, published ${day[0]}` : ''}. featherplay pins ${pinned}.`;
}

/** The canary's body for these reports. */
function body(linked, { release, pin, runUrl } = {}) {
  const numbers = linked.map((i) => i.number);
  return [
    `<!-- ytdlp-watch: ${numbers.join(',')} -->`,
    "Fresh, busy reports of YouTube breakage on yt-dlp's tracker. Playback in featherplay " +
      'usually breaks with it until yt-dlp ships a fix and the app updates it.',
    '',
    ...linked.map(line),
    '',
    releaseLine(release, pin),
    '',
    "This issue is for the maintainer; the app never reads it. Check playback on a home connection. If it's broken there too, open an issue titled exactly `Playback broken` (RELEASING.md, \"When playback breaks\") so the app links to it.",
    '',
    'The daily yt-dlp watch updates this list, comments when it changes and closes this issue once every linked report is closed.' +
      (runUrl ? ` Last change: ${runUrl}` : ''),
  ].join('\n');
}

/**
 * What to do with the canary: `none`, `open` (with title and body), `update` (body and one
 * comment) or `close` (with a comment). `tracked` is the canary's list, `stillOpen` those of
 * them still open, `fresh` today's matches.
 */
function plan({ canary, tracked, stillOpen, fresh, release, pin, runUrl }) {
  const byNumber = new Map();
  for (const i of [...stillOpen, ...fresh]) if (validNumber(i.number)) byNumber.set(i.number, i);
  const linked = [...byNumber.values()].sort((a, b) => a.number - b.number);
  const now = linked.map((i) => i.number);
  const before = [...new Set(tracked)].sort((a, b) => a - b);
  const extra = { release, pin, runUrl };
  if (!canary) {
    return linked.length ? { action: 'open', title: TITLE, body: body(linked, extra) } : { action: 'none' };
  }
  if (!linked.length) {
    return { action: 'close', comment: 'Every linked yt-dlp report is closed, so this closes too.' };
  }
  if (before.length === now.length && before.every((n, k) => n === now[k])) return { action: 'none' };
  const added = linked.filter((i) => !before.includes(i.number));
  const closed = before.filter((n) => !now.includes(n));
  const comment = [
    ...(added.length ? ['New reports:', ...added.map(line)] : []),
    ...(closed.length ? [`Closed since: ${closed.map(link).join(', ')}.`] : []),
  ].join('\n');
  return { action: 'update', body: body(linked, extra), comment };
}

function iso(t) {
  return new Date(t).toISOString().replace(/\.\d{3}Z$/, 'Z');
}

/** The workflow's step: read yt-dlp's tracker and release, then act on the canary. */
async function run({ github, context, core, now = new Date(), pinSource }) {
  const here = { owner: context.repo.owner, repo: context.repo.repo };
  const runUrl = `${context.serverUrl}/${here.owner}/${here.repo}/actions/runs/${context.runId}`;

  const { data: latest } = await github.rest.repos.getLatestRelease(UPSTREAM);
  const release = { tag: latest.tag_name, published: latest.published_at };
  const pin = readPin(pinSource);
  const recent = Date.parse(latest.published_at) >= now.getTime() - WINDOW_HOURS * 3600 * 1000;
  const rel = releaseLine(release, pin);
  if (recent) core.notice(`New yt-dlp release. ${rel}`);
  else core.info(rel);

  const since = iso(now.getTime() - WINDOW_HOURS * 3600 * 1000);
  const q = `repo:yt-dlp/yt-dlp is:issue is:open created:>=${since}`;
  const { data: found } = await github.request('GET /search/issues', { q, per_page: 100 });
  const items = found.items || [];
  if (found.incomplete_results || found.total_count > items.length) {
    core.warning(`The search returned ${items.length} of ${found.total_count} issues; some were not checked.`);
  }
  const fresh = items.filter((i) => isBreakage(i, now));

  const { data: open } = await github.rest.issues.listForRepo({ ...here, labels: LABEL, state: 'open', per_page: 100 });
  const canary = open.find((i) => !i.pull_request && i.title === TITLE && i.user && i.user.login === BOT);
  const tracked = canary ? readTracked(canary.body) : [];
  const stillOpen = [];
  for (const n of tracked) {
    try {
      const { data } = await github.rest.issues.get({ ...UPSTREAM, issue_number: n });
      if (data.state === 'open') stillOpen.push(data);
    } catch (e) {
      // Deleted or moved away: no longer a report to wait for.
      if (e.status !== 404 && e.status !== 410) throw e;
    }
  }

  const p = plan({ canary: Boolean(canary), tracked, stillOpen, fresh, release, pin, runUrl });
  const summary = [
    `Checked ${items.length} yt-dlp issues created since ${since}: ${fresh.length} fresh YouTube breakage report(s).`,
    ...fresh.map(line),
    '',
    rel,
    '',
    `Canary: ${canary ? `#${canary.number}` : 'none open'}; action: ${p.action}.`,
  ];
  await core.summary.addRaw(summary.join('\n') + '\n').write();

  if (p.action === 'open') {
    try {
      await github.rest.issues.getLabel({ ...here, name: LABEL });
    } catch (e) {
      if (e.status !== 404) throw e;
      await github.rest.issues.createLabel({
        ...here,
        name: LABEL,
        color: 'b60205',
        description: 'Early warning for the maintainer; the app never reads it',
      });
    }
    const { data } = await github.rest.issues.create({ ...here, title: p.title, body: p.body, labels: [LABEL] });
    core.notice(`Opened ${data.html_url}`);
  } else if (p.action === 'update') {
    await github.rest.issues.update({ ...here, issue_number: canary.number, body: p.body });
    await github.rest.issues.createComment({ ...here, issue_number: canary.number, body: p.comment });
  } else if (p.action === 'close') {
    await github.rest.issues.createComment({ ...here, issue_number: canary.number, body: p.comment });
    await github.rest.issues.update({ ...here, issue_number: canary.number, state: 'closed', state_reason: 'completed' });
  }
}

module.exports = { TITLE, LABEL, WINDOW_HOURS, MIN_ACTIVITY, isBreakage, line, body, readTracked, readPin, plan, run };
