#!/usr/bin/env node
// npm's `driftmark` command. The program itself comes in one package per
// system, @cj-ways/driftmark-<os>-<cpu>, and npm installs only the one whose
// `os`, `cpu` and `libc` match this machine (the optionalDependencies of
// this package). This file finds it and runs it. There is no install script.
"use strict";
const { spawnSync } = require("child_process");
const path = require("path");

const system = process.platform + "-" + process.arch;
const pkg = "@cj-ways/driftmark-" + system;
const exe = process.platform === "win32" ? "driftmark.exe" : "driftmark";

function fail(message) {
  console.error("driftmark: " + message);
  process.exit(1);
}

// glibc reports its version here; musl and other C libraries do not.
function isMusl() {
  if (process.platform !== "linux") return false;
  try {
    return !process.report.getReport().header.glibcVersionRuntime;
  } catch {
    return false;
  }
}

const builds = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "win32-arm64", "win32-x64"];
let dir;
try {
  dir = path.dirname(require.resolve(pkg + "/package.json"));
} catch {
  if (!builds.includes(system)) {
    fail(
      "there is no build for " + system + ". There are builds for macOS, Linux (glibc) and " +
        "Windows, on x64 and arm64. To build from source: cargo install driftmark",
    );
  }
  if (isMusl()) {
    fail(
      "the Linux build needs glibc 2.28 or newer, and this system uses musl (Alpine?). " +
        "Use a glibc system or container, or build from source: cargo install driftmark",
    );
  }
  fail(
    "the package " + pkg + " is missing. npm installs it with driftmark unless optional " +
      "dependencies are turned off (--omit=optional, --no-optional). Install driftmark again " +
      "with them on.",
  );
}

const run = spawnSync(path.join(dir, "bin", exe), process.argv.slice(2), { stdio: "inherit" });
if (run.error) fail(run.error.message);
// A program stopped by a signal (Ctrl+C on `driftmark serve`) stops this
// launcher the same way, so the shell sees what happened.
if (run.signal) process.kill(process.pid, run.signal);
process.exit(run.status === null ? 1 : run.status);
