// Exported files in a real browser, opened from disk with the network off.
//
// Usage: node export.mjs <folder> [<out-dir>]
//
// <folder> holds the files `crates/driftmark/tests/export.rs` writes to
// `<target>/tmp/export/`: doc.html, folder.html, changes.html and
// changes-cut.html, exported from the made-up repository in
// `fixtures/export/`. Run `cargo test -p driftmark --test export` first.
//
// Each file opens from file:// in headless Chromium, with the context
// offline and every request refused. For each one the test checks that:
//
//   - the browser asked for nothing but the file itself;
//   - no console error, page error or Content-Security-Policy violation
//     happened, the file's own policy being the only one;
//   - links inside the file work: a document's link to another document's
//     section shows that document and that section, back links return, and
//     links to documents left out are text that says so;
//   - the drawn diagram shows, in the diagram font the file carries;
//   - code is coloured by the Prism the file carries;
//   - the search finds words, phrases and prefixes, says when it finds
//     nothing, and a result leads to its block;
//   - the theme button switches light and dark;
//   - axe-core 4.14 finds no WCAG 2.1 A or AA problem on each page shown,
//     in light and dark, at 1440 px and 390 px.
//
// Screenshots go to <out-dir> when given. CHROMIUM names the browser
// binary; the default is the cached headless shell that Playwright 1.64.1
// runs with (see the anchoring spike).
import { chromium } from 'playwright';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const [folder, outDir] = process.argv.slice(2);
if (!folder) {
  console.error('usage: node export.mjs <folder> [<out-dir>]');
  process.exit(2);
}
const exe = process.env.CHROMIUM
  || path.join(os.homedir(), '.cache/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-linux64/chrome-headless-shell');
const axeSource = fs.readFileSync(createRequire(import.meta.url).resolve('axe-core/axe.min.js'), 'utf8');
if (outDir) fs.mkdirSync(outDir, { recursive: true });

const results = [];
const check = (name, ok, detail = '') => {
  results.push({ name, ok });
  console.log(`${ok ? 'pass' : 'FAIL'}  ${name}${detail && !ok ? `: ${detail}` : ''}`);
};

const browser = await chromium.launch({ executablePath: exe });
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, offline: true });
// axe is put into each page by the browser itself, before the page's own
// scripts, so the file's security policy does not have to allow it.
await context.addInitScript({ content: axeSource });
await context.addInitScript(() => {
  window.__violations = [];
  document.addEventListener('securitypolicyviolation', (event) => {
    window.__violations.push(`${event.violatedDirective} ${event.blockedURI}`);
  });
});
await context.route('**/*', (route) => {
  if (route.request().url().startsWith('file:')) {
    route.continue();
  } else {
    route.abort();
  }
});

const open = async (name) => {
  const page = await context.newPage();
  const url = pathToFileURL(path.join(folder, name)).href;
  const asked = [];
  const problems = [];
  page.on('request', (request) => {
    if (request.url() !== url && !request.url().startsWith('data:')) asked.push(request.url());
  });
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') problems.push(message.text());
  });
  page.on('pageerror', (error) => problems.push(String(error)));
  await page.goto(url, { waitUntil: 'load' });
  return { page, url, asked, problems };
};

// Only one page of the file shows at a time.
const visiblePages = (page) => page.$$eval('.xpage', (pages) => pages.filter((p) => !p.hidden).map((p) => p.id));

// Clicks a link inside the file and waits until the page it leads to shows:
// the page script switches pages on the hash change, after the click.
const follow = async (page, selector, pageId) => {
  await page.click(selector);
  await page.waitForFunction((id) => {
    const shown = document.getElementById(id);
    return shown && !shown.hidden;
  }, pageId, { timeout: 3000 }).catch(() => {});
  await page.waitForTimeout(50);
};

const shot = async (page, name) => {
  if (outDir) await page.screenshot({ path: path.join(outDir, `${name}.png`), fullPage: false });
};

// axe on what the page shows now, in light and dark, desktop and phone.
const axe = async (page, what) => {
  for (const theme of ['light', 'dark']) {
    for (const [width, height] of [[1440, 900], [390, 844]]) {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize({ width, height });
      await page.waitForTimeout(50);
      const found = await page.evaluate(async () => {
        const result = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'] } });
        return {
          problems: result.violations.map((v) => `${v.id} (${v.nodes.length}): ${v.nodes.slice(0, 2).map((n) => n.target.join(' ')).join(', ')}`),
          passed: result.passes.length,
        };
      });
      check(`axe finds no problem: ${what}, ${theme}, ${width} px`, found.problems.length === 0 && found.passed > 10, found.problems.join('; '));
      await shot(page, `${what.replace(/\W+/g, '-')}-${theme}-${width}`);
    }
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await page.setViewportSize({ width: 1440, height: 900 });
};

const clean = async (opened, name) => {
  const violations = await opened.page.evaluate(() => window.__violations);
  check(`${name}: nothing asked from the network`, opened.asked.length === 0, opened.asked.join(', '));
  check(`${name}: no console error and no policy violation`, opened.problems.length === 0 && violations.length === 0,
    [...opened.problems, ...violations].join(' | '));
};

const search = async (page, query) => {
  await page.fill('#Search', query);
  await page.waitForFunction(() => !document.getElementById('Results').hidden);
  await page.waitForTimeout(250);
  return page.$$eval('#Results li a', (links) => links.map((a) => ({
    href: a.getAttribute('href'),
    title: a.querySelector('.r-title').textContent,
    where: a.querySelector('.r-where').textContent,
    marked: [...a.querySelectorAll('mark')].map((m) => m.textContent),
  })));
};

// ---- one document ------------------------------------------------------------

{
  const opened = await open('doc.html');
  const { page } = opened;
  check('doc: its one page shows', JSON.stringify(await visiblePages(page)) === '["R1"]');
  const diagram = await page.$eval('figure.diagram svg', (svg) => {
    const box = svg.getBoundingClientRect();
    return { width: box.width, height: box.height };
  }).catch(() => null);
  check('doc: the diagram is drawn', diagram !== null && diagram.width > 50 && diagram.height > 20, JSON.stringify(diagram));
  const font = await page.evaluate(async () => {
    await document.fonts.ready;
    return [...document.fonts].some((face) => face.family.replace(/"/g, '') === 'DM Diagram' && face.status === 'loaded');
  });
  check('doc: the diagram font loads from inside the file', font);
  const tokens = await page.$$eval('.doc-body pre code .token', (spans) => spans.length);
  check('doc: code is coloured by Prism', tokens > 3, `${tokens} tokens`);
  const off = await page.$eval('.doc-body .link-off', (span) => getComputedStyle(span, '::after').content);
  check('doc: a link to a document left out says it was not exported', off === '" (not exported)"', off);
  check('doc: there is no search box for one document', await page.$eval('form.search', (form) => form.hidden));
  const before = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  await page.click('[data-theme-button]');
  const after = await page.evaluate(() => [document.documentElement.dataset.theme, getComputedStyle(document.body).backgroundColor]);
  check('doc: the theme button switches to dark', after[0] === 'dark' && after[1] !== before, JSON.stringify([before, after]));
  await page.click('[data-theme-button]');
  await page.evaluate(() => { try { localStorage.removeItem('dm-theme'); } catch (_) { /* none */ } delete document.documentElement.dataset.theme; });
  await page.click('a[href="#R1-fn:tides"]');
  const footnote = await page.evaluate(() => location.hash);
  check('doc: a footnote link stays inside the document', footnote === '#R1-fn:tides', footnote);
  await axe(page, 'doc reading page');
  await clean(opened, 'doc');
  await page.close();
}

// ---- a folder ----------------------------------------------------------------

{
  const opened = await open('folder.html');
  const { page } = opened;
  check('folder: the index shows first, alone', JSON.stringify(await visiblePages(page)) === '["Index"]');
  await axe(page, 'folder index');
  await follow(page, 'a.x-title[href="#R1"]', 'R1');
  check('folder: an index link shows that document', JSON.stringify(await visiblePages(page)) === '["R1"]');
  await follow(page, '#R1 a[href="#R2-install"]', 'R2');
  const section = await page.evaluate(() => {
    const heading = document.getElementById('R2-install');
    const box = heading.getBoundingClientRect();
    return { shown: !heading.closest('.xpage').hidden, top: Math.round(box.top) };
  });
  check('folder: a link to another document\'s section shows it at that section',
    section.shown && section.top >= 0 && section.top < 200, JSON.stringify(section));
  check('folder: the other document is the only page shown', JSON.stringify(await visiblePages(page)) === '["R2"]');
  await page.goBack();
  await page.waitForTimeout(100);
  check('folder: the browser\'s back button returns', JSON.stringify(await visiblePages(page)) === '["R1"]');
  await follow(page, '#R1 .kicker a[href="#Index"]', 'Index');
  check('folder: a page\'s Index link returns to the index', JSON.stringify(await visiblePages(page)) === '["Index"]');

  const word = await search(page, 'storm');
  check('folder search: a word finds the page that holds it', word.length === 1 && word[0].title === 'Field guide' && word[0].marked.includes('storm'), JSON.stringify(word));
  // On a tie the earlier block wins, as on the server: the warning under
  // Install says "storm" too. Two words pick the block that holds both.
  check('folder search: the result names the section of its block', word[0] && word[0].where === 'docs/guide.md › Install', JSON.stringify(word[0]));
  const two = await search(page, 'storm log');
  check('folder search: the block that holds every word wins', two[0] && two[0].where === 'docs/guide.md › Night watch' && two[0].marked.join(' ') === 'storm log', JSON.stringify(two[0]));
  const phrase = await search(page, '"lantern schedule"');
  check('folder search: a phrase finds only its words side by side', phrase.length === 1 && phrase[0].title === '0001 Cache lantern schedules', JSON.stringify(phrase));
  const prefix = await search(page, 'lante');
  check('folder search: the last word is a prefix', prefix.length === 3, JSON.stringify(prefix.map((r) => r.title)));
  check('folder search: a title match ranks above the body', prefix[0] && prefix[0].title === '0001 Cache lantern schedules', JSON.stringify(prefix.map((r) => r.title)));
  const both = await search(page, 'tuesday lanterns ');
  check('folder search: every word must match', both.length === 0 || both.every((r) => r.title === 'Tide tables'), JSON.stringify(both));
  await search(page, 'zzqx');
  const nothing = await page.$eval('#Results', (box) => box.textContent);
  check('folder search: nothing found says so', nothing === 'Nothing found for “zzqx”.', nothing);
  await search(page, 'monday');
  await axe(page, 'folder search results');
  await follow(page, '#Results li a', 'R3');
  const found = await page.evaluate(() => {
    const marked = document.querySelector('.dm-found');
    return marked && { page: marked.closest('.xpage').id, text: marked.textContent, shown: !marked.closest('.xpage').hidden };
  });
  check('folder search: a result opens its page at its block, marked', found && found.page === 'R3' && found.shown && /Monday/.test(found.text), JSON.stringify(found));
  await axe(page, 'folder reading page');
  await clean(opened, 'folder');
  await page.close();
}

// ---- a range's changes ----------------------------------------------------------

{
  const opened = await open('changes.html');
  const { page } = opened;
  check('changes: the change list shows first, alone', JSON.stringify(await visiblePages(page)) === '["Changes"]');
  const cards = await page.$$eval('li.change', (items) => items.length);
  check('changes: every change has its card', cards >= 5, `${cards} cards`);
  const buttons = await page.$$eval('.acts, button.seen, button.note', (items) => items.length);
  check('changes: no card offers Seen or a note', buttons === 0);
  await axe(page, 'changes list');
  await follow(page, '.rail a[href="#C1.0"]', 'Changes');
  check('changes: the rail leads to a card', await page.evaluate(() => location.hash === '#C1.0' && !document.getElementById('C1.0').closest('.xpage').hidden));
  await follow(page, '#D1-t a[href="#R1"]', 'R1');
  check('changes: a document\'s title opens its full text', JSON.stringify(await visiblePages(page)) === '["R1"]');
  await follow(page, '#R1 .kicker a[href="#D1"]', 'Changes');
  check('changes: its changes link returns to its cards', JSON.stringify(await visiblePages(page)) === '["Changes"]');
  const deleted = await page.$eval('#D2-t', (heading) => heading.querySelector('a') === null);
  check('changes: a deleted document\'s title is text', deleted);
  const results = await search(page, 'never cache');
  check('changes search: finds the new sentence in a card and in the document',
    results.some((r) => r.title === 'Changes: 0001 Cache lantern schedules') && results.some((r) => r.title === '0001 Cache lantern schedules'),
    JSON.stringify(results.map((r) => r.title)));
  const card = results.find((r) => r.title.startsWith('Changes:'));
  await follow(page, `#Results a[href="${card.href}"]`, 'Changes');
  check('changes search: a card result shows its card', await page.evaluate((id) => !document.getElementById(id).closest('.xpage').hidden, card.href.slice(1)));
  await follow(page, 'a[href="#R4"]', 'R4');
  await axe(page, 'changes reading page');
  await clean(opened, 'changes');
  await page.close();
}

{
  const opened = await open('changes-cut.html');
  const { page } = opened;
  const note = await page.$eval('.xcut', (box) => box.textContent).catch(() => '');
  check('cut changes: the file says what it left out', /This file holds every change of this range, and the full text of \d+ of the 3 documents it changed/.test(note), note);
  await axe(page, 'cut changes list');
  await clean(opened, 'cut changes');
  await page.close();
}

// ---- without the script --------------------------------------------------------

{
  const plain = await browser.newContext({ viewport: { width: 1440, height: 900 }, offline: true, javaScriptEnabled: false });
  const page = await plain.newPage();
  await page.goto(pathToFileURL(path.join(folder, 'folder.html')).href);
  const shown = await page.$$eval('.xpage', (pages) => pages.filter((p) => p.getBoundingClientRect().height > 0).length);
  check('no script: every page of the file shows, one after another', shown === 4, `${shown} pages`);
  await plain.close();
}

await browser.close();
const failed = results.filter((result) => !result.ok);
console.log(`${results.length - failed.length} of ${results.length} checks passed`);
process.exit(failed.length === 0 ? 0 : 1);
