// driftmark's notes script. page.js loads it on reading pages and change
// pages whose <main> carries data-notes (the notes file can be read). It:
//
// - shows a floating "Add a note" button when the reader selects text in a
//   document or a change card, and a form for the note: stance, ask,
//   blocking, text;
// - turns a reading page's selection into source bytes with anchor.js and
//   the page's position tables, which it loads only now (html.md);
// - on a change page, sends the selected text, its side and its block, and
//   activates every card's "Seen" and "Add a note" buttons;
// - shows each note in place (a highlight and a card under its block), a
//   review list with its notes, an "Unanchored" tab for notes whose text is
//   gone, and the buttons that hand a review to an agent;
// - lets the reader reply, decide, edit and withdraw;
// - on a change page, marks each card a note's applied commits made,
//   "Changed because of note n12", and lists those changes on the note's
//   card.
//
// Every text from the server goes into the page through textContent, never
// as HTML. The routes are in docs/features/commands/serve.md ("Notes").
"use strict";
(() => {
  const STANCES = [
    ["agree", "✓ Agree"],
    ["disagree", "✗ Disagree"],
    ["question", "? Question"],
    ["suggestion", "✎ Suggestion"],
  ];
  const ASKS = [
    ["change", "Change it"],
    ["explain", "Explain it"],
    ["keep", "Keep it"],
  ];
  const OUTCOMES = [
    ["change", "Change it as the note asks"],
    ["modify", "Change it another way"],
    ["keep", "Keep it as it is"],
  ];
  const STATUS = {
    open: "Open", sent: "Sent", answered: "Answered", discussing: "Discussing",
    decided: "Decided", applied: "Applied", closed: "Closed without change", withdrawn: "Withdrawn",
  };
  const STATE = {
    exact: "",
    moved: "Moved since the note",
    fuzzy: "Found with small differences",
    outdated: "The text changed after this note",
    orphaned: "Its text is gone",
  };
  const FINAL = new Set(["applied", "closed", "withdrawn"]);
  const word = (list, value) => (list.find(([v]) => v === value) || [value, value])[1];

  // ---- small helpers ------------------------------------------------------------

  const el = (tag, attrs = {}, ...children) => {
    const node = document.createElement(tag);
    for (const [name, value] of Object.entries(attrs)) {
      if (value === false || value === null || value === undefined) continue;
      if (name === "class") node.className = value;
      else if (name === "text") node.textContent = value;
      else if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
      else node.setAttribute(name, value === true ? "" : value);
    }
    for (const child of children) {
      if (child !== null && child !== undefined) node.append(child);
    }
    return node;
  };

  const call = async (method, url, body) => {
    const options = { method, credentials: "same-origin", headers: {} };
    if (body !== undefined) {
      options.headers["Content-Type"] = "application/json";
      options.body = JSON.stringify(body);
    }
    const answer = await fetch(url, options);
    const data = await answer.json().catch(() => ({}));
    if (!answer.ok) {
      throw new Error(data.error || `the server answered ${answer.status}`);
    }
    return data;
  };

  // A message the page reads out: under the review list when there is one,
  // otherwise in a box at the foot of the screen.
  let toast = null;
  const sayAlone = (text) => {
    if (!toast) {
      toast = el("p", { class: "dm-toast", role: "status", "aria-live": "polite" });
      document.body.append(toast);
    }
    toast.textContent = text;
    toast.hidden = false;
    clearTimeout(sayAlone.timer);
    sayAlone.timer = setTimeout(() => {
      toast.hidden = true;
    }, 6000);
  };
  let say = sayAlone;

  const loadAnchor = () => (window.driftmarkLoad ? window.driftmarkLoad("anchor.js") : Promise.reject());

  // ---- the page ------------------------------------------------------------------

  let main = null;
  let kind = "";
  let tables = null;
  let notes = [];
  let reviews = new Map();
  let file = "";
  let floating = null;
  let pending = null;

  const body = () => (main ? main.querySelector(".doc-body") : null);

  // The position tables of a reading page, loaded once per page.
  const positions = async () => {
    if (tables) return tables;
    await loadAnchor();
    const url = location.pathname.replace(/^\/doc\//, "/api/positions/");
    const answer = await fetch(url, { credentials: "same-origin" });
    if (!answer.ok) throw new Error("the position tables did not load");
    tables = await answer.json();
    window.driftmarkAnchor.setTable(tables, body());
    return tables;
  };

  const listUrl = () => {
    if (kind === "read") {
      return `/api/notes?path=${encodeURIComponent(main.dataset.path)}&rev=${encodeURIComponent(main.dataset.rev)}`;
    }
    let url = `/api/notes?range=${encodeURIComponent(main.dataset.range)}`;
    if (main.dataset.path) url += `&path=${encodeURIComponent(main.dataset.path)}`;
    return url;
  };

  const refresh = async () => {
    try {
      const data = await call("GET", listUrl());
      notes = Array.isArray(data.notes) ? data.notes : [];
      file = data.file || "";
      reviews = new Map();
      if (data.review) reviews.set(data.review.id, data.review);
      for (const review of data.reviews || []) reviews.set(review.id, review);
      await show();
    } catch (error) {
      say(`Notes could not be loaded: ${error.message}`);
    }
  };

  // ---- the form ------------------------------------------------------------------

  let dialog = null;

  const radios = (name, list, chosen) =>
    el("fieldset", { class: "dm-choice" },
      el("legend", { text: name === "stance" ? "Your stance" : "What should happen" }),
      ...list.map(([value, label]) =>
        el("label", {},
          el("input", { type: "radio", name, value, checked: value === chosen }),
          ` ${label}`)));

  // Opens the note form. `quote` is the text shown; `send(fields)` saves.
  const openForm = (title, quote, start, send) => {
    if (dialog) dialog.remove();
    const text = el("textarea", { id: "NoteText", rows: "5", maxlength: "20000", required: true });
    text.value = start.body || "";
    const blocking = el("input", { type: "checkbox", id: "NoteBlocking", checked: Boolean(start.blocking) });
    const problem = el("p", { class: "dm-problem", role: "alert", hidden: true });
    const form = el("form", { class: "dm-form", method: "dialog" },
      el("h2", { id: "NoteTitle", text: title }),
      quote ? el("blockquote", { class: "dm-quote", text: quote }) : null,
      radios("stance", STANCES, start.stance || "disagree"),
      radios("ask", ASKS, start.ask || "change"),
      el("p", { class: "dm-check" }, blocking, el("label", { for: "NoteBlocking", text: " Blocking: hold the work until this is settled" })),
      el("p", {}, el("label", { for: "NoteText", text: "Your point" }), text),
      problem,
      el("p", { class: "dm-buttons" },
        el("button", { type: "submit", class: "dm-primary", text: "Save the note" }),
        el("button", { type: "button", text: "Cancel", onclick: () => dialog.close() })));
    dialog = el("dialog", { class: "dm-dialog", "aria-labelledby": "NoteTitle" }, form);
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const fields = {
        stance: form.elements.stance.value,
        ask: form.elements.ask.value,
        blocking: blocking.checked,
        body: text.value.trim(),
      };
      if (!fields.body) {
        problem.hidden = false;
        problem.textContent = "Write your point first.";
        text.focus();
        return;
      }
      try {
        await send(fields);
        dialog.close();
        await refresh();
      } catch (error) {
        problem.hidden = false;
        problem.textContent = error.message;
      }
    });
    document.body.append(dialog);
    dialog.showModal();
    text.focus();
  };

  // ---- selections ----------------------------------------------------------------

  const selectionIn = () => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
    const range = selection.getRangeAt(0);
    if (!/\S/.test(range.toString())) return null;
    const scope = kind === "read" ? body() : main;
    if (!scope || !scope.contains(range.commonAncestorContainer)) return null;
    if (kind === "review" && !cardSide(range)) return null;
    return range;
  };

  const hideFloating = () => {
    if (floating) floating.hidden = true;
  };

  const placeFloating = () => {
    const range = selectionIn();
    if (!range) {
      hideFloating();
      return;
    }
    pending = range.cloneRange();
    if (!floating) {
      floating = el("button", {
        type: "button", class: "dm-float", text: "Add a note",
        onmousedown: (event) => event.preventDefault(),
        onclick: () => {
          hideFloating();
          if (pending) noteOn(pending);
        },
      });
      document.body.append(floating);
    }
    const box = range.getBoundingClientRect();
    floating.hidden = false;
    const top = window.scrollY + box.bottom + 8;
    const left = Math.max(8, Math.min(window.scrollX + box.left, window.scrollX + document.documentElement.clientWidth - floating.offsetWidth - 8));
    floating.style.top = `${top}px`;
    floating.style.left = `${left}px`;
  };

  // ---- notes on a reading page -----------------------------------------------------

  const noteOn = async (range) => {
    if (kind === "review") {
      const side = cardSide(range);
      if (side) noteOnCard(side.card, side, range);
      return;
    }
    let mapped;
    try {
      await positions();
      window.driftmarkAnchor.reset();
      mapped = window.driftmarkAnchor.toSource(range);
    } catch (error) {
      say(`A note cannot start here: ${error.message}`);
      return;
    }
    if (!mapped) {
      say("This selection holds no text of the document. Select some words.");
      return;
    }
    openForm("Add a note", range.toString().trim(), {}, (fields) =>
      call("POST", "/api/notes", {
        path: main.dataset.path,
        revision: main.dataset.rev,
        start: mapped.start,
        end: mapped.end,
        ...fields,
      }));
  };

  // ---- notes on a change page --------------------------------------------------------

  // The side of a change card a range sits in: which version, which block,
  // the text shown on that side, and where the selection starts in it.
  const cardSide = (range) => {
    let node = range.commonAncestorContainer;
    if (node.nodeType === 3) node = node.parentElement;
    const card = node && node.closest ? node.closest("li.change") : null;
    if (!card || card.classList.contains("document")) return null;
    const box = node.closest("[data-side]") || (card.contains(range.startContainer) ? card.querySelector("[data-side]") : null);
    if (!box || !card.contains(box)) return null;
    let side = box.dataset.side;
    if (side === "both") {
      const inDel = (n) => (n.nodeType === 3 ? n.parentElement : n).closest("del");
      side = inDel(range.startContainer) && inDel(range.endContainer) && inDel(range.startContainer) === inDel(range.endContainer) ? "old" : "new";
    }
    const block = side === "old" ? card.dataset.old : card.dataset.new;
    const skip = side === "old" ? "ins" : "del";
    const walker = document.createTreeWalker(box, NodeFilter.SHOW_TEXT);
    let text = "";
    let hint = -1;
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (n.parentElement.closest(`${skip}, [data-gen]`)) continue;
      const from = n === range.startContainer ? range.startOffset : 0;
      const to = n === range.endContainer ? range.endOffset : n.data.length;
      const inside = range.intersectsNode(n);
      if (inside && hint < 0) hint = [...text].length + [...n.data.slice(0, from)].length;
      if (inside) text += n.data.slice(from, to);
    }
    return { card, side, block: Number(block), text, hint: Math.max(0, hint) };
  };

  const documentOf = (card) => card.closest("ol.changes[data-path]").dataset.path;

  const noteOnCard = (card, side, range) => {
    const path = documentOf(card);
    const chosen = side || {
      side: card.dataset.kind === "removed" ? "old" : "new",
      block: Number(card.dataset.kind === "removed" ? card.dataset.old : card.dataset.new),
      text: "",
      hint: 0,
    };
    const shown = range ? chosen.text.trim() : "";
    const title = shown ? "Add a note" : "Add a note on this change";
    openForm(title, shown, {}, (fields) =>
      call("POST", "/api/notes", {
        path,
        range: main.dataset.range,
        side: chosen.side,
        block: chosen.block,
        text: shown || undefined,
        hint: chosen.hint,
        ...fields,
      }));
  };

  // The "Seen" and "Add a note" buttons of every card.
  const activateCards = () => {
    for (const section of main.querySelectorAll("section.docchg[data-keys]")) {
      const keys = section.dataset.keys ? section.dataset.keys.split(" ") : [];
      const seen = new Set(section.dataset.seen ? section.dataset.seen.split(" ") : []);
      for (const card of section.querySelectorAll("li.change")) {
        const index = Number(card.id.split(".").pop());
        const key = keys[index];
        const seenButton = card.querySelector(".acts .seen");
        const noteButton = card.querySelector(".acts .note");
        if (seenButton && key) {
          seenButton.disabled = false;
          seenButton.title = "Mark this change seen";
          seenButton.setAttribute("aria-pressed", seen.has(key) ? "true" : "false");
          card.classList.toggle("is-seen", seen.has(key));
          seenButton.onclick = async () => {
            const now = seenButton.getAttribute("aria-pressed") !== "true";
            try {
              await call("POST", "/api/seen", { key, seen: now });
              seenButton.setAttribute("aria-pressed", now ? "true" : "false");
              card.classList.toggle("is-seen", now);
              countSeen();
            } catch (error) {
              say(`The seen mark was not saved: ${error.message}`);
            }
          };
        }
        if (noteButton) {
          noteButton.disabled = false;
          if (card.classList.contains("document")) {
            noteButton.title = "Open the document to note on its text";
            noteButton.onclick = () => {
              const link = section.querySelector(".docchg-head h2 a");
              if (link) location.href = link.href;
            };
          } else {
            noteButton.title = "Note on this change, or on the words selected in it";
            noteButton.onclick = () => {
              const range = selectionIn();
              const side = range ? cardSide(range) : null;
              noteOnCard(card, side && side.card === card ? side : null, side && side.card === card ? range : null);
            };
          }
        }
      }
    }
    countSeen();
  };

  const countSeen = () => {
    const progress = main.parentElement && main.parentElement.querySelector(".rail-progress");
    if (!progress) return;
    const all = main.querySelectorAll("li.change .acts .seen:not([disabled])").length;
    const done = main.querySelectorAll('li.change .acts .seen[aria-pressed="true"]').length;
    let count = progress.querySelector(".dm-seen-count");
    if (!count) {
      count = el("span", { class: "dm-seen-count" });
      progress.append(count);
    }
    count.textContent = ` · ${done} of ${all} seen`;
  };

  // ---- showing notes ----------------------------------------------------------------

  const label = (note) => {
    const parts = [`n${note.id}`, word(STANCES, note.stance), word(ASKS, note.ask)];
    if (note.blocking) parts.push("Blocking");
    parts.push(STATUS[note.status] || note.status);
    return parts.join(" · ");
  };

  const act = async (note, action, payload, method = "POST") => {
    try {
      const url = action === "edit" ? `/api/notes/${note.id}` : `/api/notes/${note.id}/${action}`;
      await call(method, url, payload);
      await refresh();
    } catch (error) {
      say(`That did not work: ${error.message}`);
    }
  };

  // The forms under a note: reply and decide.
  const inlineForm = (note, what) => {
    const box = el("form", { class: "dm-inline" });
    const id = `N${note.id}${what}`;
    const text = el("textarea", { id, rows: "3", maxlength: "20000" });
    if (what === "reply") {
      box.append(el("label", { for: id, text: "Your answer" }), text,
        el("p", { class: "dm-buttons" }, el("button", { type: "submit", class: "dm-primary", text: "Send the answer" })));
      box.addEventListener("submit", (event) => {
        event.preventDefault();
        if (text.value.trim()) act(note, "reply", { body: text.value.trim() });
      });
    } else {
      if (note.proposal) text.value = note.proposal.summary || "";
      const choice = radios("outcome", OUTCOMES, note.proposal ? note.proposal.outcome : "change");
      choice.querySelector("legend").textContent = "Your decision";
      box.append(choice, el("label", { for: id, text: "What exactly (an agent applies this)" }), text,
        el("p", { class: "dm-buttons" }, el("button", { type: "submit", class: "dm-primary", text: "Decide" })));
      box.addEventListener("submit", (event) => {
        event.preventDefault();
        act(note, "decide", { outcome: box.elements.outcome.value, summary: text.value.trim() });
      });
    }
    return box;
  };

  // One note as a card: its label, quote, point, thread and actions.
  const card = (note) => {
    const placement = note.placement;
    const state = placement ? placement.state : "orphaned";
    const box = el("aside", { class: "dm-note", id: `Note${note.id}`, "data-gen": true, "data-state": state, "data-status": note.status, "aria-label": `Note n${note.id}` });
    box.append(el("p", { class: "dm-note-head", text: label(note) }));
    const where = STATE[state];
    if (where) box.append(el("p", { class: "dm-note-state", text: where }));
    if (note.target.side === "old") box.append(el("p", { class: "dm-note-state", text: "On text the newer version removed or changed" }));
    box.append(el("blockquote", { class: "dm-quote", text: note.target.quote.exact }));
    if (placement && placement.text && state !== "exact") {
      box.append(el("p", { class: "dm-now-label", text: "It now reads:" }), el("blockquote", { class: "dm-quote dm-now", text: placement.text }));
    }
    box.append(el("p", { class: "dm-body", text: note.body }));
    if (note.thread.length) {
      box.append(el("ol", { class: "dm-thread" }, ...note.thread.map((reply) =>
        el("li", { "data-role": reply.role },
          el("strong", { text: reply.role === "agent" ? `Agent (${reply.by})` : reply.by }),
          ` ${reply.body}`))));
    }
    if (note.proposal && !note.decision) {
      box.append(el("p", { class: "dm-proposal" }, el("strong", { text: "Proposed: " }), `${word(OUTCOMES, note.proposal.outcome)}. ${note.proposal.summary}`));
    }
    if (note.decision) {
      box.append(el("p", { class: "dm-decision" }, el("strong", { text: "Decided: " }), `${word(OUTCOMES, note.decision.outcome)}. ${note.decision.summary}`));
    }
    if (note.applied.length) {
      box.append(el("p", { class: "dm-applied" }, el("strong", { text: "Applied in " }), note.applied.map((a) => a.commit.slice(0, 7)).join(", ")));
    }
    if (note.caused && note.caused.length) {
      box.append(el("p", { class: "dm-caused-head" }, el("strong", { text: "Changes it caused on this page:" })),
        el("ul", { class: "dm-caused" }, ...note.caused.map((made) => {
          const target = cardFor(made);
          if (!target) {
            const own = `${location.pathname.replace(/\/-\/.*$/, "")}/-/${made.path}`;
            return el("li", {}, el("a", { href: own, text: `A change in ${made.path}, on its own page` }));
          }
          const kindText = (target.querySelector(".head .kind") || {}).textContent || "A change";
          const where = [...target.querySelectorAll("ol.path li")].map((li) => li.textContent).join(" › ");
          return el("li", {}, el("a", { href: `#${target.id}`, text: where ? `${kindText}, in ${where}` : kindText }));
        })));
    }
    if (!FINAL.has(note.status)) {
      const acts = el("p", { class: "dm-buttons" });
      const open = (what) => {
        const old = box.querySelector("form.dm-inline");
        if (old) old.remove();
        const form = inlineForm(note, what);
        box.append(form);
        form.querySelector("textarea, input").focus();
      };
      acts.append(el("button", { type: "button", text: "Reply", onclick: () => open("reply") }));
      acts.append(el("button", { type: "button", text: "Decide", onclick: () => open("decide") }));
      if (note.status !== "decided") {
        acts.append(el("button", {
          type: "button", text: "Edit",
          onclick: () => openForm(`Edit note n${note.id}`, note.target.quote.exact, note, (fields) =>
            call("PATCH", `/api/notes/${note.id}`, fields)),
        }));
      }
      acts.append(el("button", {
        type: "button", text: "Withdraw",
        onclick: () => {
          if (window.confirm(`Withdraw note n${note.id}? It stays in the notes file, marked withdrawn.`)) act(note, "withdraw", {});
        },
      }));
      box.append(acts);
    }
    return box;
  };

  // The change card a note's applied commit made, when this page shows it
  // and its key still names the same change.
  const cardFor = (made) => {
    const list = [...main.querySelectorAll("ol.changes[data-path]")].find((ol) => ol.dataset.path === made.path);
    const section = list ? list.closest("section.docchg[data-keys]") : null;
    if (!section || section.dataset.keys.split(" ")[made.change] !== made.key) return null;
    return [...list.querySelectorAll("li.change")].find((item) => Number(item.id.split(".").pop()) === made.change) || null;
  };

  // A link to a note's card opens the "Unanchored" tab first when the card
  // sits there, so the page can scroll to it.
  const reveal = (id) => {
    const target = document.getElementById(`Note${id}`);
    const panel = target ? target.closest("[role=tabpanel]") : null;
    if (panel && panel.hidden) {
      const button = document.getElementById(`${panel.id}b`);
      if (button) button.click();
    }
  };

  // "Changed because of note n12" on each card a note's commits made.
  const markCaused = () => {
    const byCard = new Map();
    for (const note of notes) {
      if (note.status === "withdrawn") continue;
      for (const made of note.caused || []) {
        const target = cardFor(made);
        if (!target) continue;
        if (!byCard.has(target)) byCard.set(target, []);
        if (!byCard.get(target).includes(note)) byCard.get(target).push(note);
      }
    }
    for (const [target, causes] of byCard) {
      const line = el("p", { class: "dm-because", "data-gen": true }, "Changed because of ");
      causes.forEach((note, at) => {
        if (at > 0) line.append(at === causes.length - 1 ? " and " : ", ");
        line.append(el("a", { href: `#Note${note.id}`, text: `note n${note.id}`, onclick: () => reveal(note.id) }));
      });
      const head = target.querySelector(".head");
      if (head) head.after(line);
      else target.prepend(line);
      target.classList.add("is-caused");
    }
  };

  // Where a note's card goes on a reading page: after its block.
  const afterBlock = (block) => {
    const scope = body();
    const parts = scope ? scope.querySelectorAll(`[data-k="${block}"]`) : [];
    const last = parts[parts.length - 1];
    if (!last) return null;
    if (last.tagName === "TR") return { after: last.closest("div.table") || last.closest("table") };
    if (last.tagName === "SPAN") return { inside: last.closest("li") || last.parentElement };
    return { after: last };
  };

  // Where a note's card goes on a change page: in the card of its block.
  const inCard = (note) => {
    const placement = note.placement;
    const list = [...main.querySelectorAll("ol.changes[data-path]")].find((ol) => ol.dataset.path === note.target.path);
    if (!list) return null;
    const attribute = placement.side === "old" ? "data-old" : "data-new";
    for (const change of list.querySelectorAll(`li.change[${attribute}="${placement.block}"]`)) {
      const kindOf = change.dataset.kind;
      if (placement.side === "new" && kindOf === "removed") continue;
      if (placement.side === "old" && kindOf === "added") continue;
      return { inside: change };
    }
    const section = list.closest("section.docchg");
    let rest = section.querySelector(".dm-unchanged");
    if (!rest) {
      rest = el("div", { class: "dm-unchanged", "data-gen": true },
        el("p", { class: "dm-unchanged-title", text: "Notes on text this range did not change" }));
      list.before(rest);
    }
    return { inside: rest };
  };

  const clearShown = () => {
    for (const old of main.querySelectorAll(".dm-note, .dm-unchanged, .dm-pin, .dm-because")) old.remove();
    for (const marked of main.querySelectorAll("li.change.is-caused")) marked.classList.remove("is-caused");
    if (window.CSS && CSS.highlights) {
      CSS.highlights.delete("dm-notes");
      CSS.highlights.delete("dm-notes-changed");
    }
  };

  const show = async () => {
    clearShown();
    const placed = notes.filter((note) => note.placement && note.placement.start !== null && note.placement.state !== "orphaned" && note.status !== "withdrawn");
    if (kind === "read" && placed.length) {
      try {
        await positions();
        window.driftmarkAnchor.reset();
      } catch (_) {
        // Without the tables the notes still show under their blocks.
      }
    }
    const ranges = { "dm-notes": [], "dm-notes-changed": [] };
    for (const note of placed) {
      const place = kind === "read" ? afterBlock(note.placement.block) : inCard(note);
      if (!place) continue;
      const shown = card(note);
      if (place.after) place.after.after(shown);
      else place.inside.append(shown);
      if (kind === "read" && window.driftmarkAnchor && tables) {
        const found = window.driftmarkAnchor.fromSource(note.placement.start, note.placement.end);
        if (found) ranges[note.placement.state === "exact" || note.placement.state === "moved" ? "dm-notes" : "dm-notes-changed"].push(found.range);
      }
    }
    if (window.CSS && CSS.highlights && typeof Highlight === "function") {
      for (const [name, list] of Object.entries(ranges)) {
        if (list.length) CSS.highlights.set(name, new Highlight(...list));
      }
    }
    if (kind === "review") markCaused();
    panel();
  };

  // ---- the review list -----------------------------------------------------------------

  let panelBox = null;

  const handOff = async (review, how, round) => {
    try {
      const data = await call("POST", `/api/reviews/${review}/${how}`, { round });
      const count = data.notes.length;
      if (count === 0) {
        say(round === 1 ? "No note is open for discussion in this review." : "No note in this review is decided yet.");
        return;
      }
      if (how === "file") {
        say(`Wrote ${count} ${count === 1 ? "note" : "notes"} to ${data.file}. Tell the agent to read that file.`);
      } else {
        let copied = false;
        try {
          await navigator.clipboard.writeText(data.text);
          copied = true;
        } catch (_) {
          copied = false;
        }
        if (copied) {
          say(`Copied ${count} ${count === 1 ? "note" : "notes"} as a prompt, round ${round}. Paste it into the agent's session.`);
        } else {
          showText(data.text);
        }
      }
      await refresh();
    } catch (error) {
      say(`That did not work: ${error.message}`);
    }
  };

  // When the clipboard is not allowed (a page over plain HTTP from another
  // device), the prompt shows in a box to copy by hand.
  const showText = (text) => {
    if (dialog) dialog.remove();
    const area = el("textarea", { id: "PromptText", rows: "14", readonly: true });
    area.value = text;
    dialog = el("dialog", { class: "dm-dialog", "aria-labelledby": "PromptTitle" },
      el("form", { class: "dm-form", method: "dialog" },
        el("h2", { id: "PromptTitle", text: "The prompt" }),
        el("p", { text: "This browser does not let the page copy. Select the text and copy it." }),
        el("label", { for: "PromptText", class: "vh", text: "The prompt" }), area,
        el("p", { class: "dm-buttons" }, el("button", { type: "submit", class: "dm-primary", text: "Close" }))));
    document.body.append(dialog);
    dialog.showModal();
    area.select();
  };

  const row = (note) => {
    const target = document.getElementById(`Note${note.id}`);
    const quote = note.target.quote.exact.replace(/\s+/g, " ");
    return el("li", { "data-status": note.status },
      target ? el("a", { href: `#Note${note.id}`, text: `n${note.id}` }) : el("strong", { text: `n${note.id}` }),
      ` ${label(note).split(" · ").slice(1).join(" · ")} — `,
      el("q", { text: quote.length > 90 ? `${quote.slice(0, 88)}…` : quote }),
      note.target.path !== main.dataset.path && kind === "review" ? el("span", { class: "dm-row-path", text: ` in ${note.target.path}` }) : null);
  };

  const panel = () => {
    if (panelBox) panelBox.remove();
    panelBox = null;
    say = sayAlone;
    if (notes.length === 0) return;
    // A note on the old side of a change has no place on a reading page,
    // but it is not lost: it stays with the change where it was made.
    const orphaned = notes.filter((note) =>
      note.placement ? note.placement.state === "orphaned" : note.target.side !== "old");
    const inPlace = notes.filter((note) => !orphaned.includes(note));
    const tabs = el("div", { class: "dm-tabs", role: "tablist", "aria-label": "Notes" });
    const panels = [];
    const tab = (name, list, render) => {
      const id = `NotesTab${panels.length}`;
      const button = el("button", { type: "button", role: "tab", id: `${id}b`, "aria-controls": id, "aria-selected": panels.length === 0 ? "true" : "false", text: `${name} (${list.length})` });
      const box = el("div", { role: "tabpanel", id, "aria-labelledby": `${id}b`, hidden: panels.length !== 0 });
      render(box, list);
      button.addEventListener("click", () => {
        for (const other of tabs.querySelectorAll("[role=tab]")) other.setAttribute("aria-selected", other === button ? "true" : "false");
        for (const other of panels) other.hidden = other !== box;
      });
      tabs.append(button);
      panels.push(box);
    };
    tab("On this page", inPlace, (box, list) => {
      const byReview = new Map();
      for (const note of list) {
        if (!byReview.has(note.review)) byReview.set(note.review, []);
        byReview.get(note.review).push(note);
      }
      for (const [review, members] of byReview) {
        const info = reviews.get(review);
        const name = info ? `Review r${review}: ${info.scope}` : `Review r${review}`;
        box.append(el("section", { class: "dm-review-group" },
          el("h3", { text: name }),
          el("p", { class: "dm-buttons" },
            el("button", { type: "button", text: "Copy as prompt", title: "Round 1: the agent discusses the open notes", onclick: () => handOff(review, "prompt", 1) }),
            el("button", { type: "button", text: "Copy decided notes", title: "Round 2: the agent applies the decided notes", onclick: () => handOff(review, "prompt", 2) }),
            el("button", { type: "button", text: "Write review file", title: "Round 1 as a Markdown file next to the notes file", onclick: () => handOff(review, "file", 1) })),
          el("ol", { class: "dm-rows" }, ...members.map(row))));
      }
    });
    tab("Unanchored", orphaned, (box, list) => {
      box.append(el("p", { class: "dm-hint", text: "Notes whose text is no longer in this version. They are kept; each shows where it was made." }));
      if (list.length) box.append(el("div", { class: "dm-orphans" }, ...list.map(card)));
    });
    const status = el("p", { class: "dm-say", role: "status", "aria-live": "polite" });
    say = (text) => {
      status.textContent = text;
    };
    panelBox = el("aside", { class: "dm-review", "aria-labelledby": "NotesHead", "data-gen": true },
      el("h2", { id: "NotesHead", text: `Notes (${notes.length})` }),
      tabs, ...panels, status,
      el("p", { class: "dm-file", text: `Kept in ${file}, which git never commits. Deleting the clone deletes them; a review file is a copy you can keep.` }));
    const before = kind === "read" ? main.querySelector("article.doc") : main.querySelector(".attention") || main.querySelector("section.docchg");
    if (before) before.before(panelBox);
    else main.append(panelBox);
  };

  // ---- start -------------------------------------------------------------------------

  let listening = false;

  const start = (scope) => {
    main = scope && scope.matches && scope.matches("main[data-notes]") ? scope : scope.querySelector("main[data-notes]");
    if (!main) return;
    kind = main.classList.contains("read") ? "read" : "review";
    tables = null;
    notes = [];
    if (kind === "review") activateCards();
    if (!listening) {
      listening = true;
      document.addEventListener("selectionchange", () => {
        clearTimeout(start.timer);
        start.timer = setTimeout(placeFloating, 200);
      });
      window.addEventListener("resize", hideFloating);
    }
    if (Number(main.dataset.notes) > 0) refresh();
  };

  window.driftmarkNotes = { start, refresh };
})();
