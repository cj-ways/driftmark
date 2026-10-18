//! What a reader types into the search box, read into clauses.

use std::fmt;

use super::words::{fold, words};

/// The most words a query may hold, counting the words inside phrases.
/// Each word costs one pass over its part of the index, so the limit keeps
/// one request from holding the server up.
pub const MAX_QUERY_WORDS: usize = 32;

/// One word of a clause, folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    /// The word, in the form words are compared in.
    pub(crate) word: String,
    /// Whether any word that starts with `word` matches: the last word of a
    /// query while the reader is still typing it.
    pub(crate) prefix: bool,
}

impl Slot {
    /// Whether a folded word from a document fills this slot.
    pub(crate) fn matches(&self, term: &str) -> bool {
        if self.prefix {
            term.starts_with(&self.word)
        } else {
            term == self.word
        }
    }
}

/// Words that must stand next to each other, in this order. A single word is
/// a clause of one slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Clause {
    /// The words in order.
    pub(crate) slots: Vec<Slot>,
}

/// A parsed query: clauses that a document must all contain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub(crate) clauses: Vec<Clause>,
}

/// Why a query cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// The query holds more than [`MAX_QUERY_WORDS`] words.
    TooManyWords {
        /// How many words it holds.
        words: usize,
    },
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyWords { words } => write!(
                f,
                "the query holds {words} words; a query holds at most {MAX_QUERY_WORDS}"
            ),
        }
    }
}

impl std::error::Error for QueryError {}

/// The characters that open and close a phrase. Phones type `“` and `”` in
/// place of `"`, so those count too, and so does the German `„`.
fn is_quote(character: char) -> bool {
    matches!(character, '"' | '“' | '”' | '„')
}

impl Query {
    /// Reads a query.
    ///
    /// - Words are found and folded as in documents, so case does not
    ///   matter.
    /// - Text between quotes is a phrase: its words must stand next to each
    ///   other, in order, in one block. A quote that is never closed runs to
    ///   the end.
    /// - Outside quotes, text splits at spaces into pieces. A piece that
    ///   holds several words, such as `read-only` or `2026-10-18`, is a
    ///   phrase too.
    /// - The last word is a prefix when nothing follows it, not even a space
    ///   or a closing quote: `cach` finds `cache` and `caching` while the
    ///   reader types.
    /// - A clause written twice counts once.
    ///
    /// # Errors
    ///
    /// [`QueryError::TooManyWords`] when the query holds more than
    /// [`MAX_QUERY_WORDS`] words.
    pub fn parse(text: &str) -> Result<Query, QueryError> {
        let mut clauses: Vec<Clause> = Vec::new();
        let mut count = 0;
        let mut rest = text;
        while let Some(first) = rest.chars().next() {
            // `typing`: nothing follows the piece, not even a closing quote.
            let (piece, after, typing) = if first.is_whitespace() {
                rest = &rest[first.len_utf8()..];
                continue;
            } else if is_quote(first) {
                let inner = &rest[first.len_utf8()..];
                match inner.char_indices().find(|&(_, c)| is_quote(c)) {
                    Some((end, close)) => (&inner[..end], &inner[end + close.len_utf8()..], false),
                    None => (inner, "", true),
                }
            } else {
                let end = rest
                    .find(|c: char| c.is_whitespace() || is_quote(c))
                    .unwrap_or(rest.len());
                (&rest[..end], &rest[end..], end == rest.len())
            };
            let found = words(piece);
            let last_end = found.last().map(|&(at, word)| at + word.len());
            let open_end = typing && last_end == Some(piece.len());
            let total = found.len();
            let slots: Vec<Slot> = found
                .into_iter()
                .enumerate()
                .map(|(at, (_, word))| Slot {
                    word: fold(word),
                    prefix: open_end && at + 1 == total,
                })
                .collect();
            count += slots.len();
            let clause = Clause { slots };
            if !clause.slots.is_empty() && !clauses.contains(&clause) {
                clauses.push(clause);
            }
            rest = after;
        }
        if count > MAX_QUERY_WORDS {
            return Err(QueryError::TooManyWords { words: count });
        }
        Ok(Query { clauses })
    }

    /// Whether the query holds no words, so it matches nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clauses written out: `+` joins the words of a phrase, `*` marks a
    /// prefix.
    fn read(text: &str) -> Vec<String> {
        Query::parse(text)
            .unwrap()
            .clauses
            .iter()
            .map(|clause| {
                clause
                    .slots
                    .iter()
                    .map(|slot| format!("{}{}", slot.word, if slot.prefix { "*" } else { "" }))
                    .collect::<Vec<_>>()
                    .join("+")
            })
            .collect()
    }

    #[test]
    fn words_split_at_spaces_and_the_last_one_is_a_prefix() {
        assert_eq!(read("Cache Pag"), ["cache", "pag*"]);
        assert_eq!(read("cache pag "), ["cache", "pag"]);
        assert_eq!(read("cache pag."), ["cache", "pag"]);
        assert_eq!(read("\tcache\n"), ["cache"]);
        assert_eq!(read(""), Vec::<String>::new());
        assert!(Query::parse("  ✅ — ").unwrap().is_empty());
        assert!(!Query::parse("a").unwrap().is_empty());
    }

    #[test]
    fn quotes_make_phrases() {
        assert_eq!(read("\"One hour\" cache"), ["one+hour", "cache*"]);
        assert_eq!(read("cache \"one hour\""), ["cache", "one+hour"]);
        assert_eq!(read("cache \"one ho"), ["cache", "one+ho*"]);
        assert_eq!(read("cache \"one ho "), ["cache", "one+ho"]);
        assert_eq!(read("“smart quotes” work"), ["smart+quotes", "work*"]);
        assert_eq!(read("„low quote“"), ["low+quote"]);
        assert_eq!(read("a\"b c\"d"), ["a", "b+c", "d*"]);
        assert_eq!(read("\"\" x"), ["x*"]);
    }

    #[test]
    fn a_piece_with_several_words_is_a_phrase() {
        assert_eq!(read("read-only mode"), ["read+only", "mode*"]);
        assert_eq!(read("2026-10-18"), ["2026+10+18*"]);
        assert_eq!(read("driftmark-co"), ["driftmark+co*"]);
        assert_eq!(read("ქართული ტექსტ"), ["ქართული", "ტექსტ*"]);
        assert_eq!(read("文章"), ["文+章*"]);
    }

    #[test]
    fn a_clause_written_twice_counts_once() {
        assert_eq!(read("cache CACHE cache"), ["cache", "cache*"]);
        assert_eq!(read("cache CACHE cache "), ["cache"]);
    }

    #[test]
    fn slots_match_whole_words_or_prefixes() {
        let whole = Slot {
            word: "cache".into(),
            prefix: false,
        };
        assert!(whole.matches("cache"));
        assert!(!whole.matches("caches"));
        let prefix = Slot {
            word: "cache".into(),
            prefix: true,
        };
        assert!(prefix.matches("caches"));
        assert!(prefix.matches("cache"));
        assert!(!prefix.matches("cach"));
    }

    #[test]
    fn a_query_holds_at_most_32_words() {
        let words = vec!["w"; MAX_QUERY_WORDS].join(" ");
        assert!(Query::parse(&words).is_ok());
        let phrase = format!("\"{words}\" more");
        let error = Query::parse(&phrase).unwrap_err();
        assert_eq!(error, QueryError::TooManyWords { words: 33 });
        assert_eq!(
            error.to_string(),
            "the query holds 33 words; a query holds at most 32"
        );
        let as_error: &dyn std::error::Error = &error;
        assert!(as_error.source().is_none());
    }
}
