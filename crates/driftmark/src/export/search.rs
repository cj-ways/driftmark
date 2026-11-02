//! The search index an exported file carries, built when the file is
//! written.
//!
//! The file cannot ask a server, and building an index in the browser from
//! thousands of pages would make the first search slow. So the program
//! writes, for every word of every page, which pages hold it. Words are split
//! and folded by `driftmark_core::search::terms`, as the server's search
//! does. The page script (`assets/export.js`) finds the pages that hold
//! every word of a query in this index, then reads only those pages' text
//! to check phrases, rank them and cut a snippet.
//!
//! The index is JSON, kept small:
//!
//! - `pages`: one `[id, title, path, kind]` per page, `kind` being `read`
//!   for a document's reading page or `changes` for its change cards;
//! - `words`: every word, sorted;
//! - `in`: for each word, the numbers of the pages that hold it, as the
//!   gaps between them in base 36, joined by `.`: pages 0, 3 and 40 are
//!   `0.3.11`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;

/// What a searchable page of the file shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A document's reading page.
    Reading,
    /// A document's change cards.
    Changes,
}

/// The index, while the file is written. It knows the size of its JSON at
/// every step, so the export can stop adding pages before the file passes
/// its size limit.
#[derive(Debug, Default)]
pub(crate) struct Index {
    pages: Vec<[String; 4]>,
    words: BTreeMap<String, Vec<u32>>,
    /// The bytes of the pages' entries, as JSON, without commas between.
    pages_bytes: usize,
    /// The bytes of the words, as JSON strings.
    words_bytes: usize,
    /// The bytes of the page lists, without their quotes.
    lists_bytes: usize,
}

/// One page, ready to go into an [`Index`].
#[derive(Debug, Clone)]
pub(crate) struct Page {
    entry: [String; 4],
    terms: BTreeSet<String>,
}

#[derive(Serialize)]
struct Json<'a> {
    v: u32,
    pages: &'a [[String; 4]],
    words: Vec<&'a str>,
    #[serde(rename = "in")]
    pages_of: Vec<String>,
}

/// The JSON of an empty index.
#[cfg(test)]
const EMPTY: &str = r#"{"v":1,"pages":[],"words":[],"in":[]}"#;

/// A page of the file: its element id, its title, the document's path, what
/// it shows, and its text, in any number of pieces.
pub(crate) fn page<'t>(
    id: &str,
    title: &str,
    path: &str,
    kind: Kind,
    texts: impl IntoIterator<Item = &'t str>,
) -> Page {
    let kind = match kind {
        Kind::Reading => "read",
        Kind::Changes => "changes",
    };
    let mut terms = BTreeSet::new();
    for text in texts {
        terms.extend(driftmark_core::search::terms(text));
    }
    Page {
        entry: [
            id.to_owned(),
            title.to_owned(),
            path.to_owned(),
            kind.to_owned(),
        ],
        terms,
    }
}

/// The length of a text as a JSON string inside a `<script>` element.
fn json_len(text: &str) -> usize {
    serde_json::to_string(text).map_or(0, |json| json.len()) + 5 * text.matches('<').count()
}

impl Index {
    /// How many bytes `page` would add to the JSON.
    pub(crate) fn cost(&self, page: &Page) -> usize {
        let number = self.next();
        let mut cost = page.entry.iter().map(|part| json_len(part)).sum::<usize>()
            + 5
            + usize::from(!self.pages.is_empty());
        let mut words = self.words.len();
        for term in &page.terms {
            if let Some(pages) = self.words.get(term) {
                cost += 1 + base36_len(number - pages.last().copied().unwrap_or(0));
            } else {
                // The word, its list, and a comma before each when the
                // arrays already hold something.
                cost += json_len(term) + 2 + base36_len(number) + 2 * usize::from(words > 0);
                words += 1;
            }
        }
        cost
    }

    /// Puts `page` into the index.
    pub(crate) fn insert(&mut self, page: Page) {
        let number = self.next();
        self.pages_bytes += page.entry.iter().map(|part| json_len(part)).sum::<usize>() + 5;
        self.pages.push(page.entry);
        for term in page.terms {
            let words_bytes = json_len(&term);
            let pages = self.words.entry(term).or_default();
            if let Some(last) = pages.last() {
                self.lists_bytes += 1 + base36_len(number - last);
            } else {
                self.words_bytes += words_bytes;
                self.lists_bytes += base36_len(number);
            }
            pages.push(number);
        }
    }

    /// Adds a page, as [`page`] makes it.
    #[cfg(test)]
    pub(crate) fn add<'t>(
        &mut self,
        id: &str,
        title: &str,
        path: &str,
        kind: Kind,
        texts: impl IntoIterator<Item = &'t str>,
    ) {
        self.insert(page(id, title, path, kind, texts));
    }

    fn next(&self) -> u32 {
        u32::try_from(self.pages.len()).unwrap_or(u32::MAX)
    }

    /// How many pages it holds.
    #[cfg(test)]
    pub(crate) fn pages(&self) -> usize {
        self.pages.len()
    }

    /// The length of [`Index::json`], in bytes.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        let words = self.words.len();
        EMPTY.len()
            + self.pages_bytes
            + self.pages.len().saturating_sub(1)
            + self.words_bytes
            + self.lists_bytes
            + 2 * words
            + 2 * words.saturating_sub(1)
    }

    /// The index as JSON that can sit inside a `<script>` element: every `<`
    /// is written `\u003c`, so no text closes the element.
    pub(crate) fn json(&self) -> String {
        let json = Json {
            v: 1,
            pages: &self.pages,
            words: self.words.keys().map(String::as_str).collect(),
            pages_of: self.words.values().map(|pages| gaps(pages)).collect(),
        };
        serde_json::to_string(&json)
            .unwrap_or_default()
            .replace('<', "\\u003c")
    }
}

/// How many base-36 digits a number takes.
fn base36_len(mut n: u32) -> usize {
    let mut digits = 1;
    while n >= 36 {
        n /= 36;
        digits += 1;
    }
    digits
}

/// Page numbers, rising, as the gaps between them in base 36 joined by `.`.
fn gaps(pages: &[u32]) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (at, page) in pages.iter().enumerate() {
        if at > 0 {
            out.push('.');
        }
        base36(page - last, &mut out);
        last = *page;
    }
    out
}

fn base36(mut n: u32, out: &mut String) {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut digits = Vec::new();
    loop {
        digits.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    for digit in digits.iter().rev() {
        let _ = write!(out, "{}", char::from(*digit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_word_lists_the_pages_that_hold_it() {
        let mut index = Index::default();
        index.add(
            "R1",
            "Cache",
            "docs/a.md",
            Kind::Reading,
            ["# Cache", "Keep pages for one hour."],
        );
        index.add(
            "D1",
            "Cache",
            "docs/a.md",
            Kind::Changes,
            ["Keep PAGES <b>longer</b>"],
        );
        assert_eq!(index.pages(), 2);
        assert_eq!(
            index.json(),
            r#"{"v":1,"pages":[["R1","Cache","docs/a.md","read"],["D1","Cache","docs/a.md","changes"]],"words":["b","cache","for","hour","keep","longer","one","pages"],"in":["1","0","0","0","0.1","1","0","0.1"]}"#
        );
    }

    #[test]
    fn the_size_is_known_before_and_after_each_page() {
        let mut index = Index::default();
        assert_eq!(index.bytes(), index.json().len());
        let texts = [
            vec!["# Cache", "Keep pages for one hour."],
            vec!["Pages <b>stay</b> \"quoted\" for hours."],
            vec![],
            vec!["Cache cache CACHE ქართული ტექსტი"],
        ];
        for round in 0..40 {
            for (at, text) in texts.iter().enumerate() {
                let next = page(
                    &format!("R{round}-{at}"),
                    "T <i>",
                    "docs/a.md",
                    Kind::Reading,
                    text.iter().copied(),
                );
                let before = index.bytes();
                let cost = index.cost(&next);
                index.insert(next);
                assert_eq!(
                    index.bytes(),
                    index.json().len(),
                    "round {round}, page {at}"
                );
                assert_eq!(index.bytes() - before, cost, "round {round}, page {at}");
            }
        }
        assert_eq!(index.pages(), 160);
    }

    #[test]
    fn page_numbers_are_gaps_in_base_36() {
        assert_eq!(gaps(&[0, 3, 40]), "0.3.11");
        assert_eq!(gaps(&[35, 36, 1332]), "z.1.100");
        assert_eq!(gaps(&[]), "");
        for n in [0, 35, 36, 1295, 1296, u32::MAX] {
            let mut text = String::new();
            base36(n, &mut text);
            assert_eq!(base36_len(n), text.len(), "{n}");
        }
    }

    #[test]
    fn no_text_can_close_the_script_element() {
        let mut index = Index::default();
        index.add("R1", "</script><b>", "a.md", Kind::Reading, ["x"]);
        let json = index.json();
        assert!(!json.contains('<'), "{json}");
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["pages"][0][1], "</script><b>");
    }
}
