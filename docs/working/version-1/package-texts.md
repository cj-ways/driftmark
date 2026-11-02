---
status: active
date: 2026-11-02
---

# Package texts

> Short version: the texts each registry shows for driftmark, and where each one comes from. The `driftmark` crate's `Cargo.toml` is the one source of the description, the keywords and the README for crates.io, npm and PyPI, so the three registries cannot drift apart. The release scripts write the few texts that only one registry has. Two additions for PyPI are still open, at the end.

## The sentence

Every registry describes the program with the `description` of the `driftmark` crate:

```text
See what your AI agents changed in your Markdown docs, whether it matters, and answer back with notes
```

It has no final full stop, because each registry shows it as a line on its own.

## Where each text comes from

| Registry | Description, keywords and README | Read by |
|---|---|---|
| crates.io, `driftmark` | `crates/driftmark/Cargo.toml`, with the keywords and categories of the workspace in the root `Cargo.toml` | cargo |
| crates.io, `driftmark-core` | `crates/driftmark-core/Cargo.toml`, with its own keywords, categories and README | cargo |
| npm, `driftmark` | The `driftmark` crate's | the release lane's `scripts/npm-packages.mjs`, through `cargo metadata` |
| PyPI, `driftmark` | The `driftmark` crate's | maturin, because `pyproject.toml` lists them as `dynamic` |

The values, as the `Cargo.toml` files set them:

| Field | `driftmark` | `driftmark-core` |
|---|---|---|
| `description` | The sentence above | `The engine of driftmark: Markdown blocks, ranked change detection, note anchoring, search and lint rules, with no file, git or network access` |
| `keywords` | `markdown`, `diff`, `documentation`, `mcp`, `ai-agents` | `markdown`, `diff`, `lint`, `annotation`, `documentation` |
| `categories` | `command-line-utilities`, `development-tools`, `text-processing` | `text-processing`, `development-tools` |
| `readme` | `README.md` at the repository root | `crates/driftmark-core/README.md` |

crates.io takes at most five keywords and five categories, so npm and PyPI get the same five keywords.

## Texts only one registry has

These are the release lane's, as its scripts write them, and this lane agrees with them:

- **Each npm package for one system**, `@cj-ways/driftmark-<os>-<cpu>`: the description "The driftmark program for `<system>`. Install the driftmark package, which picks this one.", and a short README that says the same and how to install `driftmark`.
- **The Homebrew formula's `desc`**: "Review what AI agents changed in Markdown docs, ranked by meaning". Homebrew allows at most 80 characters, no leading article and no final full stop; this has 65 characters.

## Two additions for PyPI

The `pyproject.toml` of the release lane has the licence as an SPDX expression and eight classifiers. This lane asks for two additions:

1. **Four more classifiers**, so that PyPI's filters find driftmark:

   ```text
   Development Status :: 3 - Alpha
   Environment :: Web Environment
   Intended Audience :: Developers
   Programming Language :: Rust
   ```

   `Programming Language :: Rust` is there already; the other three are new. "3 - Alpha" says that 0.1.0 is the first release and has not yet run in other people's repositories. The maintainer may choose "4 - Beta" instead.

2. **The project's addresses**, beside `Repository`, so that PyPI's sidebar links to them:

   ```toml
   [project.urls]
   Homepage = "https://github.com/cj-ways/driftmark"
   Documentation = "https://github.com/cj-ways/driftmark/blob/main/docs/README.md"
   Changelog = "https://github.com/cj-ways/driftmark/blob/main/CHANGELOG.md"
   Issues = "https://github.com/cj-ways/driftmark/issues"
   Repository = "https://github.com/cj-ways/driftmark"
   ```

## Why the README works on every registry

- Its images are full addresses of the form `https://github.com/cj-ways/driftmark/raw/main/docs/images/<name>.png`. PyPI never rewrites a relative address, so a relative image would not show there. GitHub's renderer leaves addresses on `github.com` as they are, so a viewer's browser asks GitHub for them with its own session, which a private repository needs.
- Its links to other documents are full addresses of the form `https://github.com/cj-ways/driftmark/blob/main/<path>`, so they work from crates.io, npm and PyPI.
- Every full address leads to `main`. A registry page of an older release therefore shows today's screenshots.
