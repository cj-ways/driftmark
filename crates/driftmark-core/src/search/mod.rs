//! Full-text search over the documents of one revision.
//!
//! The program parses every document of a revision and hands each one to a
//! [`Shard`]; [`Index::from_shards`] joins the shards into an [`Index`].
//! [`Query::parse`] reads what the reader typed, and [`search`] returns the
//! best documents with the block that matches best. [`Query::snippet`] and
//! [`place`] then describe that block for the result list. Nothing here
//! reads a file: the program supplies the documents, and parses a matching
//! document again when a result needs its snippet.
//!
//! A plain in-memory index is enough for a few thousand documents: for each
//! word, the documents that hold it and where. `docs/features/engine/search.md`
//! states the query syntax, the ranking, the snippet rules and the limits.

mod find;
mod index;
mod query;
mod snippet;
mod words;

pub use find::{Filter, Found, Hit, search};
pub use index::{Entry, Index, Shard};
pub use query::{MAX_QUERY_WORDS, Query, QueryError};
pub use snippet::{CUT, LEAD_CHARS, Piece, Place, SNIPPET_CHARS, place};
pub use words::{MAX_WORD_BYTES, terms};
