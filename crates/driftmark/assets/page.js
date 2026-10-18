// driftmark's page script. Every page loads it from <head>, after the style
// sheet. It sets the reader's theme at once, so the first paint has the
// right colours, and does the rest when the page has loaded:
//
// - the theme button and the search box, which need a script to work;
// - "on this page" and the change rail open on screens wide enough to show
//   them beside the text;
// - tables that fit their column are marked, so their header can stick to
//   the page while scrolling (a table that scrolls sideways cannot);
// - code is coloured by Prism, loading only the languages the page uses;
// - a Mermaid diagram the server could not draw is drawn by mermaid.js,
//   loaded only on a page that has one;
// - a page that shows the working tree reloads its content when a file
//   changes, keeping the scroll position;
// - a reading or change page loads notes.js, which runs the reader's notes.
//
// The page's security policy allows no inline script and no inline style,
// so everything here works through classes, attributes and the CSSOM.
// "__ASSETS__" is replaced with the built-in files' addresses when the
// server starts.
"use strict";
(() => {
  const ASSETS = __ASSETS__;
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
      // A private window may refuse; the theme still changes for this page.
    }
    redrawDiagrams();
  };

  // ---- loading the built-in scripts ------------------------------------------

  const loaded = new Map();
  const load = (name) => {
    if (!loaded.has(name)) {
      loaded.set(
        name,
        new Promise((resolve, reject) => {
          const url = ASSETS[name];
          if (!url) {
            reject(new Error(`no built-in file ${name}`));
            return;
          }
          const script = document.createElement("script");
          script.src = url;
          script.onload = () => resolve();
          script.onerror = () => reject(new Error(`${name} did not load`));
          document.head.appendChild(script);
        }),
      );
    }
    return loaded.get(name);
  };

  // The notes script loads anchor.js through this when the reader starts a
  // note.
  window.driftmarkLoad = load;

  // ---- code colours ------------------------------------------------------------

  // Prism's language files that need another one first.
  const PRISM_NEEDS = {
    c: ["clike"], cpp: ["c"], csharp: ["clike"], dart: ["clike"], go: ["clike"],
    gradle: ["clike"], groovy: ["clike"], java: ["clike"], javascript: ["clike"],
    json5: ["json"], jsx: ["markup", "javascript"], kotlin: ["clike"],
    markdown: ["markup"], protobuf: ["clike"], ruby: ["clike"], scala: ["java"],
    "shell-session": ["bash"], solidity: ["clike"], typescript: ["javascript"],
    tsx: ["jsx", "typescript"], "xml-doc": ["markup"],
  };
  // Other names for a language, as code blocks write them.
  const PRISM_ALIASES = {
    sh: "bash", shell: "bash", zsh: "bash", env: "bash", dotenv: "bash",
    cs: "csharp", "c#": "csharp", dotnet: "csharp", "c++": "cpp", cc: "cpp", h: "c", hpp: "cpp",
    dockerfile: "docker", hs: "haskell", js: "javascript", mjs: "javascript", cjs: "javascript",
    jsonc: "json", webmanifest: "json", html: "markup", xml: "markup", svg: "markup",
    kt: "kotlin", kts: "kotlin", tex: "latex", md: "markdown", py: "python", rb: "ruby",
    rs: "rust", golang: "go", ts: "typescript", yml: "yaml", tf: "hcl", terraform: "hcl",
    alloy: "hcl", ps1: "powershell", pwsh: "powershell", proto: "protobuf", gql: "graphql",
    "sh-session": "shell-session", shellsession: "shell-session", sol: "solidity",
  };
  const prismName = (language) => {
    const name = PRISM_ALIASES[language] || language;
    return ASSETS[`prism-${name}.js`] ? name : null;
  };
  // A language and what it needs, the needed ones first.
  const withNeeds = (name, out) => {
    for (const need of PRISM_NEEDS[name] || []) {
      withNeeds(need, out);
    }
    if (!out.includes(name)) {
      out.push(name);
    }
    return out;
  };

  const colourCode = async (scope) => {
    const blocks = [];
    for (const pre of scope.querySelectorAll(".doc-body pre[data-language]")) {
      const code = pre.querySelector(":scope > code");
      const name = prismName(pre.dataset.language.toLowerCase());
      if (code && name && !code.dataset.coloured) {
        blocks.push([code, name]);
      }
    }
    if (blocks.length === 0) {
      return;
    }
    window.Prism = window.Prism || {};
    window.Prism.manual = true;
    try {
      await load("prism-core.js");
      const order = [];
      for (const [, name] of blocks) {
        withNeeds(name, order);
      }
      for (const name of order) {
        await load(`prism-${name}.js`);
      }
    } catch (_) {
      return;
    }
    for (const [code, name] of blocks) {
      code.classList.add(`language-${name}`);
      code.dataset.coloured = "";
      window.Prism.highlightElement(code);
    }
  };

  // ---- diagrams the server could not draw --------------------------------------

  let diagramCount = 0;
  const drawn = [];

  // mermaid.js puts a <style> element in each drawing. The security policy
  // blocks inline styles, so its rules are moved into a style sheet built
  // by this script, which the policy allows.
  const adopt = (svgElement) => {
    for (const style of svgElement.querySelectorAll("style")) {
      try {
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(style.textContent);
        document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
      } catch (_) {
        // An old browser without constructed style sheets draws the diagram
        // with the page's own diagram colours.
      }
      style.remove();
    }
  };

  // A drawing's inline styles: the security policy blocks a `style`
  // attribute in markup, but allows the same declarations set through the
  // element's style object.
  const carryStyles = (element) => {
    const styles = [];
    for (const node of [element, ...element.querySelectorAll("[style]")]) {
      const text = node.getAttribute("style");
      if (text) {
        styles.push([node, text]);
        node.removeAttribute("style");
      }
    }
    return styles;
  };

  const drawOne = async (pre, figure) => {
    const mermaid = window.mermaid;
    const id = `dmBrowser${(diagramCount += 1)}`;
    const code = pre.querySelector("code")?.textContent ?? "";
    try {
      // parse throws on a diagram mermaid.js cannot read, before render
      // would leave its error drawing in the page.
      await mermaid.parse(code);
      const { svg } = await mermaid.render(id, code);
      const parsed = new DOMParser().parseFromString(svg, "text/html");
      const element = parsed.querySelector("svg");
      if (!element) {
        throw new Error("mermaid.js drew nothing");
      }
      adopt(element);
      const styles = carryStyles(element);
      element.setAttribute("role", "img");
      element.setAttribute("aria-label", "Diagram drawn in the browser");
      const placed = document.importNode(element, true);
      figure.replaceChildren(placed);
      // The imported copy has the same structure, so styles go to the
      // element at the same place.
      const before = [element, ...element.querySelectorAll("*")];
      const after = [placed, ...placed.querySelectorAll("*")];
      for (const [node, text] of styles) {
        const at = before.indexOf(node);
        if (at >= 0) {
          after[at].style.cssText = text;
        }
      }
      placed.style.maxWidth = "100%";
    } finally {
      // mermaid.js measures in elements it adds to the page; take them out.
      for (const left of [document.getElementById(`d${id}`), document.getElementById(id)]) {
        if (left && !figure.contains(left)) {
          left.remove();
        }
      }
    }
  };

  const drawDiagrams = async (scope) => {
    const blocks = [...scope.querySelectorAll('pre[data-undrawn][data-diagram="mermaid"]')];
    if (blocks.length === 0) {
      return;
    }
    try {
      await load("mermaid.js");
    } catch (_) {
      return;
    }
    const mermaid = window.mermaid;
    if (!mermaid) {
      return;
    }
    const settings = {
      startOnLoad: false,
      securityLevel: "strict",
      fontFamily: '"DM Diagram", "Liberation Sans", Arial, sans-serif',
    };
    if (dark()) {
      settings.theme = "dark";
    }
    mermaid.initialize(settings);
    for (const pre of blocks) {
      const figure = document.createElement("figure");
      figure.className = "diagram";
      figure.dataset.diagram = "mermaid";
      figure.dataset.drawnBy = "browser";
      try {
        await drawOne(pre, figure);
        pre.hidden = true;
        pre.after(figure);
        drawn.push([pre, figure]);
      } catch (error) {
        pre.dataset.error = String(error && error.message ? error.message : error).slice(0, 300);
      }
    }
  };

  const redrawDiagrams = async () => {
    if (drawn.length === 0 || !window.mermaid) {
      return;
    }
    const settings = {
      startOnLoad: false,
      securityLevel: "strict",
      fontFamily: '"DM Diagram", "Liberation Sans", Arial, sans-serif',
    };
    if (dark()) {
      settings.theme = "dark";
    }
    window.mermaid.initialize(settings);
    for (const [pre, figure] of drawn) {
      if (figure.isConnected) {
        try {
          await drawOne(pre, figure);
        } catch (_) {
          // The drawing in the other theme stays.
        }
      }
    }
  };

  // ---- tables, folds and places ------------------------------------------------

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

  // A place a link points at may sit inside a closed fold; open it.
  const reveal = () => {
    const hash = decodeURIComponent(location.hash.slice(1));
    if (!hash) {
      return;
    }
    let target = document.getElementById(hash);
    const block = /^k\.(\d+)$/.exec(hash);
    if (!target && block) {
      target = document.querySelector(`[data-k="${block[1]}"]`);
      if (target) {
        target.classList.add("dm-found");
        target.scrollIntoView({ block: "center" });
      }
    }
    for (let at = target; at; at = at.parentElement) {
      if (at.tagName === "DETAILS") {
        at.open = true;
      }
    }
    if (target && target.closest("details")) {
      target.scrollIntoView();
    }
  };

  // Notes: on a reading or change page whose notes file can be read
  // (docs/features/notes.md), the notes script takes over.
  const startNotes = (scope) => {
    const main = scope.matches && scope.matches("main[data-notes]") ? scope : scope.querySelector("main[data-notes]");
    if (!main) {
      return;
    }
    load("notes.js")
      .then(() => window.driftmarkNotes && window.driftmarkNotes.start(main))
      .catch(() => {});
  };

  const prepare = (scope) => {
    openWide(scope);
    fitTables(scope);
    colourCode(scope);
    drawDiagrams(scope);
    startNotes(scope);
  };

  // ---- live reload -------------------------------------------------------------

  // The path of the document this page shows, when it shows one.
  const pagePath = () => {
    const at = location.pathname.indexOf("/-/");
    if (at < 0) {
      return null;
    }
    try {
      return decodeURIComponent(location.pathname.slice(at + 3));
    } catch (_) {
      return null;
    }
  };

  let refreshing = false;
  let again = false;
  const refresh = async () => {
    if (refreshing) {
      again = true;
      return;
    }
    refreshing = true;
    try {
      const answer = await fetch(location.href, { cache: "no-store", credentials: "same-origin" });
      if (!answer.ok) {
        location.reload();
        return;
      }
      const fresh = new DOMParser().parseFromString(await answer.text(), "text/html");
      const pick = (doc) => doc.querySelector(".reviewwrap") || doc.querySelector("main");
      const now = pick(document);
      const next = pick(fresh);
      if (!now || !next) {
        location.reload();
        return;
      }
      const y = window.scrollY;
      const adopted = document.importNode(next, true);
      now.replaceWith(adopted);
      document.title = fresh.title;
      // The fresh page's marker, which the notes script sends with writes.
      const marker = fresh.querySelector('meta[name="driftmark-page"]');
      const mine = document.querySelector('meta[name="driftmark-page"]');
      if (marker && mine) {
        mine.content = marker.content;
      }
      prepare(adopted);
      window.scrollTo(0, y);
    } catch (_) {
      location.reload();
    } finally {
      refreshing = false;
      if (again) {
        again = false;
        refresh();
      }
    }
  };

  const listen = () => {
    const live = document.body.dataset.live;
    if (!live || !window.EventSource) {
      return;
    }
    const events = new EventSource("/events");
    if (live === "home") {
      events.addEventListener("counted", refresh);
      return;
    }
    const mine = pagePath();
    events.addEventListener("change", (event) => {
      let paths = [];
      try {
        paths = JSON.parse(event.data).paths || [];
      } catch (_) {
        paths = [];
      }
      const isDocument = location.pathname.startsWith("/doc/");
      if (paths.length === 0 || !isDocument || !mine || paths.includes(mine)) {
        refresh();
      }
    });
  };

  // ---- search -------------------------------------------------------------------

  // Reads one result of GET /api/search, whose JSON the search lane
  // documents in docs/features/commands/serve.md: the reading page's `url`,
  // the `title`, the `headings` above the best block, the `block` itself
  // and the `snippet` as pieces. The link goes to the exact block (`#k.<n>`,
  // which this script scrolls to) rather than to its section.
  const readResult = (result, revision) => {
    const path = typeof result.path === "string" ? result.path : "";
    const headings = Array.isArray(result.headings) ? result.headings : [];
    const block = Number.isInteger(result.block) ? result.block : null;
    let href;
    if (typeof result.url === "string" && result.url.startsWith("/doc/")) {
      href = result.url.split("#")[0];
    } else {
      const at = encodeURIComponent(revision || "worktree").replaceAll("%2F", "/");
      href = `/doc/${at}/-/${path.split("/").map(encodeURIComponent).join("/")}`;
    }
    if (block !== null) {
      href += `#k.${block}`;
    } else if (typeof result.url === "string" && result.url.includes("#")) {
      href += result.url.slice(result.url.indexOf("#"));
    }
    return { path, title: result.title || path, headings, snippet: result.snippet, href };
  };

  // A snippet with its matched words marked: either a list of parts, each
  // with its text and whether it matched, or plain text.
  const snippetNode = (snippet) => {
    const span = document.createElement("span");
    span.className = "r-snippet";
    if (Array.isArray(snippet)) {
      for (const part of snippet) {
        const text = typeof part === "string" ? part : part.text || "";
        const hit = typeof part === "object" && part !== null && part.hit === true;
        if (hit) {
          const mark = document.createElement("mark");
          mark.textContent = text;
          span.appendChild(mark);
        } else {
          span.appendChild(document.createTextNode(text));
        }
      }
    } else if (typeof snippet === "string") {
      span.textContent = snippet;
    }
    return span;
  };

  const startSearch = () => {
    const form = document.querySelector("form.search");
    const input = document.getElementById("Search");
    const box = document.getElementById("Results");
    if (!form || !input || !box) {
      return;
    }
    form.hidden = false;
    const revision = document.body.dataset.revision || "";
    let timer = 0;
    let asked = "";
    let missing = false;
    const close = () => {
      box.hidden = true;
    };
    const show = (nodes) => {
      box.replaceChildren(...nodes);
      box.hidden = false;
    };
    const note = (text) => {
      const p = document.createElement("p");
      p.className = "r-none";
      p.textContent = text;
      show([p]);
    };
    const ask = async () => {
      const query = input.value.trim();
      if (query.length < 2) {
        close();
        return;
      }
      if (missing) {
        note("Search is not available on this server yet.");
        return;
      }
      asked = query;
      let params = `q=${encodeURIComponent(query)}&limit=10`;
      if (revision) {
        params += `&rev=${encodeURIComponent(revision)}`;
      }
      let data;
      try {
        const answer = await fetch(`/api/search?${params}`, { credentials: "same-origin" });
        data = await answer.json().catch(() => ({}));
        if (answer.status === 404 && !Array.isArray(data.results) && /nothing is at/.test(data.error || "")) {
          // A server built before search: the route does not exist.
          missing = true;
          note("Search is not available on this server yet.");
          return;
        }
        if (!answer.ok) {
          note(data.error || "The search failed.");
          return;
        }
      } catch (_) {
        note("The search failed.");
        return;
      }
      if (asked !== query) {
        return;
      }
      const results = Array.isArray(data.results) ? data.results : [];
      if (results.length === 0) {
        note(`Nothing found for “${query}”.`);
        return;
      }
      const list = document.createElement("ol");
      const at = data.commit || data.revision || revision;
      for (const raw of results) {
        const result = readResult(raw, at);
        const item = document.createElement("li");
        const link = document.createElement("a");
        link.href = result.href;
        const title = document.createElement("span");
        title.className = "r-title";
        title.textContent = result.title;
        const where = document.createElement("span");
        where.className = "r-where";
        where.textContent = [result.path, ...result.headings].join(" › ");
        link.append(title, where, snippetNode(result.snippet));
        item.appendChild(link);
        list.appendChild(item);
      }
      show([list]);
    };
    input.addEventListener("input", () => {
      clearTimeout(timer);
      timer = setTimeout(ask, 150);
    });
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const first = box.querySelector("a");
      if (first) {
        location.href = first.href;
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
    const button = document.querySelector("[data-theme-button]");
    if (button) {
      button.hidden = false;
      button.title = "Switch between light and dark";
      button.addEventListener("click", toggleTheme);
    }
    startSearch();
    prepare(document);
    reveal();
    window.addEventListener("hashchange", reveal);
    let resized = 0;
    window.addEventListener("resize", () => {
      clearTimeout(resized);
      resized = setTimeout(() => fitTables(document), 150);
    });
    listen();
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
