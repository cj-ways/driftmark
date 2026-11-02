// driftmark's script for an exported file (driftmark export). It sits
// inline in the file's <head>, after the style sheet, and the file's
// security policy allows it by its hash. It loads nothing: no server, no
// network. It sets the reader's theme at once, so the first paint has the
// right colours, and does the rest when the page has loaded:
//
// - the theme button;
// - one page of the file at a time: the page a link points at, or the
//   first page. Without this script every page shows, one after another;
// - "on this page" and the change rail open on wide screens; tables that
//   fit their column are marked so their header can stick;
// - code is coloured by Prism, whose files the export put in the file;
// - the search box searches the index the export built ("SearchIndex").
//
// It has no notes, no "Seen", no live reload and no browser diagrams: the
// file is a snapshot. The search follows the server's search
// (docs/features/engine/search.md) as far as a file can: the index says
// which pages hold each word, and the pages' own text gives phrases,
// ranks and snippets.
"use strict";
(() => {
  const root = document.documentElement;

  // ---- theme ----------------------------------------------------------------

  const stored = (() => {
    try {
      return localStorage.getItem("dm-theme");
    } catch (_) {
      return null;
    }
  })();
  if (stored === "light" || stored === "dark") {
    root.dataset.theme = stored;
  }
  const dark = () =>
    root.dataset.theme
      ? root.dataset.theme === "dark"
      : matchMedia("(prefers-color-scheme: dark)").matches;
  const toggleTheme = () => {
    const next = dark() ? "light" : "dark";
    root.dataset.theme = next;
    try {
      localStorage.setItem("dm-theme", next);
    } catch (_) {
      // A file opened from disk may have no storage; the theme still changes.
    }
  };

  // Prism's files follow this script in the file. They must not colour the
  // page on their own: this script colours one page when it shows.
  window.Prism = window.Prism || {};
  window.Prism.manual = true;

  const data = (id) => {
    const element = document.getElementById(id);
    if (!element) {
      return null;
    }
    try {
      return JSON.parse(element.textContent);
    } catch (_) {
      return null;
    }
  };

  // ---- one page at a time -----------------------------------------------------

  const pages = () => [...document.querySelectorAll(".xpage")];
  let current = null;
  let found = [];

  const fitTables = (scope) => {
    for (const box of scope.querySelectorAll(".doc-body div.table")) {
      box.removeAttribute("data-fits");
      const table = box.querySelector("table");
      if (table && table.scrollWidth <= box.clientWidth + 1) {
        box.dataset.fits = "";
      }
    }
  };

  const openWide = (scope) => {
    for (const details of scope.querySelectorAll("details[data-wide-open]")) {
      const width = details.closest(".rail") ? "47.5em" : "75em";
      if (matchMedia(`(min-width: ${width})`).matches) {
        details.open = true;
      }
    }
  };

  let aliases = {};
  const colourCode = (scope) => {
    const prism = window.Prism;
    if (!prism || typeof prism.highlightElement !== "function") {
      return;
    }
    for (const pre of scope.querySelectorAll(".doc-body pre[data-language]")) {
      const code = pre.querySelector(":scope > code");
      const name = aliases[pre.dataset.language.toLowerCase()];
      if (code && name && prism.languages[name] && !code.dataset.coloured) {
        code.classList.add(`language-${name}`);
        code.dataset.coloured = "";
        prism.highlightElement(code);
      }
    }
  };

  const prepared = new WeakSet();
  const show = (page) => {
    if (!page) {
      return;
    }
    if (page !== current) {
      for (const other of pages()) {
        other.hidden = other !== page;
      }
      current = page;
    }
    if (!prepared.has(page)) {
      prepared.add(page);
      openWide(page);
      colourCode(page);
    }
    fitTables(page);
  };

  const mark = (elements) => {
    for (const element of found) {
      element.classList.remove("dm-found");
    }
    found = elements;
    for (const element of found) {
      element.classList.add("dm-found");
    }
  };

  // A place in the file: an element's id, or `<page id>/k.<n>` for block n
  // of a reading page, which is how search results point at a block.
  const place = (hash) => {
    const target = hash ? document.getElementById(hash) : null;
    if (target) {
      return { target, blocks: [] };
    }
    const block = /^(.+)\/k\.(\d+)$/.exec(hash || "");
    const page = block ? document.getElementById(block[1]) : null;
    if (!page) {
      return { target: null, blocks: [] };
    }
    const blocks = [...page.querySelectorAll(`.doc-body [data-k="${block[2]}"]`)];
    return { target: blocks[0] || page, blocks };
  };

  const go = () => {
    let hash = "";
    try {
      hash = decodeURIComponent(location.hash.slice(1));
    } catch (_) {
      hash = location.hash.slice(1);
    }
    const { target, blocks } = place(hash);
    const page = target ? target.closest(".xpage") : null;
    // A place outside every page, such as the skip link's, keeps the page.
    show(page || current || pages()[0]);
    mark(blocks);
    if (!target) {
      return;
    }
    for (let at = target; at; at = at.parentElement) {
      if (at.tagName === "DETAILS") {
        at.open = true;
      }
    }
    target.scrollIntoView({ block: blocks.length > 0 ? "center" : "start" });
  };

  // ---- search ------------------------------------------------------------------

  // Words compare in the server's folded form: lower case, ß as ss, the
  // final sigma as σ, and the typographic apostrophe as '.
  const fold = (word) =>
    word.toLowerCase().replace(/ß/g, "ss").replace(/ς/g, "σ").replace(/’/g, "'");

  const segmenter = window.Intl && Intl.Segmenter ? new Intl.Segmenter(undefined, { granularity: "word" }) : null;
  // The words of a text with where each starts and ends, folded. Unicode's
  // word boundaries split them, as the index's words were split.
  const wordsOf = (text) => {
    const out = [];
    if (segmenter) {
      for (const part of segmenter.segment(text)) {
        if (part.isWordLike) {
          out.push({ word: fold(part.segment), start: part.index, end: part.index + part.segment.length });
        }
      }
    } else {
      for (const match of text.matchAll(/[\p{L}\p{N}_]+(?:['.:’][\p{L}\p{N}_]+)*/gu)) {
        out.push({ word: fold(match[0]), start: match.index, end: match.index + match[0].length });
      }
    }
    return out;
  };

  // A query: words, "phrases", and a last word still being typed, which is
  // a prefix. A piece that holds several words, such as read-only, is a
  // phrase. Every clause must match.
  const parseQuery = (text) => {
    const clauses = [];
    const add = (words, prefix) => {
      if (words.length === 0) {
        return;
      }
      const key = `${prefix ? "^" : ""}${words.join(" ")}`;
      if (!clauses.some((clause) => clause.key === key)) {
        clauses.push({ key, words, prefix });
      }
    };
    const quote = /["“”„]/;
    let rest = text;
    while (rest.length > 0) {
      rest = rest.replace(/^\s+/, "");
      if (rest.length === 0) {
        break;
      }
      if (quote.test(rest[0])) {
        const close = rest.slice(1).search(quote);
        const inside = close < 0 ? rest.slice(1) : rest.slice(1, close + 1);
        rest = close < 0 ? "" : rest.slice(close + 2);
        const open = close < 0 && !/\s$/.test(inside);
        add(wordsOf(inside).map((w) => w.word), open);
        continue;
      }
      const piece = rest.match(/^\S+/)[0];
      rest = rest.slice(piece.length);
      const last = rest.length === 0;
      add(wordsOf(piece).map((w) => w.word), last);
    }
    return clauses;
  };

  let index = null;
  let wordAt = null;
  const pagesOf = (number) => {
    const out = [];
    let at = 0;
    for (const gap of index.in[number].split(".")) {
      at += parseInt(gap, 36);
      out.push(at);
    }
    return out;
  };
  // The pages that hold a word, or hold a word starting with it.
  const holding = (word, prefix) => {
    if (!prefix) {
      const at = wordAt.get(word);
      return new Set(at === undefined ? [] : pagesOf(at));
    }
    const out = new Set();
    index.words.forEach((known, number) => {
      if (known.startsWith(word)) {
        for (const page of pagesOf(number)) {
          out.add(page);
        }
      }
    });
    return out;
  };

  // The parts of a page that a result can point at, each with its text
  // (page furniture marked data-gen left out) and its words.
  const parts = new Map();
  const textOf = (elements) => {
    let text = "";
    for (const element of elements) {
      const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT, {
        acceptNode: (node) => (node.parentElement.closest("[data-gen]") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
      });
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        text += node.textContent;
      }
      text += " ";
    }
    return text.replace(/\s+/g, " ").trim();
  };
  const partsOf = (page) => {
    if (parts.has(page.id)) {
      return parts.get(page.id);
    }
    const list = [];
    if (page.classList.contains("xread")) {
      const byBlock = new Map();
      for (const element of page.querySelectorAll(".doc-body [data-k]")) {
        const k = element.dataset.k;
        if (!byBlock.has(k)) {
          byBlock.set(k, []);
        }
        byBlock.get(k).push(element);
      }
      for (const [k, elements] of byBlock) {
        const tag = elements[0].tagName;
        const field = tag === "H1" ? 4 : /^H[2-6]$/.test(tag) ? 2 : 1;
        list.push({ elements, field, href: `#${page.id}/k.${k}`, heading: /^H[1-6]$/.test(tag) });
      }
    } else {
      for (const element of page.querySelectorAll("li.change")) {
        list.push({ elements: [element], field: 1, href: `#${element.id}`, heading: false });
      }
    }
    for (const part of list) {
      part.text = textOf(part.elements);
      part.words = wordsOf(part.text);
    }
    parts.set(page.id, list);
    return list;
  };

  // Where a clause stands in a part's words: each match as [first, last].
  const matches = (clause, words) => {
    const out = [];
    const n = clause.words.length;
    for (let at = 0; at + n <= words.length; at += 1) {
      let ok = true;
      for (let i = 0; i < n && ok; i += 1) {
        const word = words[at + i].word;
        const wanted = clause.words[i];
        ok = clause.prefix && i === n - 1 ? word.startsWith(wanted) : word === wanted;
      }
      if (ok) {
        out.push([at, at + n - 1]);
      }
    }
    return out;
  };

  // The best part of a page for a query, and the page's score, as the
  // server ranks: weight × (field + more) per clause, where the weight
  // grows as fewer pages hold the clause.
  const rank = (page, clauses, weights) => {
    const list = partsOf(page);
    let score = 0;
    const perPart = list.map(() => ({ weight: 0, field: 0 }));
    for (let c = 0; c < clauses.length; c += 1) {
      let best = 0;
      let count = 0;
      list.forEach((part, p) => {
        if (matches(clauses[c], part.words).length > 0) {
          best = Math.max(best, part.field);
          count += 1;
          perPart[p].weight += weights[c];
          perPart[p].field = part.field;
        }
      });
      if (count === 0) {
        return null;
      }
      score += weights[c] * (best + 0.5 - 0.5 / count);
    }
    let chosen = -1;
    perPart.forEach((candidate, p) => {
      if (candidate.weight === 0) {
        return;
      }
      if (chosen < 0) {
        chosen = p;
        return;
      }
      const was = perPart[chosen];
      const title = (x) => x.field === 4;
      if (title(was) && !title(candidate)) {
        chosen = p;
      } else if (!title(was) && !title(candidate)) {
        if (candidate.weight > was.weight || (candidate.weight === was.weight && candidate.field > was.field)) {
          chosen = p;
        }
      }
    });
    return { score, part: list[chosen], parts: list };
  };

  // About 160 characters of a part's text around its first match, with the
  // matched words marked, as the server's snippets.
  const snippet = (part, clauses) => {
    const span = document.createElement("span");
    span.className = "r-snippet";
    const hits = [];
    for (const clause of clauses) {
      for (const [first, last] of matches(clause, part.words)) {
        hits.push([part.words[first].start, part.words[last].end]);
      }
    }
    hits.sort((a, b) => a[0] - b[0]);
    const text = part.text;
    let start = hits.length > 0 ? Math.max(0, hits[0][0] - 40) : 0;
    let end = Math.min(text.length, start + 160);
    if (end - start < 160) {
      start = Math.max(0, end - 160);
    }
    if (start > 0) {
      const space = text.indexOf(" ", start);
      start = space >= 0 && space < (hits[0] ? hits[0][0] : end) ? space + 1 : start;
    }
    if (end < text.length) {
      const space = text.lastIndexOf(" ", end);
      end = space > start ? space : end;
    }
    if (start > 0) {
      span.append("…");
    }
    let at = start;
    for (const [from, to] of hits) {
      if (to <= at || from >= end) {
        continue;
      }
      const a = Math.max(from, at);
      span.append(text.slice(at, a));
      const markEl = document.createElement("mark");
      markEl.textContent = text.slice(a, Math.min(to, end));
      span.append(markEl);
      at = Math.min(to, end);
    }
    span.append(text.slice(at, end));
    if (end < text.length) {
      span.append("…");
    }
    return span;
  };

  // The heading above a part, for the result's "where" line.
  const headingAbove = (part, list) => {
    let heading = null;
    for (const other of list) {
      if (other === part) {
        break;
      }
      if (other.heading && other.field !== 4) {
        heading = other.text;
      }
    }
    return part.heading && part.field !== 4 ? part.text : heading;
  };

  const search = (text) => {
    const clauses = parseQuery(text);
    if (clauses.length === 0) {
      return [];
    }
    const total = index.pages.length;
    let candidates = null;
    const weights = [];
    for (const clause of clauses) {
      let held = null;
      clause.words.forEach((word, i) => {
        const set = holding(word, clause.prefix && i === clause.words.length - 1);
        held = held ? new Set([...held].filter((page) => set.has(page))) : set;
      });
      weights.push(Math.log(1 + total / Math.max(1, held.size)));
      candidates = candidates ? new Set([...candidates].filter((page) => held.has(page))) : held;
    }
    const results = [];
    for (const number of candidates) {
      const [id, title, path, kind] = index.pages[number];
      const page = document.getElementById(id);
      if (!page) {
        continue;
      }
      const ranked = rank(page, clauses, weights);
      if (ranked) {
        results.push({ ranked, title, path, kind });
      }
    }
    results.sort((a, b) => b.ranked.score - a.ranked.score || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
    return results.slice(0, 10).map(({ ranked, title, path, kind }) => {
      const heading = headingAbove(ranked.part, ranked.parts);
      return {
        href: ranked.part.href,
        title: kind === "changes" ? `Changes: ${title}` : title,
        where: [path, heading].filter(Boolean).join(" › "),
        snippet: snippet(ranked.part, clauses),
      };
    });
  };

  const startSearch = () => {
    index = data("SearchIndex");
    const form = document.querySelector("form.search");
    const input = document.getElementById("Search");
    const box = document.getElementById("Results");
    if (!index || !form || !input || !box) {
      return;
    }
    wordAt = new Map(index.words.map((word, number) => [word, number]));
    form.hidden = false;
    let timer = 0;
    const close = () => {
      box.hidden = true;
    };
    const showNodes = (nodes) => {
      box.replaceChildren(...nodes);
      box.hidden = false;
    };
    const note = (text) => {
      const p = document.createElement("p");
      p.className = "r-none";
      p.textContent = text;
      showNodes([p]);
    };
    const ask = () => {
      const query = input.value;
      if (query.trim().length < 2) {
        close();
        return;
      }
      const results = search(query);
      if (results.length === 0) {
        note(`Nothing found for “${query.trim()}”.`);
        return;
      }
      const list = document.createElement("ol");
      for (const result of results) {
        const item = document.createElement("li");
        const link = document.createElement("a");
        link.href = result.href;
        const title = document.createElement("span");
        title.className = "r-title";
        title.textContent = result.title;
        const where = document.createElement("span");
        where.className = "r-where";
        where.textContent = result.where;
        link.append(title, where, result.snippet);
        link.addEventListener("click", close);
        item.appendChild(link);
        list.appendChild(item);
      }
      showNodes([list]);
    };
    input.addEventListener("input", () => {
      clearTimeout(timer);
      timer = setTimeout(ask, 120);
    });
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const first = box.querySelector("a");
      if (first) {
        close();
        location.hash = first.getAttribute("href");
      } else {
        ask();
      }
    });
    form.addEventListener("keydown", (event) => {
      const links = [...box.querySelectorAll("a")];
      const at = links.indexOf(document.activeElement);
      if (event.key === "Escape") {
        close();
        input.focus();
      } else if (event.key === "ArrowDown" && links.length > 0) {
        event.preventDefault();
        links[Math.min(at + 1, links.length - 1)].focus();
      } else if (event.key === "ArrowUp" && at >= 0) {
        event.preventDefault();
        if (at === 0) {
          input.focus();
        } else {
          links[at - 1].focus();
        }
      }
    });
    document.addEventListener("click", (event) => {
      if (!form.contains(event.target)) {
        close();
      }
    });
  };

  // ---- start ----------------------------------------------------------------------

  const start = () => {
    const settings = data("ExportSettings") || {};
    aliases = settings.prism || {};
    const button = document.querySelector("[data-theme-button]");
    if (button) {
      button.hidden = false;
      button.title = "Switch between light and dark";
      button.addEventListener("click", toggleTheme);
    }
    startSearch();
    go();
    window.addEventListener("hashchange", go);
    let resized = 0;
    window.addEventListener("resize", () => {
      clearTimeout(resized);
      resized = setTimeout(() => current && fitTables(current), 150);
    });
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
