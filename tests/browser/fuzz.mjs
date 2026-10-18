// Random selections on rendered pages of a real repo, mapped to source
// bytes and back.
//
// Usage: node fuzz.mjs <pages-dir> <git-repo> <out.json> [perPage] [limit]
//
// <pages-dir> is what the ignored Rust test `render_corpus` writes with
// DRIFTMARK_PAGES set: pages named by number, the page script, and
// blobs.txt with the blob id of each page in the same order. The sources
// are read with `git cat-file --batch`; nothing is copied to disk. The
// output holds counts and page numbers only.
//
// Each page is opened twice: as written, and with every word wrapped in its
// own <mark> element. Both get the same random selections. A selection
// starts at a random character and is 1 to 1,000 characters long.
import { chromium } from 'playwright';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const [pagesDir, repo, outFile, perPageArg, limitArg] = process.argv.slice(2);
const perPage = +(perPageArg || 5);
const exe = process.env.CHROMIUM
  || path.join(os.homedir(), '.cache/ms-playwright/chromium_headless_shell-1247/chrome-headless-shell-linux64/chrome-headless-shell');

const blobs = fs.readFileSync(path.join(pagesDir, 'blobs.txt'));
const batch = spawnSync('git', ['-C', repo, 'cat-file', '--batch'], { input: blobs, maxBuffer: 1 << 30 }).stdout;
const sources = [];
for (let at = 0; at < batch.length;) {
  const nl = batch.indexOf(10, at);
  const size = +batch.subarray(at, nl).toString().split(' ')[2];
  sources.push(batch.subarray(nl + 1, nl + 1 + size));
  at = nl + 1 + size + 1;
}
const limit = Math.min(sources.length, +(limitArg || sources.length));

// Seeded random numbers, so both variants get the same selections.
function rng(seed) {
  let x = seed >>> 0 || 1;
  return () => { x ^= x << 13; x >>>= 0; x ^= x >> 17; x ^= x << 5; x >>>= 0; return x / 4294967296; };
}

function inPage({ sels, marks }) {
  const A = window.driftmarkAnchor;
  const art = document.querySelector('article');
  if (marks) {
    const texts = [];
    const tw = document.createTreeWalker(art, NodeFilter.SHOW_TEXT);
    for (let n = tw.nextNode(); n; n = tw.nextNode()) texts.push(n);
    for (const t of texts) {
      const parts = t.data.split(/(\s+)/).filter((x) => x !== '');
      if (parts.length < 2) continue;
      const frag = document.createDocumentFragment();
      for (const x of parts) {
        if (/^\s+$/.test(x)) frag.appendChild(document.createTextNode(x));
        else {
          const m = document.createElement('mark');
          m.textContent = x;
          frag.appendChild(m);
        }
      }
      t.replaceWith(frag);
    }
    A.reset();
  }
  const nodes = [];
  let total = 0;
  const w = document.createTreeWalker(art, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) { nodes.push([n, total]); total += n.data.length; }
  function locate(off, isEnd) {
    let lo = 0;
    let hi = nodes.length - 1;
    while (lo < hi) {
      const m = (lo + hi + 1) >> 1;
      if (nodes[m][1] < off || (!isEnd && nodes[m][1] === off)) lo = m; else hi = m - 1;
    }
    const [n, s] = nodes[lo];
    return [n, Math.min(off - s, n.data.length)];
  }
  const out = { total, results: [] };
  let t = performance.now();
  out.pieces = A.pieces().length;
  out.buildMs = performance.now() - t;
  for (const [s0, len] of sels) {
    const s = Math.floor(s0 * total);
    const e = Math.min(total, s + len);
    if (e <= s || nodes.length === 0) { out.results.push(null); continue; }
    const r = document.createRange();
    const a = locate(s, false);
    const b = locate(e, true);
    r.setStart(a[0], a[1]);
    r.setEnd(b[0], b[1]);
    const f = A.toSource(r);
    if (!f) { out.results.push({ none: true, blank: !/[^ \t\n\r\f]/.test(r.toString()) }); continue; }
    const h = A.fromSource(f.start, f.end);
    out.results.push({ s: f.start, e: f.end, exact: f.exact, why: f.why, edges: f.edges, sel: f.text, back: h ? h.text : null });
  }
  return out;
}

const norm = (s) => (s ?? '').replace(/[ \t\n\r]/g, '');
const browser = await chromium.launch({ executablePath: exe });
const page = await browser.newPage();
const tally = {
  pages: 0, selections: 0, mapped: 0, exact: 0, snapped: {}, noneBlank: 0, noneWithText: 0,
  roundTripFail: 0, sourceEdgeFail: 0, splitChangesAnswer: 0,
};
const failures = [];
const builds = [];
for (let ix = 0; ix < limit; ix++) {
  const name = String(ix).padStart(4, '0');
  const rand = rng(ix + 11);
  const sels = [];
  for (let k = 0; k < perPage; k++) {
    const u = rand();
    const len = u < 0.4 ? 1 + Math.floor(rand() * 20) : u < 0.85 ? 20 + Math.floor(rand() * 180) : 200 + Math.floor(rand() * 800);
    sels.push([rand(), len]);
  }
  const res = {};
  for (const variant of ['page', 'split']) {
    await page.goto(`file://${path.resolve(pagesDir, `${name}.html`)}`);
    res[variant] = await page.evaluate(inPage, { sels, marks: variant === 'split' });
  }
  builds.push(res.page.buildMs);
  tally.pages++;
  const src = sources[ix];
  for (let k = 0; k < perPage; k++) {
    for (const variant of ['page', 'split']) {
      const x = res[variant].results[k];
      if (x === null) continue;
      tally.selections++;
      if (x.none) {
        if (x.blank) tally.noneBlank++;
        else { tally.noneWithText++; failures.push({ page: ix, k, variant, kind: 'text without a source range' }); }
        continue;
      }
      tally.mapped++;
      if (x.exact) tally.exact++;
      for (const w of x.why) tally.snapped[w] = (tally.snapped[w] || 0) + 1;
      const ok = x.exact ? norm(x.back) === norm(x.sel)
        : norm(x.back).includes(norm(x.sel)) || norm(x.sel).includes(norm(x.back));
      if (!ok) { tally.roundTripFail++; failures.push({ page: ix, k, variant, kind: 'round trip' }); }
      if (x.exact) {
        const chars = [...x.sel.replace(/^[ \t\n\r\f]+|[ \t\n\r\f]+$/g, '')];
        const first = Buffer.from(chars[0] || '', 'utf8');
        const last = Buffer.from(chars[chars.length - 1] || '', 'utf8');
        const bad = (x.edges[0] === 'v' && !src.subarray(x.s, x.s + first.length).equals(first))
          || (x.edges[1] === 'v' && !src.subarray(x.e - last.length, x.e).equals(last));
        if (bad) { tally.sourceEdgeFail++; failures.push({ page: ix, k, variant, kind: 'source edge' }); }
      }
    }
    const a = res.page.results[k];
    const b = res.split.results[k];
    if (JSON.stringify(a && [a.s, a.e, a.exact, !!a.none]) !== JSON.stringify(b && [b.s, b.e, b.exact, !!b.none])) {
      tally.splitChangesAnswer++;
      failures.push({ page: ix, k, kind: 'splitting the text changed the answer' });
    }
  }
  if (ix % 500 === 0) console.error(`page ${ix}/${limit}`);
}
await browser.close();
builds.sort((x, y) => x - y);
tally.buildMsMedian = builds[builds.length >> 1];
tally.buildMsLargest = builds[builds.length - 1];
fs.writeFileSync(outFile, JSON.stringify({ tally, failures }, null, 1));
console.log(JSON.stringify(tally, null, 1));
process.exitCode = tally.noneWithText + tally.roundTripFail + tally.sourceEdgeFail + tally.splitChangesAnswer === 0 ? 0 : 1;
