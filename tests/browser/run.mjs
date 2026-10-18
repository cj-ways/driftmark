// Runs every notes anchoring case of fixtures/render/anchoring/cases.json in
// headless Chromium, on the test pages the Rust test
// `the_anchoring_fixtures_become_test_pages` writes, with the shipped page
// script crates/driftmark-core/web/anchor.js.
//
// Usage: node run.mjs <pages-dir> <fixtures-dir> <results.json>
//
// Each case runs twice: on the page as written, and after every word was
// wrapped in its own <mark> element, as a highlighter would. For each run:
//   forward: build the selection with the Range API, put it in the page's
//            selection, read it back, map it to source bytes, compare with
//            the expected bytes, found in the fixture file itself;
//   reverse: highlight the expected source range and compare the page text
//            it covers with the selected text.
// CHROMIUM names the browser binary; the default is the cached headless
// shell that Playwright 1.64.1 runs with (see the anchoring spike).
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const [pagesDir, fixturesDir, outFile] = process.argv.slice(2);
const exe = process.env.CHROMIUM
  || path.join(os.homedir(), '.cache/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-linux64/chrome-headless-shell');
const spec = JSON.parse(fs.readFileSync(path.join(fixturesDir, 'cases.json'), 'utf8'));

function nthBytes(buf, text, nth) {
  const needle = Buffer.from(text, 'utf8');
  let at = -1;
  for (let k = 0; k <= nth; k++) {
    at = buf.indexOf(needle, at + 1);
    if (at < 0) return null;
  }
  return [at, at + needle.length];
}

const norm = (s) => (s ?? '').replace(/[ \t\n\r]/g, '');

// Runs inside the page.
function runCase(c) {
  const A = window.driftmarkAnchor;
  const art = document.querySelector('article');
  function textIndex() {
    const nodes = [];
    let full = '';
    const w = document.createTreeWalker(art, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      nodes.push({ node: n, start: full.length });
      full += n.data;
    }
    return { nodes, full };
  }
  function nth(full, s, k, from = 0) {
    let i = from - 1;
    for (let j = 0; j <= k; j++) {
      i = full.indexOf(s, i + 1);
      if (i < 0) return -1;
    }
    return i;
  }
  function locate(nodes, off, isEnd) {
    for (const x of nodes) {
      const len = x.node.data.length;
      if (isEnd ? off > x.start && off <= x.start + len : off >= x.start && off < x.start + len) return [x.node, off - x.start];
    }
    return null;
  }
  function imgsIn(r) {
    let n = 0;
    for (const img of art.querySelectorAll('img')) {
      const p = img.parentNode;
      const i = Array.prototype.indexOf.call(p.childNodes, img);
      if (r.comparePoint(p, i) >= 0 && r.comparePoint(p, i + 1) <= 0) n++;
    }
    return n;
  }
  let range = null;
  if (c.useCurrentSelection) {
    // A mouse gesture already made the selection.
  } else if (c.select_node) {
    range = document.createRange();
    range.selectNode(art.querySelectorAll(c.select_node)[c.nth || 0]);
  } else if (c.select_css) {
    range = document.createRange();
    range.selectNodeContents(art.querySelectorAll(c.select_css)[c.nth || 0]);
  } else if (c.select || c.select_from) {
    const { nodes, full } = textIndex();
    let s;
    let e;
    if (c.select) {
      s = nth(full, c.select, c.select_nth || 0);
      if (s < 0) return { error: 'selection text not found on the page' };
      e = s + c.select.length - (c.trim_end || 0);
    } else {
      s = nth(full, c.select_from, c.select_nth || 0);
      const t = s < 0 ? -1 : full.indexOf(c.select_to, s);
      if (s < 0 || t < 0) return { error: 'selection text not found on the page' };
      e = t + c.select_to.length;
    }
    const a = locate(nodes, s, false);
    const b = locate(nodes, e, true);
    range = document.createRange();
    range.setStart(a[0], a[1]);
    range.setEnd(b[0], b[1]);
  }
  const out = {};
  if (range || c.useCurrentSelection) {
    const sel = getSelection();
    if (range) {
      sel.removeAllRanges();
      sel.addRange(range);
    }
    if (sel.rangeCount === 0) return { error: 'no selection' };
    const r = sel.getRangeAt(0);
    out.selText = r.toString();
    out.selImgs = imgsIn(r);
    out.fwd = A.toSource(r);
  }
  if (c.expStart != null) {
    const h = A.highlight(c.expStart, c.expEnd);
    out.rev = h ? { text: h.text, exact: h.exact, imgs: imgsIn(h.range) } : null;
  }
  return out;
}

// Runs inside the page: screen points for a mouse gesture.
function gesturePoints(c) {
  const art = document.querySelector('article');
  const nodes = [];
  let full = '';
  const w = document.createTreeWalker(art, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) {
    nodes.push({ node: n, start: full.length });
    full += n.data;
  }
  function rectAt(off) {
    for (const x of nodes) {
      if (off >= x.start && off < x.start + x.node.data.length) {
        const r = document.createRange();
        r.setStart(x.node, off - x.start);
        r.setEnd(x.node, off - x.start + 1);
        return r.getBoundingClientRect();
      }
    }
    return null;
  }
  getSelection().removeAllRanges();
  if (c.gesture === 'drag') {
    const s = full.indexOf(c.select_from);
    const t = full.indexOf(c.select_to, s);
    const a = rectAt(s);
    const b = rectAt(t + c.select_to.length - 1);
    return [[a.left + 1, (a.top + a.bottom) / 2], [b.right - 1, (b.top + b.bottom) / 2]];
  }
  const s = full.indexOf(c.select);
  const a = rectAt(s);
  const b = rectAt(s + c.select.length - 1);
  return [[(a.left + b.right) / 2, (a.top + a.bottom) / 2]];
}

// Runs inside the page: split every text node at word edges and wrap each
// word in <mark>, the way a syntax highlighter or a search would. The text
// stays the same; only the nodes change.
function decorate() {
  const art = document.querySelector('article');
  const texts = [];
  const w = document.createTreeWalker(art, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) texts.push(n);
  for (const t of texts) {
    const parts = t.data.split(/(\s+)/).filter((x) => x !== '');
    if (parts.length < 2) continue;
    const frag = document.createDocumentFragment();
    for (const x of parts) {
      if (/^\s+$/.test(x)) frag.appendChild(document.createTextNode(x));
      else {
        const m = document.createElement('mark');
        m.className = 'deco';
        m.textContent = x;
        frag.appendChild(m);
      }
    }
    t.replaceWith(frag);
  }
  window.driftmarkAnchor.reset();
}

const browser = await chromium.launch({ executablePath: exe });
const page = await browser.newPage({ viewport: { width: 1000, height: 900 } });
const variants = ['page', 'words split'];
const results = [];
for (const variant of variants) {
  const byFile = new Map();
  for (const c of spec.cases) {
    if (!byFile.has(c.file)) byFile.set(c.file, []);
    byFile.get(c.file).push(c);
  }
  for (const [file, cases] of byFile) {
    const buf = fs.readFileSync(path.join(fixturesDir, `${file}.md`));
    await page.goto(`file://${path.resolve(pagesDir, `${file}.html`)}`);
    if (variant === 'words split') await page.evaluate(decorate);
    for (const c of cases) {
      const row = { id: c.id, file, variant, snap: !!c.snap };
      if (c.expect_none) {
        const res = await page.evaluate(runCase, c);
        row.fwd = res.fwd;
        row.pass = !res.error && res.fwd === null;
        row.reason = row.pass ? '' : `forward: expected no source range, got ${JSON.stringify(res.fwd)}`;
        results.push(row);
        continue;
      }
      const exp = c.expect != null ? nthBytes(buf, c.expect, c.expect_nth || 0)
        : nthBytes(buf, c.source, c.source_nth || 0);
      row.expected = exp;
      if (!exp) {
        results.push({ ...row, pass: false, reason: 'expected text not found in the fixture' });
        continue;
      }
      let arg = { ...c, expStart: exp[0], expEnd: exp[1] };
      if (c.gesture) {
        const pts = await page.evaluate(gesturePoints, c);
        if (c.gesture === 'dblclick') await page.mouse.dblclick(pts[0][0], pts[0][1]);
        if (c.gesture === 'triple') await page.mouse.click(pts[0][0], pts[0][1], { clickCount: 3 });
        if (c.gesture === 'drag') {
          await page.mouse.move(pts[0][0], pts[0][1]);
          await page.mouse.down();
          await page.mouse.move(pts[1][0], pts[1][1], { steps: 8 });
          await page.mouse.up();
        }
        arg = { ...arg, useCurrentSelection: true };
      }
      const res = await page.evaluate(runCase, arg);
      const reasons = [];
      if (res.error) reasons.push(res.error);
      const twoWay = c.source == null;
      if (twoWay && !res.error) {
        const f = res.fwd;
        if (!f) reasons.push('forward: no source range');
        else {
          if (f.start !== exp[0] || f.end !== exp[1]) {
            reasons.push(`forward: got ${JSON.stringify(buf.subarray(f.start, f.end).toString('utf8'))}`);
          }
          if (f.exact === !!c.snap) reasons.push(`forward: exact=${f.exact} (${f.why.join(',')})`);
        }
        row.fwd = f;
      }
      const wantRev = c.reverse != null ? c.reverse : res.selText;
      const gotRev = res.rev ? res.rev.text : '';
      if (norm(gotRev) !== norm(wantRev)) reasons.push(`reverse: highlighted ${JSON.stringify(gotRev)}`);
      if (twoWay && res.rev && res.rev.imgs !== res.selImgs) reasons.push('reverse: image count differs');
      if (!twoWay && c.snap && res.rev && res.rev.exact) reasons.push('reverse: expected a snap');
      row.rev = res.rev;
      row.pass = reasons.length === 0;
      row.reason = reasons.join('; ');
      results.push(row);
    }
  }
}
await browser.close();
fs.writeFileSync(outFile, JSON.stringify(results, null, 1));

const ids = [...new Set(results.map((r) => r.id))];
const pass = Object.fromEntries(variants.map((v) => [v, 0]));
for (const id of ids) {
  for (const v of variants) {
    const r = results.find((x) => x.id === id && x.variant === v);
    if (r.pass) pass[v] += 1;
    else console.log(`FAIL ${id} (${v}): ${r.reason}`);
  }
}
console.log(`cases ${ids.length}: ${variants.map((v) => `${v} ${pass[v]} pass`).join(', ')}`);
process.exitCode = variants.every((v) => pass[v] === ids.length) ? 0 : 1;
