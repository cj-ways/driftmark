---
status: target
date: 2026-10-18
---

# The diff command

> Short version: `driftmark diff <range>` compares the Markdown files of a git repository between two points. It prints one line per changed document with its highest level and its counts, then the totals. `--json` prints the whole change list instead, for tools. The rules file is read at the newer side of the range. The exit code is 0 when it ran, 1 when it could not finish, and 2 when the command line is wrong.

## Running it

```text
driftmark diff [<options>] <range> [-- <folder>...]
driftmark diff [<options>] --since <date> [<branch>] [-- <folder>...]
```

| Command | What it compares |
|---|---|
| `driftmark diff main..feature` | The tip of `main` with the tip of `feature`. |
| `driftmark diff main...feature` | The tip of `feature` with the commit where it left `main`: what a merge request shows. |
| `driftmark diff auto` | The files on disk with the commit where this branch left the default branch, so work not committed yet is included. |
| `driftmark diff --since 2026-10-01` | Everything that reached the current branch from 1 October on. |
| `driftmark diff --since 2026-10-01 main` | The same on `main`. |
| `driftmark diff main...feature -- docs` | Only the files under `docs`. |

`docs/features/engine/git-ranges.md` says exactly how each range form turns into two points: a side left out means `HEAD`, which merge-base `...` uses, how `--since` walks back, and which default branch `auto` picks.

### Options

| Option | Meaning |
|---|---|
| `--repo <path>` | A folder inside the repository. The default is the current folder. |
| `--json` | Print the whole change list as JSON instead of the summary. |
| `--timings` | Print the time of each step on standard error. Standard output does not change. |
| `--threads <n>` | Read, parse and compare files on `n` threads. The default is one per processor core. |
| `--help` | Print the usage text. |

- Options may come before or after the range, and take their value as the next word or after `=`, as in `--threads=4`.
- **Folders** come after `--`. They start at the repository root, wherever the command runs, so `-- docs` means the same from every subfolder. A folder inside another adds nothing. Without folders, every Markdown file in the repository counts.
- **Which files.** A file counts when its name ends in `.md` or `.markdown`, in any case, and it is a regular file. A moved file is one deleted file and one new file. `git-ranges.md` has the details.

### The rules file

The rules file is `driftmark.toml` at the repository root, read at the newer side of the range:

- at the head commit for `..`, `...` and `--since`;
- from the disk for `auto`, committed or not.

Both versions of every document are classified with these same rules. So a change to the rules file inside the range already applies to the documents it changes. A range whose head has no rules file uses the default rules: no types, so no statuses and no decision sections.

A rules file that cannot be used stops the command with exit code 1. The message says where the file was read and on which line the problem is:

```text
driftmark diff: driftmark.toml at commit 2cc1179a6f87: the rules file could not be read: TOML parse error at line 3, column 1
  |
3 | stauts = 1
  | ^^^^^^
unknown field `stauts`, expected one of `name`, `paths`, `status`, …
```

```text
driftmark diff: driftmark.toml in the working tree, line 6: the rules file's type 2 ("feature"): it lists no paths
```

A problem in one type names the line where that type's `[[type]]` table starts, and a problem in the `[lint]` table names the line of `[lint]`. `docs/features/engine/rules-file.md` describes the file.

## An example

A made-up repository has a branch `feature` that changes a decision record, rewords a guide, deletes one page, adds another, moves `README.md` to `docs/readme.md`, and adds this rules file:

```toml
[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status" }
current = ["accepted"]
other-statuses = ["proposed"]
decision-headings = ["Decision"]
```

The decision record is the one in `docs/features/engine/change-list.md`: its status goes from `proposed` to `accepted`, a number changes in its decision, and a sentence with `never` is added.

```text
$ driftmark diff main..feature
main..feature: from 3828db3aa184 to 2cc1179a6f87

level       changes  document
decision     3 of 5  docs/decisions/0002-cache.md (status proposed -> accepted)
fact         2 of 2  docs/new.md (new)
fact         1 of 2  docs/old.md (deleted)
fact         1 of 1  README.md (deleted)
fact         1 of 1  docs/readme.md (new)
wording      1 of 1  docs/guide.md

6 documents changed, 12 changes
By level: 3 decision, 6 fact, 3 wording
By kind: 3 edited, 4 added, 5 removed
```

`crates/driftmark/tests/diff_command.rs` builds this repository and checks this exact text.

## The summary

1. **The first line** is the range as written, then its two points: `from <base> to <head>`. Commits show their first 12 characters. The head of `auto` is `the working tree`. A `--since` range that starts before the first commit says `from before the first commit`.
2. **`Only in:`** lists the folders, when some were given.
3. **`Note:`** lines say how the points were chosen, such as ``compared with `origin/main` `` for `auto`, or which merge-base was used when there were several.
4. **One line per changed document**, in the order of the change list: the highest level first, then the most changes, then by path.
   - `level` is the document's highest level: `decision`, `rule`, `fact`, `wording` or `formatting`. It is `none` when the file's bytes changed but no block did, for example only blank lines.
   - `changes` reads `3 of 5`: 3 of the document's 5 changes are at its highest level.
   - The path runs from the repository root. It comes last, so the columns stay in place.
   - In a terminal, each document line fits the terminal's width. A path too long for its line loses its first folders, as `git diff --stat` does: `docs/features/pages/cache/lifetime/decisions.md` can become `.../lifetime/decisions.md`. A path always gets room for 20 characters, so a line with a very long status may still run past the width. The width is 80 columns, or `COLUMNS` when that holds a number of at least 40. In a file or a pipe, paths stay whole, because a program or an agent may read them.
   - After the path: `(new)` for an added file, `(deleted)` for a removed one, `(status proposed -> accepted)` when the status changed, and `(blank lines only)` when no block changed.
5. **The totals**: how many documents and changes, then the changes by level and by kind. Counts of zero are left out. A list longer than the width wraps between items.

When nothing changed, the summary says `No Markdown files changed.` after the first line.

**Colours.** In a terminal, the level is coloured: decision bold red, rule bold yellow, fact cyan, formatting and none dimmed. There are no colours when standard output is not a terminal, when the `NO_COLOR` variable is set and not empty, or when `TERM` is `dumb`. On Windows, colours appear only in Windows Terminal or a terminal that sets `TERM`, because the older console prints colour codes as text.

**Text from the repository is escaped.** A file name or a status may hold control characters, such as a line break or a terminal escape sequence. The summary writes them as `\n` or `\u{1b}`, so they cannot split a line or change the terminal.

## JSON

`--json` prints the change list on one line, followed by a line break. Its shape is the public contract in `docs/features/engine/change-list.md`, with `schema_version` 1. `range.base` and `range.head` are full commit ids, or `null` before the first commit and `worktree` for the files on disk.

The JSON holds the plain text of every block that changed, so it grows with the size of the changes. A range that adds or deletes whole documents makes it large: 30 days of the private corpus give 95.5 MB.

## Files that cannot be compared

A version that driftmark cannot parse skips its file instead of stopping the command: a file that is not UTF-8 text, or one that nests more than 100 levels deep. The file is left out of the summary and the JSON. Standard error names it and the reason:

```text
driftmark diff: skipped docs/latin1.md: the new version cannot be compared: the file is not UTF-8 text: byte 5, on line 1, is not valid UTF-8
```

The summary's totals add `; 1 file skipped, named on standard error`. The exit code stays 0.

## Timings

`--timings` prints this on standard error after the output is written. Here it is for the example above, on a machine with 12 cores:

```text
driftmark diff: timings in milliseconds, on 6 threads
  open the repository                  0.6
  resolve the range                    0.1
  read the rules file                  0.1
  list the changed files               0.2
  read, parse and compare              0.9
    reading, all threads               0.2
    parsing, all threads               0.9
    comparing, all threads             0.6
  sort the documents                   0.0
  write the output                     0.1
  total                                2.0
driftmark diff: 6 changed files, 8 versions, 7 parsed
```

- The run uses at most one thread per changed file, so this small range ran on 6 threads.
- Each step is wall-clock time, and the steps add up to the total. The total runs from reading the command line to the last byte written. Starting the program and ending it are not inside it.
- The three indented lines add up the time each thread spent reading, parsing and comparing. With several threads they add up to more than the step's own time.
- The last line counts the changed files, the versions they name (two for a modified file, one for an added or removed one), and how many of those were parsed. A blob that several files share, such as a file moved with its bytes unchanged, is read and parsed once per run. In the example, `README.md` moved to `docs/readme.md` unchanged, so 8 versions needed 7 parses. Working-tree files are always read from disk, because their bytes can differ from the blob id git gives them.

## Exit codes

| Code | When |
|---|---|
| 0 | It ran, whether or not anything changed. Skipped files do not change this. |
| 1 | It could not finish: the folder is not inside a git repository, a name leads to no commit, two points share no history, `auto` finds no default branch, the rules file is broken, git cannot be read, or the output cannot be written. The reason goes to standard error. |
| 2 | The command line is wrong: an unknown option, a missing range or value, text that is not a range, a date in neither form, `--threads` below 1, or a folder that cannot be inside a repository. The reason and the usage text go to standard error. driftmark finds these before it opens anything. |

Standard output is empty for codes 1 and 2. A reader that stops early, such as `driftmark diff auto | head`, does not count as a failed write; the code stays 0. Any other code, such as 101, means driftmark crashed, which is a bug.

## Speed

Measured on the maintainer's server, which has 12 cores, on 2026-10-18. Every figure comes from the real command, a release build, with `--timings` and the output sent to `/dev/null`. Each is the `total` line, the median of 3 runs after one warm-up run. The load average was about 5. Earlier that day, at a load of 32 to 42, the same runs took up to 632 ms on all cores and 2.2 seconds on one thread.

The two ranges come from the private corpus:

- a 19-commit branch against its merge-base: 20 changed files;
- 30 days of its main branch, with `--since` from a fixed tip: 2,652 changed files under `docs`, and 2,943 in the whole repository.

| Range | All cores, summary | All cores, JSON | 1 thread, summary | 1 thread, JSON |
|---|---|---|---|---|
| Branch, `-- docs` | 20 ms | 23 ms | 44 ms | 45 ms |
| Branch, whole repository | 21 ms | 23 ms | 45 ms | 45 ms |
| 30 days, `-- docs` | 204 ms | 337 ms | 1,758 ms | 1,896 ms |
| 30 days, whole repository | 236 ms | 374 ms | 1,944 ms | 2,167 ms |

Decision 0008 sets the limits: under 1 second on all cores and under 5 seconds on one thread for 30 days, and under 0.1 seconds for a typical branch. Every row is inside them.

- **Where the time goes.** For 30 days of `docs` on one thread: reading 161 ms, parsing 895 ms, classifying and comparing 662 ms. Resolving the range takes 8 ms and listing the changes 6 ms. On 12 threads the per-file work runs about 11 times faster.
- **The cache.** The 30-day range of `docs` names 3,550 file versions and parses 3,283 of them. The other 267 are blobs that two files share, such as a file moved with its bytes unchanged.
- **JSON.** The 30-day change list is 95.5 MB for `docs` and 108.7 MB for the whole repository. Writing it takes 118 to 137 ms to `/dev/null`, and 165 to 200 ms to a file.
- **Memory.** For 30 days the program peaks at 259 to 294 MB on all cores and 188 to 211 MB on one thread. For the branch it peaks at 20 to 37 MB.
- **Starting and ending.** The time a shell measures is about 4 ms more than `total` for the branch, and 75 to 80 ms more for 30 days. The larger gap is probably the end of the process, when the change list's memory is freed. That was not measured on its own.
- **Width.** For 30 days of the whole repository in an 80-column terminal, no document line was wider than 80 columns, and 1,953 of the 2,943 paths were shortened. Sent to a pipe, where paths stay whole, 66% of the lines were wider.

What fills the 95.5 MB of the 30-day range of `docs`:

| Part | Size |
|---|---|
| Added and removed files | 79.3 MB (83%) |
| The plain text of blocks | 34.5 MB (36%), of which 27.8 MB in added or removed files |
| Heading paths, written again on every block | 17.2 MB (18%) |
| Word marks, sentences and lines of edits | 3.3 MB (3%) |
| The list without the `changes` of added and removed files, keeping their summaries | 16.8 MB in all |
| Only each document's summary, without any changes | 0.8 MB in all |

## Not covered

- **Relative dates** such as `30 days ago`. `--since` takes a date or a moment.
- **A slimmer JSON.** The change list is always written whole. The speed section shows what a slimmer form could save.
- **Folders in the rules file.** Folders are given on the command line. The rules file cannot yet say which folders hold documents.
- **Renamed files** are one deleted and one new document, as in `git-ranges.md`.
- **Colour settings.** There is no `--color` option; `NO_COLOR` turns colours off.
- **Pages, notes and the writing check** belong to `driftmark serve` (M2), the notes loop (M3) and `driftmark lint` (`docs/features/lint.md`).
