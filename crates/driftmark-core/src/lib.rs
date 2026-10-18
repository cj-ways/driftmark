//! The rules of driftmark: how a Markdown document splits into blocks, how two
//! versions of a folder are compared and ranked, how a note stays attached to its
//! sentence, which writing rules the lint checks, and how the documents are
//! searched.
//!
//! This crate reads no files, runs no git and opens no network connection. The
//! `driftmark` program supplies all of that, so every rule here can be tested
//! with plain values.

pub mod diff;
pub mod document;
pub mod git;
pub mod lint;
pub mod render;
pub mod rules;
pub mod search;

/// The version of the driftmark engine, as written in `Cargo.toml`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_matches_the_package() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
