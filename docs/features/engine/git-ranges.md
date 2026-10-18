---
status: target
date: 2026-10-18
---

# Git ranges and readers

> Short version: driftmark asks a git repo five kinds of question through one interface, `GitRead`, in `driftmark-core`. Two readers in the `driftmark` program answer them: gix, and the `git` command. Both must give identical answers. A range is written as `<base>..<head>`, `<base>...<head>`, `--since <date>` or `auto`. `auto` includes changes not committed yet. Only `.md` and `.markdown` files count, and renames are not detected.

## What driftmark asks git

The interface is `driftmark_core::git::GitRead`. Every method has one answer, and both readers must give it.

| Question | Method | Answer |
|---|---|---|
| Which commit does this name mean? | `resolve(name)` | A commit id. A name is anything git accepts: `main`, `v2.0`, `a1b2c3d`, `HEAD~2`. A tag leads to its commit. |
| Where did two commits part? | `merge_bases(a, b)` | Their best common ancestors, in any order. Usually one, none for unrelated histories, several after criss-cross merges. |
| When was a commit made, and what came before it? | `read_commit(id)` | The raw commit bytes. `CommitInfo::parse` reads the parents and the committer date from them. |
| Which Markdown files exist here? | `markdown_files(at, roots)` | Each file's path and blob id, at a commit or in the working tree. |
| Which Markdown files changed between two commits? | `markdown_changes(base, head, roots)` | Each changed file: added, removed or modified, with its old and new blob ids. |
| What is in this file? | `read_blob(id)` | The blob's bytes, exactly as stored. |
| What is in this working-tree file? | `read_worktree_file(path)` | The bytes on disk now. |

Every list comes back sorted by path, in byte order, with each path once.

Errors that mean the same thing for both readers have their own case:

- `UnknownRevision`: the name leads to no commit.
- `MissingObject`: the object is not in the repo, for example in a partial or shallow clone.
- `WrongKind`: the id names a commit where a blob was asked for, or the other way round.
- `NoWorktree`: the repo is bare, so it has no working tree.
- `MissingFile`: no regular file is at that path in the working tree.

Anything else is `Other`, in the reader's own words.

## Which files count

A file counts when all of these hold:

1. Its name ends in `.md` or `.markdown`, in any mix of upper and lower case. `README.MD` counts. `notes.mdx` and `plan.md.bak` do not.
2. It is a regular file. Symbolic links and submodules are skipped, even when their name ends in `.md`. An executable file counts.
3. It lies inside one of the roots.

**Roots** are the folders driftmark reads. No roots means the whole repo. The rules file will narrow them later, for example to `docs`. `Roots::new(["docs", "specs/api/"])` reads those two folders; the trailing `/` is optional. A folder inside another root adds nothing and is dropped. A root must be a folder path inside the repo: `/docs`, `./docs` and `docs/../x` are refused. A root that names a file holds nothing, and a root that does not exist holds nothing.

A path inside the repo is bytes, not text, because git allows any byte but NUL in a name. A file named in Latin-1, such as `café.md` written by an old tool, still works.

## Range forms

A range names two points: the base, which is older, and the head, which is newer. `RangeSpec::parse` reads the first three forms below, and `RangeSpec::since` builds the fourth. `RangeSpec::resolve` turns any of them into commits.

### `<base>..<head>`: compare two points directly

`main..feature/login` compares the tip of `main` with the tip of `feature/login`. Changes made on `main` since the branch started show up too, reversed. This matches `git diff main..feature/login`.

A side left out means `HEAD`, as in git:

- `main..` is `main..HEAD`.
- `..feature/login` is `HEAD..feature/login`.

### `<base>...<head>`: compare with where the branch started

`main...feature/login` compares the tip of `feature/login` with its merge-base with `main`, the commit where the branch left `main`. Only what the branch added shows up. This matches `git diff main...feature/login`, and it is what a merge request shows.

A side left out means `HEAD` here too: `main...` is `main...HEAD`.

When the two have several merge-bases, after criss-cross merges, the one with the lowest id is used. The range then carries a note that says so and lists the others. Another merge-base would show different changes.

When the two share no history, resolving fails with `NoMergeBase`.

### `--since <date>`: everything since a date on one branch

`--since 2026-10-01` on `main` shows everything that reached `main` from 1 October on. The branch is `HEAD` when none is given.

How the base is found:

1. Start at the tip of the branch.
2. Walk back through first parents only. On a branch that merges others, these are the branch's own commits and its merge commits.
3. Stop at the first commit made before the date. That commit is the base, and the tip is the head.

Example. `main` holds these commits, oldest first, with their commit dates:

| Commit | Date |
|---|---|
| `a` | 28 September |
| `b` | 30 September |
| `c` | 1 October, 10:00 |
| `d` | 5 October |

`--since 2026-10-01` gives base `b` and head `d`, so the changes of `c` and `d` show. A commit made exactly at the date is inside the range.

Two edge cases:

- If every commit is from the date on, there is no base. Every file at the tip counts as added.
- If the tip itself is older than the date, the base is the tip, and the range is empty.

The date is the committer date, the one `git log --since` uses. It changes when a commit is rebased.

Two date forms are accepted:

- `2026-10-01` means midnight at the start of that day in UTC.
- `2026-10-01T09:30:00+04:00` or `2026-10-01T05:30:00Z` means that moment.

A date alone is UTC, not local time, because the engine reads no time zone. Write the offset to mean local time: midnight at UTC+4 is `2026-10-01T00:00:00+04:00`. Years run from 1970 to 9999. Other forms, such as `last week`, are refused with `BadDate`.

### `auto`: this branch's work, committed or not

`auto` compares the working tree with the commit where this branch left the default branch.

1. The default branch is the first that exists of `origin/main`, `main`, `origin/master`, `master`, `origin/dev`, `dev`. Remote branches come first because a local `main` is often behind.
2. The base is the merge-base of `HEAD` with that branch.
3. The head is the working tree, so changes not committed yet are included.

The range carries a note naming the default branch it used, such as "compared with `origin/main`".

On the default branch itself, the base is usually `HEAD`, so `auto` shows only uncommitted changes. The word `auto` always means this range, even if a branch is named `auto`; write `auto..` for that branch.

`auto` fails with `NoDefaultBranch` when none of the six branches exists, and with `UnknownRevision` for `HEAD` in a repo with no commits yet.

### Text that is not a range

A name alone, such as `main`, is refused with `NotARange`, as are `..`, `...`, and stray dots such as `a....b` or `a..b..c`. The message lists the accepted forms.

## Uncommitted work: the working-tree side

Yes, the interface has a working-tree side: `Snapshot::Worktree`. A range's head is either `Snapshot::Commit(id)` or `Snapshot::Worktree`. Only `auto` uses the working tree today.

**Which files.** The working tree holds the Markdown files on disk that git tracks, because they are in the index, or would offer to track, because they are untracked and not ignored. So a new file an agent wrote but never ran `git add` on still shows up. These are left out:

- a tracked file deleted from disk, which then shows as removed;
- an ignored file;
- a symbolic link, and any file reached through a linked folder.

**Which ids.** Each file's id is the blob id git would store if the file were added now. Git's line-ending conversion and filters apply. So a file git calls unchanged keeps its committed id, and drops out of the change list. Example: on Windows with `core.autocrlf=true`, a checked-out file has CRLF line endings on disk while its blob has LF. Its id is still the committed one, so it is not listed as changed.

**Which bytes.** `read_worktree_file` returns the bytes on disk, unconverted. They can differ from the blob its id names, in line endings. They may also have changed since the list was made. So working-tree bytes must never be cached under that id.

**How the changes are found.** `ResolvedRange::changes` lists all Markdown files at the base commit and in the working tree, then compares the two lists by path in `driftmark-core` (`diff_files`). Both readers share this step, so only the listing differs between them.

**How each reader lists the working tree.**

| Step | gix reader | `git` command reader |
|---|---|---|
| Tracked files | the index entries with a regular-file mode | `git ls-files -z --stage` |
| Untracked files | gix's directory walk, with ignore rules | `git ls-files -z --others --exclude-standard` |
| On disk | the same check in both: a regular file, reached through real folders only | |
| Ids | gix's filter pipeline, then the blob hash | `git hash-object --stdin-paths` |

Both readers hash every listed file on every call. For about 3,400 files that reads about 55 MB. Using the index's saved file sizes and times to skip unchanged files would be faster; that is not built yet.

## The two readers and the fallback

**gix reader** (`GixReader`). It reads the repo inside the process, so it starts no other program. It keeps one repo handle per thread in a small pool, so many threads can read at once. Each handle caches up to 16 MB of pack data, capped by bytes, and 4 MB of recent objects. The byte cap avoids gix's default cache, which needs an MPL-2.0 crate. gix is pinned to one version, because each 0.x release may change its interface.

**`git` command reader** (`CommandReader`). It runs `git` with plumbing commands whose output is meant for programs:

- `rev-parse --verify` to resolve names;
- `merge-base --all` for merge-bases;
- `ls-tree -r -z` to list files at a commit;
- `diff-tree -r -z --no-renames` to list changes;
- one long-lived `cat-file --batch` process to read commits and blobs;
- `ls-files` and `hash-object` for the working tree.

Paths are passed with `--literal-pathspecs`, so a folder name is never read as a pattern. Variables such as `GIT_DIR`, which would point git at another repo, are removed, and git never takes a lock or asks for a password.

**The chooser** (`Git`). The commands use this one.

1. It opens the repo with gix. If gix cannot open it, it opens it with the `git` command and says why on standard error.
2. It asks gix each question.
3. If gix fails with a missing object or an error of its own, it asks the `git` command instead. It says why once, on standard error, and uses the `git` command for the rest of the run.
4. An unknown name, a wrong kind, a missing working-tree file or a bare repo do not cause a switch. These are facts about the repo, and git would report them the same way.

The message reads, for example: `driftmark: gix could not read a file (object 3f2a… is not in the repo); using the git command from now on`.

## Tests

Tests build small repos with the `git` command in a temporary folder, then ask gix, the `git` command and the chooser every question and require identical answers. The repos hold:

- commits on two branches, a merge, a criss-cross merge and an unrelated history;
- a deleted file, a file in a subfolder, a moved file, a file that becomes a folder, and a file that becomes a symbolic link;
- files that are not Markdown, an executable file, a mode-only change, and CRLF content;
- a symbolic link and a submodule entry, a name that is not UTF-8, and, except on Windows, names with a newline and a quote;
- lightweight and annotated tags, and a remote branch;
- uncommitted edits, deletions, staged and untracked files, ignored files, links, and line-ending conversion;
- a bare repo.

Fallback tests cover a SHA-256 repo, which gix cannot open; a partial clone, where the `git` command fetches a missing file; and an object deleted from the repo.

## Not supported yet

- **Renames.** A moved file is one removed file and one added file. Rename detection in git was slow in the speed spike, and moves between files are not decided yet. If driftmark pairs moved files later, it should compare the block hashes it already has.
- **Partial clones in gix.** gix cannot fetch a missing file. The chooser switches to the `git` command, which fetches it, so this works but starts processes.
- **SHA-256 and reftable repos in gix.** gix opens SHA-1 repos only. The chooser uses the `git` command for these. The types already hold 64-character ids.
- **Shallow clones.** `--since` and `...` need the history they walk. When it was cut off, they fail with `MissingObject`. CI jobs that clone with a depth of 1 must fetch more history.
- **Sparse checkouts.** A tracked file left off the disk by a sparse checkout counts as removed in the working tree.
- **Local time for a date alone.** `--since 2026-10-01` means UTC. Relative dates such as `30 days ago` are not read; the `diff` command can turn them into a date.
- **Rare revision syntax.** gix and git agree on branch names, tags, ids, `~`, `^` and `@`. Rarer forms, such as reflog dates (`main@{yesterday}`), may resolve in git and not in gix. The chooser does not switch for an unknown name.
- **Skipping unchanged working-tree files.** Every working-tree file is hashed on each call.
