// The notes loop in a real browser, against the real program.
//
// Usage: node notes.mjs <driftmark binary> [<out-dir>]
//
// It makes a small git repo with a made-up document, starts
// `driftmark serve` on 127.0.0.1 with a free port, and in headless Chromium:
//
//   0. without the token, selects a sentence, fills in a note and saves:
//      the form says how to get the token instead of failing; after the
//      printed link is opened in another tab, the same save goes through,
//      and the note's history says it came from the page;
//   1. finds the new note in place;
//   2. reloads, and finds the note in place: its card under the sentence's
//      block, and a highlight that covers exactly the selected words;
//   3. edits the document on disk (a new paragraph above), reloads, and
//      finds the note still exact, highlighted at its new place;
//   4. rewords the sentence slightly, reloads, and finds it fuzzy, with the
//      new words shown; then deletes it, and finds it in "Unanchored";
//   5. on the change page of a branch: selects words in a card, notes them,
//      marks the card seen, reloads, and finds both kept;
//   6. an agent in a linked worktree answers that note through
//      `driftmark mcp`, recorded as the connector's; the reader decides it
//      on the page; the agent commits the change and records the commit;
//      the card then says "Changed because of note n2", its link leads to
//      the note, and the note lists the change;
//   7. a page whose marker the server no longer knows still saves as the
//      page: it gets a fresh marker and sends again;
//   8. a reply sent with the token from a script, not the page, shows on
//      the note's card as a warning that has no button to dismiss it.
//
// axe-core checks the "get the token" message and a card with a warning,
// in light and dark, at desktop and phone width: 0 problems each.
//
// Screenshots of each step go to <out-dir> when given. CHROMIUM names the
// browser binary; the default is the cached headless shell that Playwright
// 1.64.1 runs with (see the anchoring spike).
import { chromium } from 'playwright';
import { spawn, execFileSync } from 'node:child_process';
import readline from 'node:readline';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';

const [program, outDir] = process.argv.slice(2);
if (!program) {
  console.error('usage: node notes.mjs <driftmark binary> [<out-dir>]');
  process.exit(2);
}
const exe = process.env.CHROMIUM
  || path.join(os.homedir(), '.cache/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-linux64/chrome-headless-shell');

const DOC = `# Release steps

> Short version: how a release goes out, step by step.

## Before the release

Freeze the branch two days before the release, and tell every team in the channel.

Run the full test suite on the frozen branch.

## After the release

Watch the error dashboards for one hour.
`;
const SENTENCE = 'Freeze the branch two days before the release';

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'driftmark-notes-browser-'));
const repo = path.join(work, 'repo');
const gitIn = (dir, ...args) => execFileSync('git', ['-C', dir, ...args], {
  env: {
    ...process.env,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: path.join(work, 'gitconfig'),
    GIT_AUTHOR_NAME: 'Test', GIT_AUTHOR_EMAIL: 'test@example.com',
    GIT_COMMITTER_NAME: 'Test', GIT_COMMITTER_EMAIL: 'test@example.com',
  },
}).toString().trim();
const git = (...args) => gitIn(repo, ...args);
fs.writeFileSync(path.join(work, 'gitconfig'), '');
fs.mkdirSync(path.join(repo, 'docs'), { recursive: true });
execFileSync('git', ['init', '-q', '-b', 'main', repo]);
const file = path.join(repo, 'docs/release.md');
fs.writeFileSync(file, DOC);
git('add', '-A');
git('commit', '-q', '-m', 'first');
git('update-ref', 'refs/remotes/origin/main', 'HEAD');
git('checkout', '-q', '-b', 'feature');
fs.writeFileSync(file, DOC.replace('Run the full test suite', 'Run the full test suite twice'));
git('commit', '-q', '-am', 'feature');
git('checkout', '-q', 'main');

const environment = { ...process.env };
delete environment.DRIFTMARK_TOKEN;
const server = spawn(program, ['serve', '--repo', repo, '--addr', '127.0.0.1:0'], { stdio: ['ignore', 'pipe', 'inherit'], env: environment });
const [base, token] = await new Promise((resolve, reject) => {
  let seen = '';
  server.stdout.on('data', (chunk) => {
    seen += chunk;
    const found = /open (http:\/\/127\.0\.0\.1:\d+)\/\?token=([0-9a-f]+)/.exec(seen);
    if (found) resolve([found[1], found[2]]);
  });
  server.on('exit', () => reject(new Error(`the server stopped: ${seen}`)));
});
const cookie = `driftmark-${new URL(base).port}=${token}`;

// axe-core, read from the test's own packages, for WCAG 2.1 A and AA.
const axeSource = fs.readFileSync(createRequire(import.meta.url).resolve('axe-core/axe.min.js'), 'utf8');

const results = [];
const check = (name, ok, detail = '') => {
  results.push({ name, ok, detail });
  console.log(`${ok ? 'pass' : 'FAIL'}  ${name}${detail ? `: ${detail}` : ''}`);
};

const browser = await chromium.launch({ executablePath: exe });
const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
// The browser logs the refusals this test asks for (401 without the token,
// 412 for an old marker) as failed loads; those are expected.
const expected = /^Failed to load resource: the server responded with a status of (401|412)\b/;
page.on('console', (message) => { if (message.type() === 'error' && !expected.test(message.text())) errors.push(message.text()); });
const shot = async (name) => {
  if (outDir) {
    fs.mkdirSync(outDir, { recursive: true });
    await page.screenshot({ path: path.join(outDir, `${name}.png`), fullPage: true });
  }
};

// Selects `text` inside `scope` with the Range API, as a reader's drag
// would, and waits for the floating button.
const select = async (scope, text) => {
  await page.evaluate(([scope, text]) => {
    const root = document.querySelector(scope);
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const at = node.data.indexOf(text);
      if (at >= 0) {
        const range = document.createRange();
        range.setStart(node, at);
        range.setEnd(node, at + text.length);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        return;
      }
    }
    throw new Error(`"${text}" is not on the page`);
  }, [scope, text]);
  await page.locator('.dm-float').waitFor({ state: 'visible' });
};

const saveNote = async (body, stance, { closes = true } = {}) => {
  await page.locator('.dm-float').click();
  await page.locator('dialog.dm-dialog').waitFor({ state: 'visible' });
  await page.locator(`dialog input[name=stance][value=${stance}]`).check();
  await page.locator('dialog textarea').fill(body);
  await page.locator('dialog button[type=submit]').click();
  if (closes) await page.locator('dialog.dm-dialog').waitFor({ state: 'hidden' });
};

// The note cards and the highlighted text on the page.
const shown = () => page.evaluate(() => {
  const cards = [...document.querySelectorAll('.dm-note')].map((card) => ({
    id: card.id,
    state: card.dataset.state,
    after: card.previousElementSibling ? card.previousElementSibling.textContent.trim().slice(0, 60) : '',
    now: card.querySelector('.dm-now') ? card.querySelector('.dm-now').textContent : null,
  }));
  const highlighted = [];
  for (const name of ['dm-notes', 'dm-notes-changed']) {
    const set = window.CSS && CSS.highlights && CSS.highlights.get(name);
    if (set) for (const range of set) highlighted.push(range.toString());
  }
  const orphans = [...document.querySelectorAll('.dm-orphans .dm-note')].map((card) => card.id);
  return { cards, highlighted, orphans };
});

// Runs axe on the page as it is now, in light and dark, at desktop and
// phone width, and checks each finds nothing.
const accessible = async (what) => {
  for (const theme of ['light', 'dark']) {
    for (const [width, height] of [[1280, 900], [390, 844]]) {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize({ width, height });
      const found = await page.evaluate(async (source) => {
        if (!window.axe) (0, eval)(source);
        const result = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'] } });
        return {
          problems: result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`),
          passed: result.passes.length,
        };
      }, axeSource);
      // `passed` proves axe ran its rules on the page.
      check(`axe finds no problem: ${what}, ${theme}, ${width} px`, found.problems.length === 0 && found.passed > 10,
        `${found.passed} rules passed; ${found.problems.join(' | ')}`);
      await shot(`axe-${what.replace(/\W+/g, '-')}-${theme}-${width}`);
    }
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await page.setViewportSize({ width: 1280, height: 900 });
};

// A note as the server holds it, read with the page's own cookie.
const noteJson = (id) => page.evaluate(async (id) => (await (await fetch(`/api/notes/${id}`)).json()).note, id);
const sources = (note) => note.history.map((event) => `${event.action} ${event.role} ${event.source}`);

const reading = `${base}/doc/worktree/-/docs/release.md`;
try {
  // 0. Without the token, the form says how to get it; with it, the same
  //    save goes through and counts as the page.
  await page.goto(reading);
  await page.waitForFunction(() => window.driftmarkNotes);
  await select('.doc-body', SENTENCE);
  await saveNote('Two days is too short for the mobile team.', 'disagree', { closes: false });
  await page.locator('dialog .dm-token').waitFor();
  const told = await page.locator('dialog .dm-token').textContent();
  check('without the token the form says how to get it', told.includes('?token=') && told.includes('does not have the reader\'s token'), told);
  check('the note is kept in the form', (await page.locator('dialog textarea').inputValue()).startsWith('Two days'));
  await shot('0-token-needed');
  await accessible('the get-the-token message');
  const other = await context.newPage();
  await other.goto(`${base}/?token=${token}`);
  await other.close();
  await page.locator('dialog button[type=submit]').click();
  await page.locator('dialog.dm-dialog').waitFor({ state: 'hidden' });

  // 1. The note is in place, from the page.
  await page.locator('.dm-note').first().waitFor();
  await shot('1-made');
  let now = await shown();
  check('the new note shows in place', now.cards.length === 1 && now.cards[0].state === 'exact', JSON.stringify(now.cards));
  const first = await noteJson(1);
  check('a note saved on the page is recorded as the page', sources(first).join() === 'made reader page' && !first.history[0].warning, JSON.stringify(first.history));
  check('a note from the page shows no warning', (await page.locator('.dm-note .dm-warn').count()) === 0);

  // 2. After a reload it is still there, highlighted exactly.
  await page.reload();
  await page.locator('.dm-note').first().waitFor();
  now = await shown();
  check('after a reload the card sits under its paragraph', now.cards.length === 1 && now.cards[0].after.startsWith('Freeze the branch'), JSON.stringify(now.cards));
  check('the highlight covers exactly the selected words', now.highlighted.length === 1 && now.highlighted[0] === SENTENCE, JSON.stringify(now.highlighted));
  await shot('2-reloaded');

  // 3. A paragraph above: still exact, at its new place.
  fs.writeFileSync(file, DOC.replace('## Before the release\n', '## Before the release\n\nA new paragraph, written by an agent.\n'));
  await page.reload();
  await page.locator('.dm-note').first().waitFor();
  now = await shown();
  check('a paragraph added above keeps the note exact', now.cards[0].state === 'exact' && now.highlighted[0] === SENTENCE, JSON.stringify(now));

  // 4. Reworded: fuzzy, with the new words. Deleted: unanchored.
  fs.writeFileSync(file, DOC.replace('two days before the release', 'three days before the release'));
  await page.reload();
  await page.locator('.dm-note').first().waitFor();
  now = await shown();
  check('a reworded sentence makes the note fuzzy', now.cards[0].state === 'fuzzy' && (now.cards[0].now || '').includes('three days'), JSON.stringify(now.cards));
  await shot('4-fuzzy');
  fs.writeFileSync(file, DOC.replace(/Freeze the branch[^\n]*\n\n/, ''));
  await page.reload();
  await page.locator('.dm-review').waitFor();
  now = await shown();
  check('a deleted sentence leaves the note unanchored, kept', now.cards.length === 1 && now.orphans.length === 1, JSON.stringify(now));
  await page.locator('[role=tab]', { hasText: 'Unanchored' }).click();
  await shot('4-unanchored');
  fs.writeFileSync(file, DOC);

  // 5. A change page: a note on a card's words, and a seen mark.
  await page.goto(`${base}/changes/origin/main...feature`);
  await page.waitForFunction(() => document.querySelector('li.change .acts .seen:not([disabled])'));
  await select('li.change .side', 'twice');
  await saveNote('Once is enough.', 'question');
  await page.locator('li.change .dm-note').first().waitFor();
  await page.locator('li.change .acts .seen').first().click();
  await page.waitForFunction(() => document.querySelector('li.change .acts .seen[aria-pressed="true"]'));
  await page.reload();
  await page.locator('li.change .dm-note').first().waitFor();
  const change = await page.evaluate(() => ({
    note: document.querySelector('li.change .dm-note').dataset.state,
    seen: document.querySelector('li.change .acts .seen').getAttribute('aria-pressed'),
    count: document.querySelector('.dm-seen-count') ? document.querySelector('.dm-seen-count').textContent : '',
  }));
  check('a note on a card and its seen mark survive a reload', change.note === 'exact' && change.seen === 'true' && change.count.includes('1 of 1 seen'), JSON.stringify(change));
  await shot('5-change-page');

  // 6. An agent answers the card's note over MCP, the reader decides, the
  //    agent applies it, and the card says which note changed it.
  const agentDir = path.join(work, 'agent');
  git('worktree', 'add', '-q', agentDir, 'feature');
  const agent = spawn(program, ['mcp'], { cwd: agentDir, stdio: ['pipe', 'pipe', 'inherit'] });
  const waiting = new Map();
  readline.createInterface({ input: agent.stdout }).on('line', (line) => {
    const message = JSON.parse(line);
    if (waiting.has(message.id)) {
      waiting.get(message.id)(message);
      waiting.delete(message.id);
    }
  });
  let next = 0;
  const rpc = (method, params) => new Promise((resolve) => {
    next += 1;
    waiting.set(next, resolve);
    agent.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: next, method, params })}\n`);
  });
  const tool = async (name, args) => {
    const answer = (await rpc('tools/call', { name, arguments: args })).result;
    if (answer.isError) throw new Error(`${name}: ${answer.content[0].text}`);
    return JSON.parse(answer.content[0].text);
  };
  try {
    await rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'claude-code', version: '1' } });
    agent.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    const open = await tool('notes_list', { review: 'r2' });
    const id = open.notes[0].id;
    await tool('note_reply', { note: id, stance: 'agree', body: 'Once is enough; the second run finds nothing new.' });
    await tool('note_propose_decision', { note: id, outcome: 'modify', summary: 'Run the full test suite once on the frozen branch.' });
    const answered = await noteJson(id);
    check('the agent\'s actions are recorded as the connector\'s', sources(answered).slice(-2).join() === 'reply agent mcp,propose agent mcp'
      && answered.history.slice(-2).every((event) => event.user_agent === 'claude-code 1' && !event.warning), JSON.stringify(answered.history));
    // The reader decides on the page; the proposal fills the form in.
    await page.reload();
    await page.locator(`#Note${id}`).waitFor();
    await page.locator(`#Note${id} button`, { hasText: 'Decide' }).click();
    await page.locator(`#Note${id} form.dm-inline button[type=submit]`).click();
    await page.waitForFunction((id) => (document.querySelector(`#Note${id}`) || {}).dataset?.status === 'decided', id);
    const decided = await noteJson(id);
    check('the decision on the page is recorded as the page', sources(decided).at(-1) === 'decide reader page' && !decided.history.at(-1).warning, JSON.stringify(decided.history));
    fs.writeFileSync(path.join(agentDir, 'docs/release.md'), DOC.replace('Run the full test suite', 'Run the full test suite once'));
    gitIn(agentDir, 'commit', '-q', '-am', 'apply the note');
    const commit = gitIn(agentDir, 'rev-parse', 'HEAD');
    const applied = await tool('note_mark_applied', { note: id, commits: [commit.slice(0, 7)] });
    check('the agent records the commit over MCP', applied.note.status === 'applied', JSON.stringify(applied.note));
    await page.reload();
    await page.locator('.dm-because').first().waitFor();
    const because = await page.evaluate((id) => ({
      marks: [...document.querySelectorAll('li.change .dm-because')].map((line) => line.textContent),
      link: (document.querySelector('.dm-because a') || {}).getAttribute?.('href'),
      listed: [...document.querySelectorAll(`#Note${id} .dm-caused a`)].map((a) => a.getAttribute('href')),
    }), id);
    check('the changed card says which note changed it', because.marks.length === 1 && because.marks[0] === `Changed because of note n${id}` && because.link === `#Note${id}`, JSON.stringify(because));
    check('the note lists the change it caused', because.listed.length === 1 && because.listed[0].startsWith('#C'), JSON.stringify(because));
    await page.locator('.dm-because a').first().click();
    check('the mark leads to the note', (await page.evaluate(() => location.hash)) === `#Note${id}`);
    await shot('6-changed-because');
  } finally {
    agent.kill();
  }

  // 7. A page whose marker is out of date gets a fresh one and still
  //    counts as the page.
  await page.goto(reading);
  await page.locator('#Note1').waitFor();
  await page.evaluate(() => { document.querySelector('meta[name="driftmark-page"]').content = 'a-marker-the-server-never-gave'; });
  await page.locator('#Note1 button', { hasText: 'Reply' }).click();
  await page.locator('#Note1 form.dm-inline textarea').fill('The mobile team agreed to three days.');
  await page.locator('#Note1 form.dm-inline button[type=submit]').click();
  await page.waitForFunction(() => document.querySelectorAll('#Note1 .dm-thread li').length === 1);
  const renewed = await noteJson(1);
  check('a page with an old marker gets a fresh one and counts as the page', sources(renewed).at(-1) === 'reply reader page', JSON.stringify(renewed.history));

  // 8. The same reader's reply from a script with the token: accepted,
  //    recorded as the API, and shown on the card as a warning.
  const scripted = await fetch(`${base}/api/notes/1/reply`, {
    method: 'POST',
    headers: { Cookie: cookie, 'Content-Type': 'application/json', 'User-Agent': 'curl/8.5.0' },
    body: JSON.stringify({ body: 'Sent from a script.' }),
  });
  check('a write from a script with the token is accepted', scripted.status === 200, String(scripted.status));
  const fromScript = (await scripted.json()).note;
  check('a write from a script is recorded as the API', sources(fromScript).at(-1) === 'reply reader api', JSON.stringify(fromScript.history));
  await page.reload();
  await page.locator('#Note1 .dm-warn').waitFor();
  const warn = await page.evaluate(() => {
    const box = document.querySelector('#Note1 .dm-warn');
    return { text: box.textContent, buttons: box.querySelectorAll('button').length, hidden: box.closest('details') !== null };
  });
  check('the card warns that the reply came through the API', warn.text.includes('Replied through the API, not on the page') && warn.text.includes('curl/8.5.0'), warn.text);
  check('the warning cannot be dismissed or folded away', warn.buttons === 0 && !warn.hidden, JSON.stringify(warn));
  await page.locator('#Note1 .dm-history summary').click();
  await shot('8-warning');
  await accessible('a note with a warning');
  check('no script errors', errors.length === 0, errors.join(' | '));
} catch (error) {
  check('the run finished', false, error.message);
} finally {
  await browser.close();
  server.kill();
  fs.rmSync(work, { recursive: true, force: true });
}
const failed = results.filter((result) => !result.ok);
console.log(`${results.length - failed.length} of ${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
