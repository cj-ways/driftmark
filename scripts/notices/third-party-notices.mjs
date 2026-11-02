#!/usr/bin/env node
// Writes THIRD-PARTY-NOTICES: the licence of everything built into the
// driftmark program that driftmark did not write. Every release archive,
// wheel and npm package carries the file (.github/workflows/release.yml).
//   node scripts/notices/third-party-notices.mjs <out-file>
// Needs cargo-about 0.9 on PATH (`cargo about`), and the network the first
// time, to read crates that were published without their licence file.
//
// Part 1 comes from cargo-about: every Rust crate in the program on the six
// release systems, grouped by licence text. Part 2 is the files under
// crates/driftmark/assets/ that come from elsewhere: listed below by hand,
// because cargo-about sees only crates. A new folder under assets/vendor/
// stops this script until it is listed here.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import process from "node:process";

const out = process.argv[2];
if (!out) {
  console.error("usage: node scripts/notices/third-party-notices.mjs <out-file>");
  process.exit(2);
}
const root = resolve(import.meta.dirname, "..", "..");
const assets = join(root, "crates", "driftmark", "assets");
const read = (path) => readFileSync(join(assets, path), "utf8").replace(/\r\n/g, "\n").trim();

// What the program carries besides crates. Update the version here when a
// vendored file is replaced.
const bundled = [
  {
    folder: "vendor/prism",
    title: "Prism 1.30.0",
    use: "colours code on the pages and in exported files",
    licence: "MIT",
    source: "https://github.com/PrismJS/prism",
    texts: ["vendor/prism/LICENSE"],
  },
  {
    folder: "vendor/mermaid",
    title: "mermaid.js 12.1.0",
    use: "draws a diagram in the browser when the program could not draw it",
    licence: "MIT, and the licences of the libraries inside its bundle, listed after it",
    source:
      "https://github.com/mermaid-js/mermaid; elkjs, the one EPL-2.0 library in the bundle, " +
      "at https://github.com/kieler/elkjs and as the npm package elkjs",
    texts: ["vendor/mermaid/LICENSE", "vendor/mermaid/BUNDLED-LICENSES.txt"],
  },
  {
    folder: "fonts",
    title: "Liberation Sans 2.1.5",
    use: "the font for diagram text, used to measure it and shown on the pages",
    licence: "SIL Open Font License 1.1 (OFL-1.1)",
    source: "https://github.com/liberationfonts/liberation-fonts",
    texts: ["fonts/LICENSE-Liberation.txt"],
  },
];

const vendored = readdirSync(join(assets, "vendor")).map((d) => `vendor/${d}`);
const unlisted = vendored.filter((f) => !bundled.some((b) => b.folder === f));
if (unlisted.length > 0) {
  console.error(`third-party-notices: list ${unlisted.join(", ")} in this script's \`bundled\``);
  process.exit(1);
}

const json = execFileSync(
  "cargo",
  [
    "about",
    "generate",
    "--locked",
    "--fail",
    "--format",
    "json",
    "--manifest-path",
    join(root, "crates", "driftmark", "Cargo.toml"),
    "--config",
    join(root, "scripts", "notices", "about.toml"),
  ],
  { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
);
const about = JSON.parse(json);

// driftmark's own crates have no registry source; they are not third-party.
// A crate under two licences at once (`MIT AND BSD-3-Clause`) counts under both.
const crates = new Map();
const perLicence = new Map();
const blocks = [];
for (const licence of about.licenses) {
  const users = new Map();
  for (const { crate } of licence.used_by) {
    if (crate.source === null) continue;
    users.set(crate.id, crate);
    crates.set(crate.id, crate);
    perLicence.set(licence.id, (perLicence.get(licence.id) ?? new Set()).add(crate.id));
  }
  if (users.size > 0) blocks.push({ name: licence.name, id: licence.id, users: [...users.values()], text: licence.text });
}
const order = about.overview.map((o) => o.id);
blocks.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
const sqlite = [...crates.values()].find((c) => c.name === "libsqlite3-sys");

const rule = "=".repeat(80);
const thin = "-".repeat(80);
const lines = [];
const add = (...l) => lines.push(...l);
add(
  "THIRD-PARTY NOTICES",
  "",
  "driftmark is licensed under MIT OR Apache-2.0, at your choice (LICENSE-MIT and",
  "LICENSE-APACHE). The driftmark program also contains software that others",
  "wrote. Each part keeps its own licence. This file names every part and gives",
  "the full text of each licence.",
  "",
  `Part 1 lists the ${crates.size} Rust crates built into the program, grouped by licence.`,
  "Part 2 lists the files built into the program that are not Rust crates: a code",
  "highlighter, a diagram library with the libraries inside it, a font, and SQLite.",
  "",
  "Everything is used unchanged. The crates under the Mozilla Public License 2.0",
  "and the Eclipse Public License 2.0 have their source code on crates.io, at",
  "https://crates.io/crates/<name>/<version>, and at the repository named beside",
  "them. Part 2 says where the source of each file is.",
  "",
  rule,
  "PART 1: RUST CRATES",
  rule,
  "",
  "The Rust standard library is built into every Rust program; it is MIT OR",
  "Apache-2.0, like driftmark, and both texts are below.",
  "",
  "Crates per licence (a crate under two licences at once counts twice):",
  ...order.filter((id) => perLicence.has(id)).map((id) => `  ${id}: ${perLicence.get(id).size}`),
);
for (const b of blocks) {
  add("", thin, `${b.name} (${b.id})`, "", "Used by:");
  for (const c of b.users) add(`  ${c.name} ${c.version}${c.repository ? ` - ${c.repository}` : ""}`);
  add("", b.text.replace(/\r\n/g, "\n").trim());
}
add("", rule, "PART 2: FILES BUILT INTO THE PROGRAM THAT ARE NOT RUST CRATES", rule);
for (const b of bundled) {
  add("", thin, b.title, "", `What it does: ${b.use}.`, `Licence: ${b.licence}.`, `Source: ${b.source}.`);
  for (const t of b.texts) add("", read(t));
}
add(
  "",
  thin,
  `SQLite${sqlite ? `, from the copy inside the crate libsqlite3-sys ${sqlite.version}` : ""}`,
  "",
  "What it does: stores the notes file.",
  "Licence: public domain. The authors disclaim copyright; no licence text is required.",
  "Source: https://sqlite.org.",
);
writeFileSync(out, `${lines.join("\n")}\n`);
console.log(`${out}: ${crates.size} crates, ${blocks.length} licence texts, ${bundled.length + 1} other parts`);
