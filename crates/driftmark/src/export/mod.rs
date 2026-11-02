//! `driftmark export`: one HTML file for someone outside the repository
//! (decision 0005). It opens from disk in any browser, with no server and
//! no network, so it can travel as an email attachment, sit on a ticket, or
//! go with a merge request.
//!
//! Three things can be exported:
//!
//! - `doc`: one document at a revision;
//! - `folder`: every document under a folder, with an index;
//! - `changes`: a range's change list, with every document's change cards
//!   and the full text of the documents it changed.
//!
//! The pages are the server's (decision 0010), read through the same
//! [`Repo`], drawn by the same diagram [`Drawer`], and written by
//! `pages.rs` with the same classes. `file.rs` puts them in one file with
//! everything they need inside it. A folder or a range too big for the size
//! limit is cut down, and the file says what it left out.
//! `docs/features/commands/export.md` is the user's guide and the contract.

mod args;
mod file;
mod links;
mod pages;
mod prism;
mod search;

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write;

use driftmark_core::diff::{ChangeList, DocumentChanges, FileState, RangeInfo, Subject};
use driftmark_core::document::Field;
use driftmark_core::git::{GitRead, RepoPath, Roots, Snapshot};
use driftmark_core::render::{Setup, reading_prefixed};
use driftmark_core::rules::Classification;

use crate::diagram::Drawer;
use crate::git::Git;
use crate::serve::address::{RangeAddress, Revision};
use crate::serve::assets::Assets;
use crate::serve::compare::Skipped;
use crate::serve::page::{escape, plural, short};
use crate::serve::repo::{Problem, Repo, parallel};
use args::{Args, Request, What};
use links::{Links, Pages};
use search::{Index, Kind};

/// Exit code when the file was written.
pub const EXIT_OK: u8 = 0;
/// Exit code when the file could not be made: the folder is not in a git
/// repository, a revision or a document does not exist, or the file could
/// not be written.
pub const EXIT_ERROR: u8 = 1;
/// Exit code when the command line is wrong.
pub const EXIT_USAGE: u8 = 2;

/// How many bytes of parsed documents the export keeps while it works, as
/// the server does (`serve/cache.rs`).
const CACHE_BYTES: usize = 128 * 1024 * 1024;

/// The line at the top of every exported file: what it is, for a reader who
/// got it from someone else.
const ABOUT: &str = "Exported by driftmark: this file opens offline, holds no notes, and stays as it is when the documents change.";

/// How `driftmark export` is called.
const USAGE: &str = "\
usage: driftmark export doc <revision> <path> [<options>]
       driftmark export folder <revision> <folder> [<options>]
       driftmark export changes <range> [<options>] [-- <folder>...]
       driftmark export changes --since <date> [<branch>] [<options>] [-- <folder>...]

Writes one HTML file that opens offline in any browser, with no server:
one document, every document under a folder with an index, or a range's
changes with the full text of the documents it changed.

  doc       one document; <revision> is a branch, a tag, a commit, or
            `worktree` for the files on disk; <path> runs from the
            repository root
  folder    every Markdown document under <folder>; `.` is all of them
  changes   a range as `driftmark diff` takes it: main...feature, a..b,
            auto, or --since <date> [<branch>]; folders after `--` keep
            only the documents under them

Options:
  -o, --output <file>  write the file here (default: standard output)
  --repo <path>        a folder inside the repository (default: here)
  --max-size <size>    keep the file under this size, in bytes, or with K or
                       M for thousands or millions of bytes (default: 10M);
                       a folder or a range that passes it is cut down, and
                       the file says what it left out
  --base <branch>      the branch `auto` compares with

The file has no notes, no \"Seen\" and no live updates. Links to documents
it does not hold show as text. Search works inside it when it holds more
than one document.

Exit codes: 0 written, 1 could not be made, 2 the command line is wrong.";

/// Runs `driftmark export` with the arguments after `export`, and returns
/// the exit code. The file goes to `-o` or to `stdout`; what was written,
/// and what was left out, goes to `stderr`.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let args = match args::parse(args) {
        Ok(Request::Export(args)) => args,
        Ok(Request::Help) => {
            let _ = writeln!(stdout, "{USAGE}");
            return EXIT_OK;
        }
        Err(message) => {
            let _ = writeln!(stderr, "driftmark export: {message}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let made = match export(&args) {
        Ok(made) => made,
        Err(message) => {
            let _ = writeln!(stderr, "driftmark export: {message}");
            return EXIT_ERROR;
        }
    };
    let written = match &args.output {
        Some(path) => std::fs::write(path, &made.html)
            .map_err(|error| format!("could not write `{}`: {error}", path.display())),
        None => stdout
            .write_all(made.html.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| format!("could not write to standard output: {error}")),
    };
    if let Err(message) = written {
        let _ = writeln!(stderr, "driftmark export: {message}");
        return EXIT_ERROR;
    }
    let place = args.output.as_ref().map_or_else(
        || "standard output".to_owned(),
        |path| format!("`{}`", path.display()),
    );
    let _ = writeln!(
        stderr,
        "driftmark export: wrote {place}: {}, {} bytes",
        plural(made.documents, "document"),
        made.html.len()
    );
    if let Some(cut) = &made.cut {
        let _ = writeln!(stderr, "driftmark export: {cut}");
    }
    EXIT_OK
}

/// A finished file.
struct Made {
    /// The whole file.
    html: String,
    /// How many documents it shows, in full or by their changes.
    documents: usize,
    /// What it left out to stay under the size limit, in words.
    cut: Option<String>,
}

fn export(args: &Args) -> Result<Made, String> {
    let git = Git::open(&args.repo).map_err(|error| {
        format!(
            "`{}` is not inside a git repository that can be read: {error}",
            args.repo.display()
        )
    })?;
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let base = match &args.what {
        What::Changes { base, .. } => base.clone(),
        What::Document { .. } | What::Folder { .. } => None,
    };
    let repo = Repo::new(git, CACHE_BYTES, threads, base);
    let work = Work {
        repo: &repo,
        drawer: Drawer::new(),
        assets: Assets::built_in(),
        limit: usize::try_from(args.max_bytes).unwrap_or(usize::MAX),
        threads,
    };
    let made = match &args.what {
        What::Document { revision, path } => work.document(revision, path),
        What::Folder {
            revision,
            label,
            roots,
        } => work.folder(revision, label, roots),
        What::Changes {
            label,
            range,
            roots,
            ..
        } => work.changes(label, range, roots),
    };
    made.map_err(|problem| problem.message)
}

/// What every export uses.
struct Work<'r> {
    repo: &'r Repo,
    drawer: Drawer,
    assets: Assets,
    /// The size limit, in bytes.
    limit: usize,
    threads: usize,
}

/// One document, rendered as a page of the file.
struct Rendered {
    path: String,
    title: Option<String>,
    summary: Option<String>,
    classification: Option<Classification>,
    fields: Vec<Field>,
    /// The page.
    html: String,
    /// Each block's text, for the search index.
    texts: Vec<String>,
    /// The languages its code blocks name, as written.
    languages: Vec<String>,
}

impl Rendered {
    fn search_page(&self, id: &str) -> search::Page {
        search::page(
            id,
            self.title.as_deref().unwrap_or(&self.path),
            &self.path,
            Kind::Reading,
            self.texts.iter().map(String::as_str),
        )
    }
}

/// The languages a page's code blocks name, from the reading HTML's
/// `data-language` attributes.
fn languages(html: &str) -> Vec<String> {
    const ATTRIBUTE: &str = " data-language=\"";
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(ATTRIBUTE) {
        rest = &rest[at + ATTRIBUTE.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        let value = rest[..end]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        if !out.contains(&value) {
            out.push(value);
        }
        rest = &rest[end..];
    }
    out
}

/// A size limit in words: `10 MB`, `500 KB` or `1234 bytes`.
fn size_words(bytes: usize) -> String {
    if bytes.is_multiple_of(1_000_000) {
        format!("{} MB", bytes / 1_000_000)
    } else if bytes.is_multiple_of(1_000) {
        format!("{} KB", bytes / 1_000)
    } else {
        format!("{bytes} bytes")
    }
}

/// The note at the top of a cut-down file.
fn cut_note(text: &str) -> String {
    format!(
        "<div class=\"xcut\" role=\"note\"><p>{}</p></div>\n",
        escape(text)
    )
}

/// What a range's page is called: the branch of `<default>...<branch>`,
/// "This checkout" for `auto`, otherwise the range, as on the server.
fn range_heading(label: &str) -> String {
    if label == "auto" {
        return "This checkout".to_owned();
    }
    match label.split_once("...") {
        Some((_, head)) if !head.is_empty() => head.to_owned(),
        _ => label.to_owned(),
    }
}

/// What the file's Prism files add: the core, then each language file, with
/// its tag and its hash in the security policy.
struct Code {
    /// The Prism names used, with what they need.
    names: BTreeSet<String>,
}

impl Code {
    fn bytes(&self, assets: &Assets) -> usize {
        if self.names.is_empty() {
            return 0;
        }
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        std::iter::once("core".to_owned())
            .chain(prism::files(&names))
            .filter_map(|name| assets.get(&format!("prism-{name}.js")))
            .map(|asset| asset.bytes.len() + 80)
            .sum()
    }
}

impl Work<'_> {
    /// The Prism name of a code block's language, when the program carries
    /// Prism's file for it.
    fn prism_name(&self, language: &str) -> Option<String> {
        prism::name(language, |name| {
            self.assets.get(&format!("prism-{name}.js")).is_some()
        })
    }

    /// One document as a page of the file, with id `id`; its links lead to
    /// the documents in `pages`.
    fn render(
        &self,
        revision: &Revision,
        label: &str,
        path: &RepoPath,
        id: &str,
        back: Option<(&str, &str)>,
        pages: &Pages,
    ) -> Result<Rendered, Problem> {
        let document = self.repo.document_at(revision, path)?;
        let path_text = path.to_lossy().into_owned();
        let commit = document
            .commit
            .as_ref()
            .map(|commit| commit.as_hex().to_owned());
        let at = pages::at_words(label, commit.as_deref());
        let links = Links::new(&path_text, pages);
        let (body, outline, readable, texts) = match document.parsed.as_ref() {
            Ok(parsed) => {
                let setup = Setup {
                    links: &links,
                    diagrams: &self.drawer,
                    allowed_html: &document.rules.rules.lint().allowed_html,
                };
                let reading = reading_prefixed(parsed, &setup, &format!("{id}-"));
                let texts = parsed
                    .blocks
                    .iter()
                    .map(|block| block.text.clone())
                    .collect();
                (reading.html, reading.outline, true, texts)
            }
            Err(error) => (
                format!(
                    "<p class=\"problem\">This document cannot be shown: {}.</p>\n",
                    escape(&error.to_string())
                ),
                Vec::new(),
                false,
                Vec::new(),
            ),
        };
        let parsed = document.parsed.as_ref().as_ref().ok();
        let fields = parsed.map_or_else(Vec::new, |parsed| parsed.meta.fields.clone());
        let html = pages::reading(&pages::ReadingPage {
            id,
            path: &path_text,
            at: &at,
            classification: document.classification.as_ref(),
            fields: &fields,
            outline: &outline,
            body: &body,
            readable,
            back,
        });
        Ok(Rendered {
            languages: languages(&body),
            path: path_text,
            title: parsed.and_then(|parsed| parsed.meta.title.clone()),
            summary: parsed.and_then(|parsed| parsed.meta.summary.clone()),
            classification: document.classification,
            fields,
            html,
            texts,
        })
    }

    /// The Prism languages of these pages, as written with each one's Prism
    /// name, and Prism's files to carry.
    fn code_of<'p>(
        &self,
        pages: impl IntoIterator<Item = &'p Rendered>,
    ) -> (Vec<(String, String)>, Vec<String>) {
        let mut written = BTreeSet::new();
        for page in pages {
            written.extend(page.languages.iter().cloned());
        }
        let named: Vec<(String, String)> = written
            .into_iter()
            .filter_map(|language| self.prism_name(&language).map(|name| (language, name)))
            .collect();
        let prism_names: Vec<&str> = named
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<BTreeSet<&str>>()
            .into_iter()
            .collect();
        let files = prism::files(&prism_names);
        (named, files)
    }

    /// What one more page costs the file besides its own HTML and index: the
    /// diagram font, if it is the first drawing, and Prism's files for the
    /// languages no earlier page used.
    fn extra_cost(&self, page: &Rendered, code: &mut Code, font: &mut bool) -> usize {
        let mut cost = 0;
        if !*font && file::draws(&page.html) {
            cost += file::font_bytes();
        }
        let before = code.bytes(&self.assets);
        let mut names = code.names.clone();
        names.extend(
            page.languages
                .iter()
                .filter_map(|language| self.prism_name(language)),
        );
        let after = Code { names }.bytes(&self.assets);
        cost + after - before
    }

    /// Takes a page's extras into account once it is in the file.
    fn take_extras(&self, page: &Rendered, code: &mut Code, font: &mut bool) {
        *font |= file::draws(&page.html);
        code.names.extend(
            page.languages
                .iter()
                .filter_map(|language| self.prism_name(language)),
        );
    }

    // ---- doc ----------------------------------------------------------------

    fn document(&self, revision: &Revision, path: &RepoPath) -> Result<Made, Problem> {
        let path_text = path.to_lossy().into_owned();
        let pages: Pages = [(path_text.clone(), "R1".to_owned())].into_iter().collect();
        let page = self.render(revision, revision.label(), path, "R1", None, &pages)?;
        let (languages, prism) = self.code_of([&page]);
        let title = page.title.clone().unwrap_or_else(|| path_text.clone());
        let html = file::file(
            &file::Parts {
                title: &title,
                kind: "doc",
                first: "R1",
                about: ABOUT,
                pages: &page.html,
                search: None,
                languages: &languages,
                prism: &prism,
            },
            &self.assets,
        );
        Ok(Made {
            html,
            documents: 1,
            cut: None,
        })
    }

    /// Picks the pages that fit, in the order given, each as long as it
    /// fits: its HTML, its words in the search index, and the font or
    /// Prism files it is the first to need. `used` is what the file holds
    /// already; returns what it holds after.
    fn choose<'p>(
        &self,
        candidates: impl IntoIterator<Item = (usize, &'p Rendered)>,
        used: usize,
        allowance: usize,
        index: &mut Index,
        chosen: &mut [bool],
    ) -> usize {
        let mut used = used;
        let (mut code, mut font) = (
            Code {
                names: BTreeSet::new(),
            },
            false,
        );
        for (at, page) in candidates {
            let entry = page.search_page(&format!("R{}", at + 1));
            let cost =
                page.html.len() + index.cost(&entry) + self.extra_cost(page, &mut code, &mut font);
            if used + cost <= allowance {
                used += cost;
                chosen[at] = true;
                index.insert(entry);
                self.take_extras(page, &mut code, &mut font);
            }
        }
        used
    }

    // ---- folder ---------------------------------------------------------------

    #[allow(clippy::too_many_lines, reason = "one export, read top to bottom")]
    fn folder(&self, revision: &Revision, label: &str, roots: &Roots) -> Result<Made, Problem> {
        // Every document is read at the commit the revision names now, even
        // if the branch moves while the export runs.
        let (snapshot, pinned) = match revision {
            Revision::Name(name) => {
                let commit = self.repo.git.resolve(name)?;
                (
                    Snapshot::Commit(commit.clone()),
                    Revision::Name(commit.as_hex().to_owned()),
                )
            }
            Revision::Worktree => (Snapshot::Worktree, Revision::Worktree),
        };
        let mut files = self.repo.git.markdown_files(&snapshot, roots)?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let numbered: Vec<(usize, RepoPath)> = files
            .into_iter()
            .map(|file| file.path)
            .enumerate()
            .collect();
        let pages_of = |kept: &dyn Fn(usize) -> bool| -> Pages {
            numbered
                .iter()
                .filter(|(at, _)| kept(*at))
                .map(|(at, path)| (path.to_lossy().into_owned(), format!("R{}", at + 1)))
                .collect()
        };
        let render_all = |pages: &Pages, which: &[(usize, RepoPath)]| {
            parallel(which, self.threads, |(at, path)| {
                self.render(
                    &pinned,
                    revision.label(),
                    path,
                    &format!("R{}", at + 1),
                    Some(("#Index", "Index")),
                    pages,
                )
            })
            .into_iter()
            .collect::<Result<Vec<Rendered>, Problem>>()
        };
        let rendered = render_all(&pages_of(&|_| true), &numbered)?;
        let heading = if label == "." {
            "Every document".to_owned()
        } else {
            label.to_owned()
        };
        let at = match &pinned {
            Revision::Name(commit) => pages::at_words(revision.label(), Some(commit)),
            Revision::Worktree => pages::at_words(revision.label(), None),
        };
        let index_page = |chosen: &[bool], cut: &str| {
            let listed: Vec<pages::Listed<'_>> = rendered
                .iter()
                .enumerate()
                .map(|(at, page)| pages::Listed {
                    path: &page.path,
                    title: page.title.as_deref(),
                    summary: page.summary.as_deref(),
                    classification: page.classification.as_ref(),
                    fields: &page.fields,
                    id: chosen[at].then(|| format!("R{}", at + 1)),
                })
                .collect();
            pages::index(&heading, &at, &listed, cut)
        };
        let all = vec![true; rendered.len()];
        let base = file::frame_bytes() + index_page(&all, &cut_note(&"x".repeat(300))).len();
        let mut allowance = self.limit;
        loop {
            let mut index = Index::default();
            let mut chosen = vec![false; rendered.len()];
            self.choose(
                rendered.iter().enumerate(),
                base,
                allowance,
                &mut index,
                &mut chosen,
            );
            let kept = chosen.iter().filter(|chosen| **chosen).count();
            // The kept documents are written again when some were left out,
            // so their links to those show as text.
            let again: Vec<Rendered>;
            let shown: Vec<&Rendered> = if kept == rendered.len() {
                rendered.iter().collect()
            } else {
                let which: Vec<(usize, RepoPath)> = numbered
                    .iter()
                    .filter(|(at, _)| chosen[*at])
                    .cloned()
                    .collect();
                again = render_all(&pages_of(&|at| chosen[at]), &which)?;
                again.iter().collect()
            };
            let cut = (kept < rendered.len()).then(|| {
                format!(
                    "This folder is too big for one file of at most {}. The file holds the text of {} of its {} documents; the index lists the others without their text.",
                    size_words(self.limit),
                    kept,
                    rendered.len()
                )
            });
            let mut body = index_page(&chosen, &cut.as_deref().map(cut_note).unwrap_or_default());
            for page in &shown {
                body.push_str(&page.html);
            }
            let (languages, prism) = self.code_of(shown.iter().copied());
            let search = (kept > 1).then(|| index.json());
            let title = format!("{heading} at {}", revision.label());
            let html = file::file(
                &file::Parts {
                    title: &title,
                    kind: "folder",
                    first: "Index",
                    about: ABOUT,
                    pages: &body,
                    search: search.as_deref(),
                    languages: &languages,
                    prism: &prism,
                },
                &self.assets,
            );
            if html.len() <= self.limit || kept == 0 {
                return Ok(Made {
                    html,
                    documents: kept,
                    cut,
                });
            }
            // Links written again, or the index, took more than counted:
            // try again with the difference held back.
            allowance = allowance.saturating_sub(html.len() - self.limit + 4096);
        }
    }

    // ---- changes --------------------------------------------------------------

    #[allow(clippy::too_many_lines, reason = "one export, read top to bottom")]
    fn changes(
        &self,
        label: &str,
        spec: &driftmark_core::git::RangeSpec,
        roots: &Roots,
    ) -> Result<Made, Problem> {
        let resolved = self.repo.resolve(&RangeAddress {
            label: label.to_owned(),
            spec: spec.clone(),
        })?;
        let count = self.repo.range_count(&resolved)?;
        let inside =
            |path: &str| RepoPath::new(path.as_bytes()).is_some_and(|path| roots.contains(&path));
        let paths: Vec<RepoPath> = count
            .list
            .documents
            .iter()
            .filter(|document| inside(&document.path))
            .filter_map(|document| RepoPath::new(document.path.as_bytes()))
            .collect();
        let compared = parallel(&paths, self.threads, |path| {
            let located = self.repo.locate(&resolved, path)?;
            let title = self.repo.title_of(&located);
            let changes = self.repo.document_changes(&resolved, located)?;
            Ok::<_, Problem>((
                changes.list.documents.into_iter().next(),
                title,
                changes.skipped,
            ))
        });
        let mut documents = Vec::new();
        let mut titles: HashMap<String, String> = HashMap::new();
        let mut skipped: Vec<Skipped> = count
            .skipped
            .iter()
            .filter(|file| inside(&file.path))
            .cloned()
            .collect();
        for outcome in compared {
            let (document, title, more) = outcome?;
            skipped.extend(more);
            if let Some(document) = document {
                if let Some(title) = title {
                    titles.insert(document.path.clone(), title);
                }
                documents.push(document);
            }
        }
        let info = RangeInfo::of(&resolved);
        let list = ChangeList::new(Some(info.clone()), documents);
        let (head, head_words) = match &resolved.head {
            Snapshot::Commit(id) => (
                Revision::Name(id.as_hex().to_owned()),
                format!("<code>{}</code>", short(id.as_hex())),
            ),
            Snapshot::Worktree => (Revision::Worktree, "the working tree".to_owned()),
        };
        let mut compared_words = match &info.base {
            Some(base) => format!("from <code>{}</code> to {head_words}", short(base)),
            None => format!("from an empty history to {head_words}"),
        };
        if !info.notes.is_empty() {
            compared_words.push_str(" · ");
            compared_words.push_str(&escape(&info.notes.join("; ")));
        }
        if !roots.folders().is_empty() {
            let folders: Vec<String> = roots
                .folders()
                .iter()
                .map(|folder| format!("<code>{}</code>", escape(&folder.to_lossy())))
                .collect();
            let _ = write!(compared_words, " · only in {}", folders.join(", "));
        }
        let heading = range_heading(label);
        let n = list.documents.len();
        let cards: Vec<String> = list
            .documents
            .iter()
            .enumerate()
            .map(|(at, document)| {
                driftmark_core::render::changes(document, &format!("C{}", at + 1))
                    .replace(pages::CARD_BUTTONS, "")
            })
            .collect();
        let card_pages: Vec<search::Page> = list
            .documents
            .iter()
            .enumerate()
            .map(|(at, document)| {
                let title = titles
                    .get(&document.path)
                    .map_or(document.path.as_str(), String::as_str);
                search::page(
                    &format!("D{}", at + 1),
                    title,
                    &document.path,
                    Kind::Changes,
                    change_texts(document).iter().map(String::as_str),
                )
            })
            .collect();
        // The documents whose newer side the file can show whole.
        let readable: Vec<(usize, RepoPath)> = list
            .documents
            .iter()
            .enumerate()
            .filter(|(_, document)| document.file != FileState::Removed)
            .filter_map(|(at, document)| {
                RepoPath::new(document.path.as_bytes()).map(|path| (at, path))
            })
            .collect();
        let pages_of = |kept: &dyn Fn(usize) -> bool| -> Pages {
            readable
                .iter()
                .filter(|(at, _)| kept(*at))
                .map(|(at, path)| (path.to_lossy().into_owned(), format!("R{}", at + 1)))
                .collect()
        };
        let render_reading = |pages: &Pages, which: &[(usize, RepoPath)]| {
            parallel(which, self.threads, |(at, path)| {
                let back = format!("#D{}", at + 1);
                self.render(
                    &head,
                    head.label(),
                    path,
                    &format!("R{}", at + 1),
                    Some((&back, "its changes")),
                    pages,
                )
                .map(|page| (*at, page))
            })
            .into_iter()
            .collect::<Result<Vec<(usize, Rendered)>, Problem>>()
        };
        let shell = |chosen: &[bool], reading: &[bool], cut: &str| {
            let changed: Vec<pages::Changed<'_>> = list
                .documents
                .iter()
                .enumerate()
                .map(|(at, document)| pages::Changed {
                    document,
                    number: at + 1,
                    title: titles.get(&document.path).map(String::as_str),
                    cards: chosen[at].then(|| cards[at].as_str()),
                    reading: reading[at],
                })
                .collect();
            pages::changes(&pages::ChangesPage {
                label,
                heading: &heading,
                compared: &compared_words,
                list: &list,
                documents: &changed,
                skipped: &skipped,
                cut,
            })
        };
        let none = vec![false; n];
        let base = file::frame_bytes() + shell(&none, &none, &cut_note(&"x".repeat(300))).len();
        // Every document's cards, highest ranked first, while they fit: the
        // changes are what the file is for. What one adds to the rail is
        // counted roughly; the check after the file is written corrects it.
        let take_cards = |allowance: usize, index: &mut Index, chosen: &mut [bool]| {
            let mut used = base;
            for at in 0..n {
                let rail = 250 + 260 * list.documents[at].changes.len().min(80);
                let cost = cards[at].len() + 600 + rail + index.cost(&card_pages[at]);
                if used + cost > allowance {
                    break;
                }
                used += cost;
                chosen[at] = true;
                index.insert(card_pages[at].clone());
            }
            used
        };
        // The full text comes only when every change fits; each document is
        // read once whole to measure it.
        let every_reading = {
            let mut chosen = vec![false; n];
            take_cards(self.limit, &mut Index::default(), &mut chosen);
            if chosen.iter().all(|chosen| *chosen) {
                render_reading(&pages_of(&|_| true), &readable)?
            } else {
                Vec::new()
            }
        };
        let mut allowance = self.limit;
        loop {
            let mut index = Index::default();
            let mut chosen = vec![false; n];
            let used = take_cards(allowance, &mut index, &mut chosen);
            let all_cards = chosen.iter().all(|chosen| *chosen);
            let mut reading = vec![false; n];
            if all_cards {
                self.choose(
                    every_reading.iter().map(|(at, page)| (*at, page)),
                    used,
                    allowance,
                    &mut index,
                    &mut reading,
                );
            }
            let kept = reading.iter().filter(|kept| **kept).count();
            let again: Vec<(usize, Rendered)>;
            let shown: Vec<&Rendered> = if kept == every_reading.len() {
                every_reading.iter().map(|(_, page)| page).collect()
            } else {
                let which: Vec<(usize, RepoPath)> = readable
                    .iter()
                    .filter(|(at, _)| reading[*at])
                    .cloned()
                    .collect();
                again = render_reading(&pages_of(&|at| reading[at]), &which)?;
                again.iter().map(|(_, page)| page).collect()
            };
            let shown_cards = chosen.iter().filter(|chosen| **chosen).count();
            let cut = if !all_cards {
                Some(format!(
                    "This range is too big for one file of at most {}. The file holds the changes of the {} highest-ranked of its {} documents, without their full text; the other {} are listed at the end with their counts.",
                    size_words(self.limit),
                    shown_cards,
                    n,
                    n - shown_cards
                ))
            } else if kept < readable.len() {
                Some(format!(
                    "This file holds every change of this range, and the full text of {} of the {} documents it changed, highest ranked first. The others did not fit in one file of at most {}.",
                    kept,
                    readable.len(),
                    size_words(self.limit)
                ))
            } else {
                None
            };
            let mut body = shell(
                &chosen,
                &reading,
                &cut.as_deref().map(cut_note).unwrap_or_default(),
            );
            for page in &shown {
                body.push_str(&page.html);
            }
            let (languages, prism) = self.code_of(shown.iter().copied());
            let search = (shown_cards > 1).then(|| index.json());
            let html = file::file(
                &file::Parts {
                    title: &heading,
                    kind: "changes",
                    first: "Changes",
                    about: ABOUT,
                    pages: &body,
                    search: search.as_deref(),
                    languages: &languages,
                    prism: &prism,
                },
                &self.assets,
            );
            if html.len() <= self.limit || shown_cards == 0 {
                return Ok(Made {
                    html,
                    documents: shown_cards,
                    cut,
                });
            }
            allowance = allowance.saturating_sub(html.len() - self.limit + 4096);
        }
    }
}

/// The text a document's change cards show, for the search index: each
/// changed block's text on both sides, or a whole document's title and
/// summary.
fn change_texts(document: &DocumentChanges) -> Vec<String> {
    let mut texts = Vec::new();
    for change in &document.changes {
        match &change.subject {
            Subject::Block { old, new } => {
                texts.extend(old.block.as_ref().map(|block| block.text.clone()));
                texts.extend(new.block.as_ref().map(|block| block.text.clone()));
            }
            Subject::Document { document } => {
                texts.extend(document.title.clone());
                texts.extend(document.summary.clone());
            }
        }
    }
    texts
}

#[cfg(test)]
mod tests;
