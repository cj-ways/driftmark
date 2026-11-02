#!/usr/bin/env node
// Writes and packs the npm packages of one release from its archives:
//   - @cj-ways/driftmark-<os>-<cpu>, one per system, each holding that
//     system's program (about 35 MB, too big to ship all six in one package);
//   - driftmark, which lists those six as optionalDependencies, so npm
//     installs only the one matching the machine, and runs it through a small
//     launcher (npm/driftmark.js). No package has an install script.
// This is the layout of esbuild and Biome.
//   node scripts/npm-packages.mjs <archives-dir> <out-dir> [<target>...]
// The archives are release.yml's driftmark-<target>.tar.gz (.zip on Windows).
// With targets named, only their system packages are packed (a local test);
// the driftmark package still lists all six. The version, description,
// keywords and readme are the driftmark crate's (crates/driftmark/Cargo.toml).
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import process from "node:process";

const [archivesArg, outArg, ...only] = process.argv.slice(2);
if (!archivesArg || !outArg) {
  console.error("usage: node scripts/npm-packages.mjs <archives-dir> <out-dir> [<target>...]");
  process.exit(2);
}
const root = resolve(import.meta.dirname, "..");
const archives = resolve(archivesArg);
const out = resolve(outArg);

// Rust target, Node's process.platform and process.arch for it, and the
// words the system package's description uses.
const systems = [
  { target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", name: "macOS on arm64 (Apple silicon)" },
  { target: "x86_64-apple-darwin", os: "darwin", cpu: "x64", name: "macOS on x64 (Intel)" },
  { target: "aarch64-unknown-linux-gnu", os: "linux", cpu: "arm64", name: "Linux on arm64, glibc 2.28 or newer" },
  { target: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64", name: "Linux on x64, glibc 2.28 or newer" },
  { target: "aarch64-pc-windows-msvc", os: "win32", cpu: "arm64", name: "Windows on arm64" },
  { target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", name: "Windows on x64" },
];
const unknown = only.filter((t) => !systems.some((s) => s.target === t));
if (unknown.length > 0) {
  console.error(`npm-packages: unknown target ${unknown.join(", ")}`);
  process.exit(2);
}
const packed = only.length > 0 ? systems.filter((s) => only.includes(s.target)) : systems;

const metadata = JSON.parse(
  execFileSync("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"], {
    cwd: root,
    encoding: "utf8",
  }),
);
const crate = metadata.packages.find((p) => p.name === "driftmark");
const { version } = crate;
const repo = crate.repository;
const readme = crate.readme ? resolve(dirname(crate.manifest_path), crate.readme) : null;
// npm adds a licence file by itself only when it is named LICENSE or LICENCE.
const licences = ["LICENSE-MIT", "LICENSE-APACHE"];
const notices = "THIRD-PARTY-NOTICES";
const common = {
  version,
  license: crate.license,
  repository: { type: "git", url: `git+${repo}.git` },
  homepage: `${repo}#readme`,
  bugs: `${repo}/issues`,
};
const packageName = (s) => `@cj-ways/driftmark-${s.os}-${s.cpu}`;

mkdirSync(out, { recursive: true });
const work = mkdtempSync(join(tmpdir(), "driftmark-npm-"));
const results = [];
function pack(dir) {
  const args = ["pack", dir, "--pack-destination", out, "--ignore-scripts", "--json"];
  const [p] = JSON.parse(execFileSync("npm", args, { encoding: "utf8" }));
  results.push(`${p.filename}: ${p.size} bytes, ${p.unpackedSize} unpacked, ${p.entryCount} files`);
}

// Takes the program, the licences and the notices out of a release archive.
function unpack(s, dir) {
  const exe = s.os === "win32" ? "driftmark.exe" : "driftmark";
  const files = [exe, ...licences, notices];
  if (s.os === "win32") {
    execFileSync("unzip", ["-q", join(archives, `driftmark-${s.target}.zip`), ...files, "-d", dir]);
  } else {
    execFileSync("tar", ["-xzf", join(archives, `driftmark-${s.target}.tar.gz`), "-C", dir, ...files]);
  }
  mkdirSync(join(dir, "bin"));
  copyFileSync(join(dir, exe), join(dir, "bin", exe));
  rmSync(join(dir, exe));
  if (s.os !== "win32") chmodSync(join(dir, "bin", exe), 0o755);
}

try {
  for (const s of packed) {
    const dir = join(work, `${s.os}-${s.cpu}`);
    mkdirSync(dir);
    unpack(s, dir);
    const manifest = {
      name: packageName(s),
      ...common,
      description: `The driftmark program for ${s.name}. Install the driftmark package, which picks this one.`,
      os: [s.os],
      cpu: [s.cpu],
      ...(s.os === "linux" ? { libc: ["glibc"] } : {}),
      // Yarn's Plug'n'Play must unpack the program to run it.
      preferUnplugged: true,
      files: ["bin", ...licences, notices],
    };
    writeFileSync(join(dir, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    writeFileSync(
      join(dir, "README.md"),
      `# ${manifest.name}\n\nThe [driftmark](${repo}) program for ${s.name}.\n\n` +
        "Install `driftmark` instead: `npm install -g driftmark` or `npx driftmark`. " +
        "npm then installs this package only on the system it is for.\n",
    );
    pack(dir);
  }

  const dir = join(work, "driftmark");
  mkdirSync(join(dir, "bin"), { recursive: true });
  copyFileSync(join(root, "npm", "driftmark.js"), join(dir, "bin", "driftmark.js"));
  chmodSync(join(dir, "bin", "driftmark.js"), 0o755);
  for (const f of licences) copyFileSync(join(root, f), join(dir, f));
  // The notices of the first archive; every archive carries the same file.
  const first = join(work, `${packed[0].os}-${packed[0].cpu}`, notices);
  copyFileSync(first, join(dir, notices));
  if (readme) copyFileSync(readme, join(dir, "README.md"));
  const manifest = {
    name: "driftmark",
    ...common,
    description: crate.description,
    keywords: crate.keywords,
    bin: { driftmark: "bin/driftmark.js" },
    files: ["bin", ...licences, notices],
    engines: { node: ">=18" },
    optionalDependencies: Object.fromEntries(systems.map((s) => [packageName(s), version])),
  };
  writeFileSync(join(dir, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  pack(dir);
} finally {
  rmSync(work, { recursive: true, force: true });
}
for (const r of results) console.log(r);
