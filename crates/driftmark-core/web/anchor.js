// driftmark page script: notes anchoring.
//
// It turns a text selection on a reading page into a byte range of the
// Markdown source, and a source byte range back into a range on the page.
// The rules are in docs/features/engine/html.md, "Position tables".
//
//   driftmarkAnchor.setTable(table, root)  give it the page's position tables
//   driftmarkAnchor.toSource(range)        a DOM Range -> { start, end, exact, why }
//   driftmarkAnchor.fromSource(start, end) a source byte range -> { range, text, exact }
//   driftmarkAnchor.highlight(start, end)  fromSource, painted with the CSS
//                                          Custom Highlight API as "dm-note"
//
// Both directions work on "pieces": stretches of rendered text tied to
// source bytes.
//   v  verbatim: the text is the source bytes; any offset maps exactly.
//   a  atomic:   an entity, an escape, a footnote number; maps only whole.
//   o  object:   an element with no text (an image); maps only whole.
// Pieces come from the table of each block (an array indexed by block, from
// the server) laid over the text of the block's elements: every element
// with data-k="<block>", in page order. Text inside an element marked
// data-gen was added by the page and is skipped.
(function () {
  'use strict';

  const seg = typeof Intl !== 'undefined' && Intl.Segmenter
    ? new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    : null;

  let table = null;
  let root = null;
  let cache = null;

  // UTF-8 byte length of s.slice(from, to).
  function utf8len(s, from, to) {
    let n = 0;
    for (let i = from; i < to; i++) {
      const c = s.charCodeAt(i);
      if (c < 0x80) n += 1;
      else if (c < 0x800) n += 2;
      else if (c >= 0xd800 && c <= 0xdbff && i + 1 < to && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) { n += 4; i++; }
      else n += 3;
    }
    return n;
  }

  // UTF-16 offset in s reached after `bytes` UTF-8 bytes. A byte offset that
  // falls inside a character moves to its start (floor) or end (ceil).
  function utf16At(s, bytes, ceil) {
    let n = 0;
    let i = 0;
    while (i < s.length) {
      const c = s.charCodeAt(i);
      let w = 3;
      let u = 1;
      if (c < 0x80) w = 1;
      else if (c < 0x800) w = 2;
      else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) { w = 4; u = 2; }
      if (n + w > bytes) return ceil && n < bytes ? i + u : i;
      n += w;
      i += u;
    }
    return i;
  }

  // Largest grapheme boundary <= k, smallest >= k.
  function graphemeFloor(t, k) {
    if (k >= t.length) return t.length;
    if (!seg) return k;
    let last = 0;
    for (const g of seg.segment(t)) {
      if (g.index > k) break;
      last = g.index;
    }
    return last;
  }
  function graphemeCeil(t, k) {
    if (k <= 0) return 0;
    if (!seg) return k;
    for (const g of seg.segment(t)) if (g.index >= k) return g.index;
    return t.length;
  }

  // Text inside an element marked data-gen belongs to no block.
  const skipGenerated = {
    acceptNode(n) {
      return n.nodeType === 1 && n.hasAttribute('data-gen') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
    },
  };

  // The text runs and images of one block, over all its elements.
  function blockContent(elements) {
    const runs = [];
    const imgs = [];
    let r = 0;
    for (const el of elements) {
      const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, skipGenerated);
      for (let n = w.nextNode(); n; n = w.nextNode()) {
        if (n.nodeType === 3) { runs.push({ node: n, r0: r, r1: r + n.data.length }); r += n.data.length; }
        else if (n.tagName === 'IMG') imgs.push(n);
      }
    }
    return { runs, imgs };
  }

  // Lays one block's table over its text and adds its pieces to `out`.
  function layBlock(d, elements, out) {
    const { runs, imgs } = blockContent(elements);
    let i = 1;
    let rp = 0;
    let sp = d[0];
    let ri = 0;
    let ii = 0;
    while (i < d.length) {
      const rgap = d[i++];
      const t = d[i++];
      const sgap = d[i++];
      const rlen = t >> 1;
      const atomic = (t & 1) === 1;
      const r0 = rp + rgap;
      const s = sp + sgap;
      let e = atomic ? s + d[i++] : 0;
      if (atomic && rlen === 0) {
        const el = imgs[ii++];
        if (el) out.push({ kind: 'o', el, s, e });
        rp = r0;
        sp = e;
        continue;
      }
      // A piece may cover several text nodes when other code split the
      // block's text (highlighting, search marks). Cut it per node.
      const r1 = r0 + rlen;
      while (ri < runs.length && runs[ri].r1 <= r0) ri++;
      if (ri >= runs.length) break;
      let k = ri;
      let bytes = s;
      const parts = [];
      while (k < runs.length && runs[k].r0 < r1) {
        const run = runs[k];
        const a = Math.max(r0, run.r0) - run.r0;
        const b = Math.min(r1, run.r1) - run.r0;
        if (b > a) {
          const n = atomic ? 0 : utf8len(run.node.data, a, b);
          parts.push({ kind: atomic ? 'a' : 'v', node: run.node, a, b, s: atomic ? s : bytes, e: atomic ? e : bytes + n });
          bytes += n;
        }
        if (run.r1 > r1) break;
        k++;
      }
      if (!atomic) e = bytes;
      parts.forEach((x, j) => {
        if (atomic && j > 0) x.notFirst = true;
        if (atomic && j < parts.length - 1) x.notLast = true;
        out.push(x);
      });
      rp = r1;
      sp = e;
    }
  }

  // Every piece of the page, in page order. The elements of one block sit
  // next to each other (the paragraphs of one list item), so each run of
  // elements with the same data-k is one block.
  function build() {
    const out = [];
    if (!table) return out;
    const scope = root || document;
    let group = [];
    let k = -1;
    const flush = () => {
      const d = table[k];
      if (group.length && d && d.length) layBlock(d, group, out);
      group = [];
    };
    for (const el of scope.querySelectorAll('[data-k]')) {
      const n = +el.getAttribute('data-k');
      if (n !== k) { flush(); k = n; }
      group.push(el);
    }
    flush();
    return out;
  }

  function pieces() {
    if (!cache) cache = build();
    return cache;
  }

  function indexIn(el) {
    return Array.prototype.indexOf.call(el.parentNode.childNodes, el);
  }

  // Boundary points of a piece.
  function startPoint(p) {
    return p.kind === 'o' ? [p.el.parentNode, indexIn(p.el)] : [p.node, p.a];
  }
  function endPoint(p) {
    return p.kind === 'o' ? [p.el.parentNode, indexIn(p.el) + 1] : [p.node, p.b];
  }

  // The part of a piece inside a range, as local offsets [lo, hi), or null.
  function overlap(p, range) {
    if (p.kind === 'o') {
      const [n, i] = startPoint(p);
      return range.comparePoint(n, i) >= 0 && range.comparePoint(n, i + 1) <= 0 ? [0, 0] : null;
    }
    const n = p.node;
    let lo;
    let hi;
    if (range.startContainer === n) lo = range.startOffset;
    else if (range.comparePoint(n, p.a) === 0) lo = p.a;
    else return null;
    if (range.endContainer === n) hi = range.endOffset;
    else if (range.comparePoint(n, p.b) === 0) hi = p.b;
    else return null;
    lo = Math.max(lo, p.a);
    hi = Math.min(hi, p.b);
    return lo < hi ? [lo, hi] : null;
  }

  // First piece whose end is not before the range start, then forward to
  // the first that really overlaps. Binary search: pieces are in page order.
  function firstOverlap(P, range) {
    let lo = 0;
    let hi = P.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      const [n, o] = endPoint(P[m]);
      if (range.comparePoint(n, o) < 0) lo = m + 1; else hi = m;
    }
    for (let i = lo; i < P.length; i++) {
      if (overlap(P[i], range)) return i;
      const [n, o] = startPoint(P[i]);
      if (range.comparePoint(n, o) > 0) break;
    }
    return -1;
  }
  function lastOverlap(P, range) {
    let lo = 0;
    let hi = P.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      const [n, o] = startPoint(P[m]);
      if (range.comparePoint(n, o) <= 0) lo = m + 1; else hi = m;
    }
    for (let i = lo - 1; i >= 0; i--) {
      if (overlap(P[i], range)) return i;
      const [n, o] = endPoint(P[i]);
      if (range.comparePoint(n, o) < 0) break;
    }
    return -1;
  }

  function hasText(n1, o1, n2, o2) {
    const r = document.createRange();
    try {
      r.setStart(n1, o1);
      r.setEnd(n2, o2);
    } catch (e) {
      return false;
    }
    return /[^\s]/.test(r.toString());
  }

  const WS = /[ \t\n\r\f]/;

  // Drop whitespace at both edges of a range. An image counts as content.
  // Walks only the nodes at the two edges, not everything in between.
  function trimRange(range) {
    const r = range.cloneRange();
    let top = r.commonAncestorContainer;
    if (top.nodeType === 3) top = top.parentNode;
    const w = document.createTreeWalker(top, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    const len = (n) => (n.nodeType === 3 ? n.data.length : n.childNodes.length);
    // First node at or after the start boundary.
    let first = r.startContainer;
    if (first.nodeType !== 3) {
      const child = first.childNodes[r.startOffset];
      if (child) first = child;
      else {
        w.currentNode = first;
        let x = w.nextNode();
        while (x && first.contains(x)) x = w.nextNode();
        first = x;
      }
    }
    let found = false;
    if (first) {
      w.currentNode = first;
      for (let n = first; n; n = w.nextNode()) {
        if (r.comparePoint(n, 0) > 0) break;
        if (n.nodeType === 1) {
          if (n.tagName === 'IMG') { found = true; break; }
          continue;
        }
        const from = n === r.startContainer ? r.startOffset : 0;
        const to = n === r.endContainer ? r.endOffset : n.data.length;
        let i = from;
        while (i < to && WS.test(n.data[i])) i++;
        if (i < to) { r.setStart(n, i); found = true; break; }
      }
    }
    if (!found) return null;
    // Last node at or before the end boundary.
    let last = r.endContainer;
    if (last.nodeType !== 3) {
      if (r.endOffset > 0) {
        last = last.childNodes[r.endOffset - 1];
        while (last.lastChild) last = last.lastChild;
      } else {
        w.currentNode = last;
        last = w.previousNode();
      }
    }
    w.currentNode = last;
    for (let n = last; n; n = w.previousNode()) {
      if (r.comparePoint(n, len(n)) < 0) break;
      if (n.nodeType === 1) {
        if (n.tagName === 'IMG') break;
        continue;
      }
      const from = n === r.startContainer ? r.startOffset : 0;
      const to = n === r.endContainer ? r.endOffset : n.data.length;
      let i = to;
      while (i > from && WS.test(n.data[i - 1])) i--;
      if (i > from) { r.setEnd(n, i); break; }
    }
    return r;
  }

  // Selection -> source. Returns { start, end, exact, why } or null.
  //   start: source start of the first selected character
  //   end:   source end of the last selected character
  // Snapping: a partly selected entity, escape, footnote number or grapheme
  // grows to the whole thing; selected text that has no source (an alert
  // title, a footnote back link) is dropped from the edges.
  function toSource(selected) {
    const range = trimRange(selected);
    if (!range) return null;
    const P = pieces();
    const f = firstOverlap(P, range);
    const l = lastOverlap(P, range);
    if (f < 0 || l < 0 || l < f) return null;
    const why = [];
    const p = P[f];
    const q = P[l];
    const po = overlap(p, range);
    const qo = overlap(q, range);
    let start;
    let end;
    let sp;
    let ep;
    if (p.kind === 'v') {
      const t = p.node.data.slice(p.a, p.b);
      const k = graphemeFloor(t, po[0] - p.a);
      if (k !== po[0] - p.a) why.push('grapheme');
      start = p.s + utf8len(t, 0, k);
      sp = [p.node, p.a + k];
    } else {
      if (p.kind === 'a' && (po[0] !== p.a || p.notFirst)) why.push('atomic');
      start = p.s;
      sp = startPoint(p);
    }
    if (q.kind === 'v') {
      const t = q.node.data.slice(q.a, q.b);
      const k = graphemeCeil(t, qo[1] - q.a);
      if (k !== qo[1] - q.a) why.push('grapheme');
      end = q.s + utf8len(t, 0, k);
      ep = [q.node, q.a + k];
    } else {
      if (q.kind === 'a' && (qo[1] !== q.b || q.notLast)) why.push('atomic');
      end = q.e;
      ep = endPoint(q);
    }
    if (range.comparePoint(sp[0], sp[1]) === 0 && hasText(range.startContainer, range.startOffset, sp[0], sp[1])) why.push('unmapped');
    if (range.comparePoint(ep[0], ep[1]) === 0 && hasText(ep[0], ep[1], range.endContainer, range.endOffset)) why.push('unmapped');
    // Page order and source order differ for footnotes, which render at the
    // end. Cover every selected character: the pieces between the edges are
    // selected whole, the first one up to its end, the last one from its start.
    let lo = Math.min(start, end);
    let hi = Math.max(start, end);
    if (l > f) {
      lo = Math.min(lo, q.s);
      hi = Math.max(hi, p.e);
      for (let i = f + 1; i < l; i++) {
        lo = Math.min(lo, P[i].s);
        hi = Math.max(hi, P[i].e);
      }
    }
    if (lo < start || hi > end) {
      why.push('reordered');
      start = lo;
      end = hi;
    }
    return { start, end, exact: why.length === 0, why, edges: [p.kind, q.kind], text: range.toString() };
  }

  // Source -> page. Returns { range, text, exact } or null.
  function fromSource(qs, qe) {
    const P = pieces();
    let f = -1;
    let l = -1;
    for (let i = 0; i < P.length; i++) {
      const p = P[i];
      if (p.s < qe && p.e > qs) {
        if (f < 0) f = i;
        l = i;
      }
    }
    if (f < 0) return null;
    let exact = true;
    const range = document.createRange();
    const p = P[f];
    if (p.kind === 'v') {
      const t = p.node.data.slice(p.a, p.b);
      const k0 = utf16At(t, Math.max(0, qs - p.s), false);
      const k = graphemeFloor(t, k0);
      if (k !== k0 || (qs > p.s && utf8len(t, 0, k) !== qs - p.s)) exact = false;
      range.setStart(p.node, p.a + k);
    } else {
      if (qs > p.s) exact = false;
      const [n, o] = startPoint(p);
      range.setStart(n, o);
    }
    const q = P[l];
    if (q.kind === 'v') {
      const t = q.node.data.slice(q.a, q.b);
      const k0 = utf16At(t, Math.min(q.e, qe) - q.s, true);
      const k = graphemeCeil(t, k0);
      if (k !== k0 || (qe < q.e && utf8len(t, 0, k) !== qe - q.s)) exact = false;
      range.setEnd(q.node, q.a + k);
    } else {
      if (qe < q.e) exact = false;
      const [n, o] = endPoint(q);
      range.setEnd(n, o);
    }
    return { range, text: range.toString(), exact };
  }

  function highlight(qs, qe) {
    const h = fromSource(qs, qe);
    const scope = root || document;
    for (const img of scope.querySelectorAll('img.dm-note')) img.classList.remove('dm-note');
    if (!h) return null;
    if (window.CSS && CSS.highlights && typeof Highlight === 'function') {
      CSS.highlights.set('dm-note', new Highlight(h.range));
    }
    for (const img of scope.querySelectorAll('img')) if (h.range.intersectsNode(img)) img.classList.add('dm-note');
    return h;
  }

  function setTable(t, r) {
    table = t;
    root = r || null;
    cache = null;
  }

  window.driftmarkAnchor = { setTable, toSource, fromSource, highlight, pieces, reset() { cache = null; } };
})();
