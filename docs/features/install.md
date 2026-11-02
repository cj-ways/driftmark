---
status: target
date: 2026-11-02
---

# Install

> Short version: driftmark is one program, the same on every install path. Install it with npm, Python's tools, Homebrew or cargo, or download it from the GitHub release. Builds exist for macOS, Linux and Windows, each on x64 and arm64. Every release file is listed in `SHA256SUMS` and carries a signed record of the workflow that built it, so a download can be checked before it runs.

## Pick a path

Each path puts the command `driftmark` on your `PATH`. The agent hook and the MCP connector start it by that name.

| You have | Install | Try once without installing | Updates with |
|---|---|---|---|
| Node.js 18 or newer | `npm install -g driftmark` | `npx driftmark serve` | `npm install -g driftmark@latest` |
| Python with uv | `uv tool install driftmark` | `uvx driftmark serve` | `uv tool upgrade driftmark` |
| Python with pipx | `pipx install driftmark` | `pipx run driftmark serve` | `pipx upgrade driftmark` |
| Homebrew (macOS or Linux) | `brew install cj-ways/driftmark/driftmark` | | `brew upgrade driftmark` |
| Rust 1.95 or newer | `cargo install driftmark --locked` | | the same command again |
| cargo-binstall | `cargo binstall driftmark` | | the same command again |
| Nothing of the above | a release archive, below | | a newer archive |

`driftmark --version` shows which version runs, for example `driftmark 0.1.0`.

## Systems

| System | Release file | npm package | Python wheel tag |
|---|---|---|---|
| macOS, Apple silicon | `driftmark-aarch64-apple-darwin.tar.gz` | `@cj-ways/driftmark-darwin-arm64` | `macosx_*_arm64` |
| macOS, Intel | `driftmark-x86_64-apple-darwin.tar.gz` | `@cj-ways/driftmark-darwin-x64` | `macosx_*_x86_64` |
| Linux, x64 | `driftmark-x86_64-unknown-linux-gnu.tar.gz` | `@cj-ways/driftmark-linux-x64` | `manylinux_2_28_x86_64` |
| Linux, arm64 | `driftmark-aarch64-unknown-linux-gnu.tar.gz` | `@cj-ways/driftmark-linux-arm64` | `manylinux_2_28_aarch64` |
| Windows, x64 | `driftmark-x86_64-pc-windows-msvc.zip` | `@cj-ways/driftmark-win32-x64` | `win_amd64` |
| Windows, arm64 | `driftmark-aarch64-pc-windows-msvc.zip` | `@cj-ways/driftmark-win32-arm64` | `win_arm64` |

- **Linux needs glibc 2.28 or newer.** That is every mainstream distribution from 2018 on: Debian 10, Ubuntu 18.10, RHEL 8 and their successors. The program is linked against glibc 2.28 on purpose, and each release starts it in an AlmaLinux 8 container, which has exactly that version.
- **Alpine and other musl systems have no build** (decision 0011). Use a glibc system or container, or build from source with `cargo install`. A build on musl works but counts changes 15 to 17 times slower, because of musl's memory allocator.
- **The macOS versions** are the oldest the Rust compiler supports for each processor. The Python wheel's tag names them.

The Linux x64 download is about 19 MB, and the program is about 40 MB once unpacked. The release page shows the size of each file.

## What each path needs

### npm

- Node.js 18 or newer, with npm. pnpm, Yarn and Bun read the same package fields, but the release is tested with npm only.
- The `driftmark` package holds a small launcher, `bin/driftmark.js`. The program itself is in a second package, one per system: `@cj-ways/driftmark-<os>-<cpu>`. `driftmark` lists all six as optional dependencies, and the package manager installs only the one whose `os`, `cpu` and `libc` match the machine. This is how esbuild and Biome ship.
- Nothing runs at install time: no package has an install script, so `--ignore-scripts` changes nothing.
- Optional dependencies must stay on. With `--omit=optional` (or `--no-optional`) the program package is left out, and `driftmark` says so and stops.
- On a system without a build, `driftmark` names the systems that have one and suggests `cargo install`.

### Python (pip, pipx, uv)

- Any Python 3. The wheel holds only the program, no Python code, so its tag is `py3-none-<system>`.
- `pip install driftmark` puts `driftmark` in the environment's `bin` (`Scripts` on Windows). On Linux, pip 20.3 or newer is needed to read the `manylinux_2_28` tag.
- `uv tool install`, `uvx`, `pipx install` and `pipx run` keep it in its own environment, which is the better choice for a command.

### Homebrew

- Homebrew on macOS or Linux, and the tap `cj-ways/homebrew-driftmark`, which `brew install cj-ways/driftmark/driftmark` adds by itself.
- The formula downloads the release archive for the machine and checks it against the checksum written in the formula.
- `scripts/update-tap.sh <version>` writes the formula from a release's `SHA256SUMS`, after each release.

### cargo

- `cargo install driftmark --locked` builds from source. It needs Rust 1.95 or newer and a C compiler, because SQLite is built from its C source. The build takes several minutes. `--locked` uses the exact library versions the release was tested with.
- `cargo binstall driftmark` downloads the release archive instead of building, if cargo-binstall is installed.
- The library crate `driftmark-core` is published too. It is for programs that build on driftmark's engine; it has no command.

### A release archive

1. Open the release on GitHub: `https://github.com/cj-ways/driftmark/releases`.
2. Download the file for your system from the table above, and `SHA256SUMS`.
3. Check the file (next section).
4. Unpack it: `tar -xzf driftmark-<target>.tar.gz`, or open the `.zip` on Windows.
5. Move `driftmark` (`driftmark.exe` on Windows) to a folder on your `PATH`, such as `~/.local/bin`.

On macOS, a file downloaded with a browser is marked as coming from the internet. macOS may then refuse to start the program, because Apple has not notarized it. `xattr -d com.apple.quarantine driftmark` removes the mark. The other install paths download without a browser, so they do not set it.

## What every package carries

- The program, built by `.github/workflows/release.yml` from the tagged commit.
- `LICENSE-MIT` and `LICENSE-APACHE`: driftmark is MIT OR Apache-2.0, at your choice.
- `THIRD-PARTY-NOTICES`: the licence of everything inside the program that others wrote. That is every Rust crate, Prism, mermaid.js with the libraries in its bundle, the Liberation Sans font and SQLite. `scripts/notices/third-party-notices.mjs` writes it for each release with cargo-about.

Where to find it:

| Path | `THIRD-PARTY-NOTICES` is |
|---|---|
| Release archive | beside the program |
| npm | in both packages' folders under `node_modules` |
| Python wheel | in `driftmark-<version>.dist-info/licenses/` |
| Homebrew | in `$(brew --prefix driftmark)/share/doc/driftmark/` |
| crates.io | not in the crate; the crate holds each built-in file's licence beside it, under `assets/` |

## Check a download

### Against SHA256SUMS

`SHA256SUMS` has one line per release file: the file's SHA-256 checksum, two spaces, its name. A file is unchanged when its checksum equals its line.

On Linux:

```sh
sha256sum --ignore-missing -c SHA256SUMS
```

On macOS:

```sh
grep ' driftmark-aarch64-apple-darwin.tar.gz$' SHA256SUMS | shasum -a 256 -c -
```

On Windows, in PowerShell, compare the two values by eye:

```powershell
(Get-FileHash driftmark-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
Select-String driftmark-x86_64-pc-windows-msvc.zip SHA256SUMS
```

`OK` on Linux and macOS means the file matches. A match proves the file is the one listed; the attestation proves who listed it.

### Against the attestation

Each release file has a build-provenance attestation: a record, signed through GitHub, of which repository, workflow and commit built it. With the GitHub CLI:

```sh
gh attestation verify driftmark-x86_64-unknown-linux-gnu.tar.gz -R cj-ways/driftmark
```

It prints `Verification succeeded!` and the workflow that built the file, `.github/workflows/release.yml`. A file built anywhere else, or changed after the build, fails.

The registries keep their own records:

- PyPI shows each wheel's provenance on the package page, under "Verified details".
- npm records where each package was built; `npm audit signatures` checks the installed packages.
- crates.io records that each crate was published by the release workflow, through trusted publishing.

## How a release is made

This part is for the maintainer and the agents.

1. The workspace version in `Cargo.toml` is set to the new version, for example `0.1.0`, and merged to `main`.
2. A tag `v0.1.0` on that commit is pushed. The workflow `release.yml` starts.
3. It writes `THIRD-PARTY-NOTICES`, then builds the six systems: macOS arm64 on `macos-15`, macOS Intel on `macos-15-intel`, Linux on `ubuntu-24.04` and `ubuntu-24.04-arm` (linked against glibc 2.28 with zig), Windows on `windows-2025`. Each build makes the release archive and the Python wheel. Every build whose runner can start the program checks that `driftmark --version` prints the version; the Linux builds also start it in an AlmaLinux 8 container, on glibc 2.28.
4. It checks that the tag equals the version and that the tagged commit is on `main`. It dry-runs `cargo publish` for both crates, checks that the six wheels are there and packs the seven npm packages.
5. It writes `SHA256SUMS`, signs the attestation, uploads everything to a draft release and then publishes the release.
6. It publishes to crates.io (`driftmark-core`, then `driftmark`), PyPI and npm (the six system packages, then `driftmark`). Each goes through its GitHub environment (`crates-io`, `pypi`, `npm`), which accepts only `v*` tags, and through trusted publishing: no token is stored anywhere. A job that fails can be re-run; it skips what is already published.
7. The maintainer runs `scripts/update-tap.sh 0.1.0` to update the Homebrew formula.

Started by hand (`gh workflow run release.yml --ref main`), the workflow does steps 3 and 4 only. It builds and packs every file and publishes nothing. Its files can be downloaded from the run's page.
