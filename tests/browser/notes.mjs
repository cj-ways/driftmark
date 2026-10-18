// The notes loop in a real browser, against the real program.
//
// Usage: node notes.mjs <driftmark binary> [<out-dir>]
//
// It makes a small git repo with a made-up document, starts
// `driftmark serve` on 127.0.0.1 with a free port, and in headless Chromium:
//
//   1. selects a sentence on the reading page, presses "Add a note", fills
//      in the form and saves;
//   2. reloads, and finds the note in place: its card under the sentence's
//      block, and a highlight that covers exactly the selected words;
//   3. edits the document on disk (a new paragraph above), reloads, and
//      finds the note still exact, highlighted at its new place;
//   4. rewords the sentence slightly, reloads, and finds it fuzzy, with the
//      new words shown; then deletes it, and finds it in "Unanchored";
//   5. on the change page of a branch: selects words in a card, notes them,
//      marks the card seen, reloads, and finds both kept.
//
// Screenshots of each step go to <out-dir> when given. CHROMIUM names the
// browser binary; the default is the cached headless shell that Playwright
// 1.64.1 runs with (see the anchoring spike).
import { chromium } from 'playwright';
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

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
const git = (...args) => execFileSync('git', ['-C', repo, ...args], {
  env: {
    ...process.env,
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: path.join(work, 'gitconfig'),
    GIT_AUTHOR_NAME: 'Test', GIT_AUTHOR_EMAIL: 'test@example.com',
    GIT_COMMITTER_NAME: 'Test', GIT_COMMITTER_EMAIL: 'test@example.com',
  },
}).toString().trim();
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

const server = spawn(program, ['serve', '--repo', repo, '--addr', '127.0.0.1:0'], { stdio: ['ignore', 'pipe', 'inherit'] });
const base = await new Promise((resolve, reject) => {
  let seen = '';
  server.stdout.on('data', (chunk) => {
    seen += chunk;
    const found = /open (http:\/\/127\.0\.0\.1:\d+)\//.exec(seen);
    if (found) resolve(found[1]);
  });
  server.on('exit', () => reject(new Error(`the server stopped: ${seen}`)));
});

const results = [];
const check = (name, ok, detail = '') => {
  results.push({ name, ok, detail });
  console.log(`${ok ? 'pass' : 'FAIL'}  ${name}${detail ? `: ${detail}` : ''}`);
};

const browser = await chromium.launch({ executablePath: exe });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
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

const saveNote = async (body, stance) => {
  await page.locator('.dm-float').click();
  await page.locator('dialog.dm-dialog').waitFor({ state: 'visible' });
  await page.locator(`dialog input[name=stance][value=${stance}]`).check();
  await page.locator('dialog textarea').fill(body);
  await page.locator('dialog button[type=submit]').click();
  await page.locator('dialog.dm-dialog').waitFor({ state: 'hidden' });
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

const reading = `${base}/doc/worktree/-/docs/release.md`;
try {
  // 1. A note made by selecting text.
  await page.goto(reading);
  await page.waitForFunction(() => window.driftmarkNotes);
  await select('.doc-body', SENTENCE);
  await saveNote('Two days is too short for the mobile team.', 'disagree');
  await page.locator('.dm-note').first().waitFor();
  await shot('1-made');
  let now = await shown();
  check('the new note shows in place', now.cards.length === 1 && now.cards[0].state === 'exact', JSON.stringify(now.cards));

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
