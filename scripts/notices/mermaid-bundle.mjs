#!/usr/bin/env node
// Writes the licences of the libraries inside mermaid's browser bundle, which
// driftmark carries as crates/driftmark/assets/vendor/mermaid/mermaid.min.js.gz.
// mermaid's own licence is MIT, but its bundle also holds d3 (ISC), elkjs
// (EPL-2.0), chevrotain (Apache-2.0), DOMPurify (MPL-2.0 or Apache-2.0) and
// others, so THIRD-PARTY-NOTICES needs their texts too.
//
// Run it by hand when the bundled mermaid.js changes; the result is committed.
//   node scripts/notices/mermaid-bundle.mjs 12.1.0 > crates/driftmark/assets/vendor/mermaid/BUNDLED-LICENSES.txt
//
// The package list is every runtime dependency of mermaid at that release,
// followed through mermaid's own lock file, without type-only packages. It
// can name a few packages the browser bundle leaves out; it never misses one
// the bundle holds. The licence texts come from each package at the exact
// version the lock file names, through jsDelivr's copy of the npm registry.
import process from "node:process";

const version = process.argv[2];
if (!version) {
  console.error("usage: node scripts/notices/mermaid-bundle.mjs <mermaid version>");
  process.exit(2);
}
const tag = `mermaid@${version}`;

async function get(url, as = "text") {
  for (let attempt = 1; ; attempt++) {
    const res = await fetch(url);
    if (res.ok) return as === "json" ? res.json() : res.text();
    if (res.status === 404 || attempt === 3) throw new Error(`${url}: HTTP ${res.status}`);
    await new Promise((r) => setTimeout(r, 1000 * attempt));
  }
}

// pnpm-lock.yaml (lockfile version 9) is read line by line: only the
// `importers` and `snapshots` maps are needed, and each is two levels deep.
function readLock(text) {
  const importers = new Map();
  const snapshots = new Map();
  let section = "";
  let entry = null;
  let group = "";
  let pending = null;
  const unquote = (s) => s.trim().replace(/^'(.*)'$/, "$1");
  for (const line of text.split("\n")) {
    if (/^\S/.test(line)) {
      section = line.replace(/:$/, "");
      continue;
    }
    if (section !== "importers" && section !== "snapshots") continue;
    let m;
    if ((m = line.match(/^ {2}(\S.*):$/))) {
      entry = { dependencies: new Map(), devDependencies: new Map(), optionalDependencies: new Map() };
      (section === "importers" ? importers : snapshots).set(unquote(m[1]), entry);
    } else if ((m = line.match(/^ {4}(\w+):$/))) {
      group = m[1];
    } else if (section === "snapshots" && (m = line.match(/^ {6}(\S.*?): (\S.*)$/))) {
      entry[group]?.set(unquote(m[1]), unquote(m[2]));
    } else if (section === "importers" && (m = line.match(/^ {6}(\S.*):$/))) {
      pending = unquote(m[1]);
    } else if (section === "importers" && (m = line.match(/^ {8}version: (\S.*)$/))) {
      entry[group]?.set(pending, unquote(m[1]));
    }
  }
  if (importers.size === 0 || snapshots.size === 0) throw new Error("pnpm-lock.yaml: no importers or snapshots");
  return { importers, snapshots };
}

const lockUrl = `https://raw.githubusercontent.com/mermaid-js/mermaid/${encodeURIComponent(tag)}/pnpm-lock.yaml`;
const { importers, snapshots } = readLock(await get(lockUrl));

// mermaid's dependencies, plus what its parser package compiles in.
const roots = [];
for (const [name, v] of importers.get("packages/mermaid").dependencies) {
  if (!name.startsWith("@types/") && !v.startsWith("link:")) roots.push([name, v]);
}
const parser = importers.get("packages/parser");
for (const name of ["chevrotain", "langium", "@chevrotain/types"]) {
  const v = parser.dependencies.get(name) ?? parser.devDependencies.get(name);
  if (!v) throw new Error(`packages/parser: no ${name}`);
  roots.push([name, v]);
}

const found = new Map();
const stack = [...roots];
while (stack.length) {
  const [name, v] = stack.pop();
  const key = `${name}@${v}`;
  if (found.has(key)) continue;
  found.set(key, [name, v.split("(")[0]]);
  const snap = snapshots.get(key);
  for (const deps of [snap?.dependencies, snap?.optionalDependencies]) {
    for (const [d, dv] of deps ?? []) if (!d.startsWith("@types/")) stack.push([d, dv]);
  }
}
const packages = [...new Map([...found.values()].map(([n, v]) => [`${n}@${v}`, [n, v]])).values()].sort(
  ([a, av], [b, bv]) => (a === b ? av.localeCompare(bv) : a.localeCompare(b)),
);

const licenceFile = /^(licen[cs]e|copying)([.-][\w.-]*)?$/i;
const entries = [];
for (const [name, v] of packages) {
  const meta = await get(`https://registry.npmjs.org/${name.replace("/", "%2f")}/${v}`, "json");
  const id = typeof meta.license === "string" ? meta.license : (meta.license?.type ?? "see its licence file");
  const listing = await get(`https://data.jsdelivr.com/v1/packages/npm/${name}@${v}`, "json");
  const files = listing.files.filter((f) => f.type === "file" && licenceFile.test(f.name)).map((f) => f.name);
  if (files.length === 0) throw new Error(`${name}@${v}: no licence file in the package`);
  const texts = [];
  for (const f of files.sort()) {
    texts.push((await get(`https://cdn.jsdelivr.net/npm/${name}@${v}/${f}`)).replace(/\r\n/g, "\n").trim());
  }
  entries.push({ name, version: v, id, text: texts.join("\n\n") });
}

// One block per distinct licence text, naming every package that carries it.
const byText = new Map();
for (const e of entries) {
  const block = byText.get(e.text) ?? { ids: new Set(), names: [], text: e.text };
  block.ids.add(e.id);
  block.names.push(`${e.name} ${e.version}`);
  byText.set(e.text, block);
}

const out = [];
out.push(`Libraries inside mermaid.js ${version}`);
out.push("");
out.push(`driftmark carries mermaid's browser bundle (mermaid.min.js, mermaid ${version}) unchanged.`);
out.push("The bundle holds the libraries below. Each keeps its own licence, whose text follows.");
out.push(`The list follows mermaid's lock file at the tag ${tag} and may name a few packages the`);
out.push("bundle leaves out. Written by scripts/notices/mermaid-bundle.mjs; do not edit by hand.");
out.push("");
out.push("Packages and their licences:");
for (const e of entries) out.push(`  ${e.name} ${e.version}: ${e.id}`);
for (const b of byText.values()) {
  out.push("");
  out.push("-".repeat(80));
  out.push(`${[...b.ids].join(", ")}, used by: ${b.names.join(", ")}`);
  out.push("");
  out.push(b.text);
}
process.stdout.write(`${out.join("\n")}\n`);
console.error(`${entries.length} packages, ${byText.size} distinct licence texts`);
