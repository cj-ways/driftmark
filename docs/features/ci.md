---
status: target
date: 2026-11-02
---

# Merge-request review in CI

> Short version: on every GitHub pull request or GitLab merge request, one CI job renders the documentation changes as driftmark sees them, attaches them as one HTML file, and posts one short comment with the ranked summary and a link. Every later push updates that same comment. The same job checks the changed Markdown files with `driftmark lint`, in a form the host shows next to the lines. GitHub gets a composite action in this repository; GitLab gets a template to `include:`. GitLab shows no rendered Markdown diff at all ([issue 17000](https://gitlab.com/gitlab-org/gitlab/-/issues/17000)), and GitHub shows one file at a time.

## What a reviewer gets

1. **One comment** on the pull request or merge request, written by `driftmark diff --format markdown-summary` (see "The comment"):
   - how many documents and changes, by level;
   - the ten highest-ranked changes, one line each: level, what happened, document, headings and the changed text;
   - a link to the rendered file.
2. **One HTML file** with every change and the full text of each changed document, written by `driftmark export changes` (`docs/features/commands/export.md`). It opens offline in any browser.
3. **The writing check's findings** on the changed files: annotations beside the lines on GitHub, a Code Quality report on GitLab (`docs/features/lint.md`, "Output for CI").

A made-up comment for a branch that edits a decision record and a guide:

```text
<!-- driftmark:changes -->
### Documentation changes

**2 documents changed, 3 changes**: 2 decision, 1 wording.

Highest ranked first:

1. **decision**, edited · `docs/decisions/0002-cache.md`: `status: accepted`
2. **decision**, edited · `docs/decisions/0002-cache.md` › `Cache rendered pages › Decision`: `Keep each rendered page in the cache for 90 minutes.`
3. **wording**, edited · `docs/guide.md` › `Guide`: `Read the whole setup first.`

**[Every change, rendered](<https://github.com/example/lanterns/actions/runs/7/artifacts/9>)**: one HTML file that opens offline in any browser, with each changed document in full.

<sub>driftmark 0.1.0 compared `3828db3aa184` with `2cc1179a6f87`, only in `docs`.</sub>
```

## GitHub

### A ready workflow

Save this as `.github/workflows/docs-review.yml`:

```yaml
name: docs review

on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  driftmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          # Both ends of the pull request must be in the clone, and the
          # files on disk must be the head's: the writing check reads them.
          fetch-depth: 0
          ref: ${{ github.event.pull_request.head.sha }}
      - uses: cj-ways/driftmark/.github/actions/driftmark@main
        with:
          folders: docs
```

- **`pull-requests: write`** is the only permission the comment needs. GitHub lets a token with either "Issues" or "Pull requests" write create and edit a pull request's comments, because every pull request is an issue ([permissions for fine-grained tokens](https://docs.github.com/en/rest/authentication/permissions-required-for-fine-grained-personal-access-tokens)). `contents: read` is for the checkout of a private repository.
- **`@main`** follows driftmark's main branch. Pin a commit id or, once releases exist, a tag, to keep the action from changing under you.
- **Only some pull requests.** `on: pull_request: paths: ['**/*.md', '**/*.markdown']` runs the job only when Markdown changed. The cost: when a later push removes every Markdown change, the job does not run, and the comment keeps its last summary.

### Inputs

| Input | Default | Meaning |
|---|---|---|
| `base` | the pull request's base commit | The commit to compare with. driftmark compares the head with where it left the base (`base...head`), as the pull request's own diff does. |
| `head` | the pull request's head commit | The commit to review. |
| `folders` | empty | The folders to read, from the repository root, separated by spaces or lines, such as `docs specs`. Empty reads every Markdown file. A folder name cannot hold a space. |
| `comment` | `true` | `true` posts one comment and updates it later; `false` only writes the job summary. |
| `lint` | `report` | `report` shows the writing check's findings as annotations and never fails the job; `fail` also fails the job when a changed file has an error; `off` skips the check. |
| `version` | `source` | `source` builds driftmark from the action's own repository; `0.1.0` or `latest` downloads that release; `installed` uses the driftmark already on `PATH`. See "Getting driftmark onto the runner". |

The outputs are `documents` (how many Markdown documents changed), `export` (the HTML file's path on the runner), `artifact-url` and `comment-url`.

### What the action does

The steps live in `.github/actions/driftmark/action.yml`, and their shell code in `driftmark.sh` beside it. Every input reaches the shell through an environment variable, never pasted into a command, so a branch name or a folder cannot run as code.

1. **Get driftmark**: build it, take it from the cache, or download a release.
2. **Check.** `driftmark diff <base>...<head> --json=summary` counts the changed documents. `driftmark lint --format github --changed <base>...<head> <folders>` prints its annotations. When a document changed, `driftmark export changes <base>...<head> -o driftmark-changes-<head>.html -- <folders>` writes the file, where `<head>` is the head's first 12 characters.
3. **Upload** the HTML file with `actions/upload-artifact@v7` and `archive: false`, so the reviewer downloads the file itself, not a zip. The artifact's address needs a GitHub login, as every artifact does.
4. **Comment.** `driftmark diff <base>...<head> --format markdown-summary --link <artifact address>` writes the comment. It goes to the job summary every time. With `comment: true` it is posted or updated (see "One comment, updated").
5. **Finish.** With `lint: fail`, the job fails when the check found an error. With `report`, an error is only an annotation.

**Annotations.** GitHub shows at most 10 error and 10 warning annotations per step, and 50 per job; the rest stay in the log ([actions/toolkit, problem matchers](https://github.com/actions/toolkit/blob/main/docs/problem-matchers.md)). The check reads only the files the pull request changed, so old problems elsewhere do not crowd out the new ones.

**A pull request from a fork** gets a read-only token, so the comment cannot be written. The action then prints a warning and goes on; the job summary still has the comment. The export and the annotations work as usual.

**The runner** needs bash, git, curl and jq, which every GitHub-hosted runner has. Linux and macOS runners are what the tests cover; Windows is untested.

## GitLab

### A ready `.gitlab-ci.yml`

```yaml
include:
  - remote: https://raw.githubusercontent.com/cj-ways/driftmark/main/ci/gitlab/driftmark.gitlab-ci.yml

driftmark:
  variables:
    DRIFTMARK_FOLDERS: docs
```

Then give the job a token: in the project's **Settings › CI/CD › Variables**, add `DRIFTMARK_GITLAB_TOKEN`, masked, holding a project access token.

- **The token needs the `api` scope.** It is the only scope that lets a token write notes; `read_api` only reads. The role must be one that can comment on merge requests: Reporter or higher.
- **Without the token** the job still renders the file and the report; it says in its log that it posted no note.
- **While the repository is private**, `include: remote` cannot fetch the template. Copy `ci/gitlab/driftmark.gitlab-ci.yml` into your repository and use `include: local:` instead.

### What the job does

The template defines one job, `driftmark`, in the `test` stage. Override any of its keys in your own `driftmark:` job, such as `stage`, `image` or `variables`.

- **When.** Only in merge request pipelines: `rules: - if: $CI_PIPELINE_SOURCE == "merge_request_event"`.
- **The range.** From `CI_MERGE_REQUEST_DIFF_BASE_SHA` (the base of the merge request's diff) to the source branch's head: `CI_MERGE_REQUEST_SOURCE_BRANCH_SHA` in a merged results pipeline, `CI_COMMIT_SHA` otherwise. `GIT_DEPTH: "0"` keeps both in the clone.
- **The check.** `driftmark lint --format gitlab --changed <range>` writes `driftmark-code-quality.json`, kept as `artifacts:reports:codequality`.
- **The file.** `driftmark export changes <range> -o driftmark-changes.html` writes the file, kept as an artifact for 30 days, and `expose_as: driftmark changes` links it from the merge request ("View exposed artifact"). `expose_as` needs a fixed file name, so this name never changes.
- **The note.** It links to `$CI_JOB_URL/artifacts/file/driftmark-changes.html`, the job's own view of that file. GitLab opens an HTML artifact in the browser only when GitLab Pages is on for the instance, and for a private project only with Pages access control too; otherwise the reader downloads it. GitLab's current documentation shows the per-branch form of this address, not this per-job form; the live GitLab check (not done yet) should confirm it.
- **Variables.**

| Variable | Default | Meaning |
|---|---|---|
| `DRIFTMARK_FOLDERS` | empty | The folders to read, separated by spaces. |
| `DRIFTMARK_COMMENT` | `true` | `true` posts and updates the note; `false` does not. |
| `DRIFTMARK_LINT` | `report` | `report`, `fail` or `off`, as the GitHub input `lint`. |
| `DRIFTMARK_VERSION` | `source` | `source`, a version, `latest`, or `installed`, as the GitHub input `version`. |
| `DRIFTMARK_SOURCE` | `https://github.com/cj-ways/driftmark.git` | Where `source` builds from. |
| `DRIFTMARK_REF` | `main` | The branch, tag or commit `source` builds. |
| `DRIFTMARK_RELEASES` | `https://github.com/cj-ways/driftmark/releases` | Where a release is downloaded from; a mirror works. |
| `DRIFTMARK_GITLAB_TOKEN` | none | The token that writes the note. Set it in the project's CI/CD variables, never in the file. |

**Code Quality across tiers.** Every tier shows the findings in the merge request's Reports tab; only GitLab Ultimate also shows them next to the lines in the Changes tab ([Code Quality](https://docs.gitlab.com/ci/testing/code_quality/)). GitLab marks a finding as new by comparing it with the target branch's report. This job runs only in merge request pipelines, so the target branch has no report of its own.

**The image** is `rust:1` for `source`. With a release or `installed`, any image with bash, git and curl works; the script installs jq with `apt-get` or `apk` when it is missing.

## The comment

### Ranking

Every change of the range is ranked:

1. the highest level first: decision, rule, fact, wording, formatting;
2. within a level, the documents keep the change list's order (the highest level first, then the most changes, then by path, as `docs/features/engine/change-list.md` says);
3. within a document, its changes keep their reading order.

So the same range always ranks the same way, and the comment's first line is the change the reader should look at first. The first ten are listed; the comment then says how many more there are.

### One line per change

`**level**, what happened · document › headings: text`

- **What happened**: edited, added, removed, moved, moved and edited, formatting only, new document or deleted document.
- **Document**: its path, at most 100 characters; a longer path loses its first folders, as in the text summary.
- **Headings**: the headings above the block, outermost first, at most 80 characters; when they do not fit, the outer ones go first. Left out for a block above every heading and for a whole document.
- **Text**: the changed block's plain text on one line, at most 120 characters, ending with `…` when cut. It is the new text, or the old text for a removed block. For a new or deleted document it is its title, or its summary when it has no title.

### Safety

Every text from the repository (paths, headings, changed text, folder names) sits inside a Markdown code span. GitHub and GitLab turn `@name` into a mention that notifies someone, `#12` into a link to an issue, and raw HTML into markup, but neither looks inside code. Without this, a document that names a team's handle would page that team each time an agent edited the sentence. The span's fence is one backtick longer than any run of backticks inside it, so no text can close it.

`--link` takes only an `https://` or `http://` address with no space, `<`, `>` or backslash, written between `<` and `>`.

### Size

Each part of a line is cut to a fixed length, ten lines at most are listed, and the last line names at most five folders. So a comment stays under 16,000 characters, whatever the range holds. The hardest case the tests build (every text made of backticks, a 2,048-character link, 100 folders) comes to 13,190 characters. GitHub refuses a comment over 65,536 characters (its API's own error text), and GitLab takes up to 1,000,000 ([notes API](https://docs.gitlab.com/api/notes/)).

### One comment, updated

The comment's first line is always `<!-- driftmark:changes -->`, which Markdown hides.

- **GitHub.** The action reads the pull request's comments, 100 per page, and takes the first one written by `github-actions[bot]` (the job's token) whose text starts with that line. It edits that comment (`PATCH /repos/{owner}/{repo}/issues/comments/{id}`), or posts a new one when there is none (`POST /repos/{owner}/{repo}/issues/{number}/comments`).
- **GitLab.** The job asks who the token is (`GET /user`), reads the merge request's notes, 100 per page, oldest first, and takes the first note by that user, not a system note, whose text starts with that line. It edits it (`PUT .../notes/{id}`) or posts a new one (`POST .../notes`).
- **Someone else's comment** that starts with the same line is never edited.
- **Nothing changed.** When the range changes no Markdown document and no comment exists yet, nothing is posted. When a comment exists, it is updated to say that no document changed.
- **A comment that cannot be written** is a warning in the log, never a failed job.

## Getting driftmark onto the runner

- **`source`** builds driftmark from the action's own repository at the ref in `uses:` (on GitHub), or from `DRIFTMARK_SOURCE` at `DRIFTMARK_REF` with `cargo install` (on GitLab). It is a dev build with no debug info and no incremental files (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_INCREMENTAL=0`), as driftmark's own CI builds, because a hosted runner has about 14 GB free. Symbols are stripped, and debug assertions and overflow checks are off, so it behaves as a release build does. It needs Rust 1.95 or newer; on GitHub, an older toolchain is updated with `rustup` when rustup is there.
  - **The cache.** On GitHub the binary is cached under a key made from the SHA-256 of every file in `Cargo.toml`, `Cargo.lock` and `crates/`, so the next run with the same sources skips the build. It is saved right after the build, so a later failing step does not lose it. On GitLab the job's cache keeps the binary with the commit it was built from, and a new commit is built again.
  - **Why a dev build.** Measured on 2026-11-02 on the maintainer's server, 4 build jobs, at a load of 11 to 26: a clean dev build took 51 seconds, a release build without link-time optimisation 167 seconds, and the workspace's release build 302 seconds. The stripped dev build is 112 MB, about 35 MB compressed in the cache. On a real merge of the private corpus, its export took 0.17 seconds against 0.02 for a release build; on 30 days of that corpus, 6.5 seconds against 1.4 to 1.7.
- **A version** such as `0.1.0`, or `latest`, downloads a release, once M6 publishes them. The files the action expects, as cj-ways/acctswap's release workflow names them:
  - `driftmark-<target>.tar.gz` (`.zip` on Windows), with the program at the archive's root, for the targets `x86_64` and `aarch64` of `unknown-linux-gnu` (decision 0011: glibc, not musl), `apple-darwin` and `pc-windows-msvc`;
  - `SHA256SUMS`, one `<sha256>  <file>` line per archive;
  - under the tag `v<version>`, so `https://github.com/cj-ways/driftmark/releases/download/v0.1.0/…`, or `…/releases/latest/download/…` for `latest`.

  The archive is refused unless its SHA-256 matches its line. While the repository is private, a release cannot be downloaded from another repository.
- **`installed`** uses the driftmark on `PATH`, such as one an earlier step installed, or a GitLab image that ships it.

## Tests

- `crates/driftmark/tests/ci_scripts.rs` runs the action's steps and the template's script with bash, as a runner does, on a made-up repository whose branch edits a decision record, adds a page with a broken link, touches a fixture the rules file excludes and a file outside the folders. A small HTTP server on 127.0.0.1 plays GitHub's comments API, GitLab's notes API and a release host, and records every request. The tests check:
  - the annotations name only the changed files, without the excluded fixture, and the export holds only the given folders;
  - the first run posts one comment with the marker, the link and the right counts, reading two pages of 120 older comments; a second push edits that same comment; someone else's comment with the marker stays as it was;
  - a read-only token, a range with no change, `comment: false`, and a run outside a pull request each post nothing and keep the job green;
  - `lint: fail` fails the job on an error, and `report` never does;
  - the cache key stays the same for the same sources, a cached build is used without building, and a release is downloaded, checked against `SHA256SUMS`, and refused when its sum does not match;
  - on GitLab, the Code Quality report, the export, one note posted and then updated, system notes and other people's notes left alone, a release downloaded, and a job without a token that still exports.

  The scripts need bash, git, curl, jq and tar; the tests run on Linux and macOS, and say so and pass when a tool is missing.
- `crates/driftmark/tests/lint_command.rs` checks `--changed` (removed files left out, paths narrowing, `exclude` on named files, `auto`) and both CI formats. `src/lint/ci.rs` checks the escaping, including a message that tries to start a second workflow command, and the fingerprints.
- `crates/driftmark/tests/diff_command.rs` checks the whole comment for the example in `docs/features/commands/diff.md`. `src/diff/markdown.rs` checks the ranking, the top ten, the code spans against mentions, references, HTML and backticks, the cuts, and the size of the worst case.
- **The source build itself** is too slow for the test suite. It was run once by hand on 2026-11-02: the action's `key` and `install` steps with an empty cache built in 39 seconds, and the template's `cargo install` line against this repository's `main` in 40 seconds; both programs printed their version.
- **Live checks.** A pull request on this repository runs the action end to end (see the M5 plan). GitLab is checked against its documented API and the local run above, until a GitLab project is named for a live check.

## Not covered

- **Line comments.** Findings are annotations or a Code Quality report, not review comments on the diff.
- **Links into the file.** The comment links the whole file. A change's own place in it (`#C3.0`, `docs/features/commands/export.md`) is reachable only when the host serves the file as a page, which GitHub's artifacts do not.
- **Bitbucket, Gitea, Forgejo and Azure DevOps.**
- **A target-branch Code Quality report**, which would let GitLab mark only new findings.
- **Windows runners** for the action are untested.
