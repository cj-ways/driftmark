//! The `#anchors` a document's headings get, as GitHub makes them.

use std::collections::BTreeSet;

use comrak::Anchorizer;

use crate::document::{BlockKind, Document};

/// The anchors of one document's headings.
///
/// GitHub's rule, which comrak follows: the heading's text in lower case,
/// with every character dropped that is not a letter, a digit, a mark, `-`,
/// `_` or a space, and each space turned into `-`. A second heading with the
/// same anchor gets `-1`, a third `-2`, and so on. So `## Cache: one hour`
/// gets `#cache-one-hour`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anchors {
    ids: BTreeSet<String>,
}

impl Anchors {
    /// The anchors of every heading in `document`, inside containers too,
    /// because GitHub gives those headings anchors as well.
    #[must_use]
    pub fn of(document: &Document) -> Anchors {
        let mut anchorizer = Anchorizer::new();
        let ids = document
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Heading { .. }))
            .map(|block| anchorizer.anchorize(&block.text))
            .collect();
        Anchors { ids }
    }

    /// Whether a heading has this anchor, without the `#`.
    #[must_use]
    pub fn contains(&self, anchor: &str) -> bool {
        self.ids.contains(anchor)
    }

    /// The anchor that matches `anchor` when case is ignored, if there is
    /// one. Anchors are lower case, so a link written in another case finds
    /// its heading this way.
    #[must_use]
    pub fn matching_case(&self, anchor: &str) -> Option<&str> {
        let wanted = anchor.to_lowercase();
        self.ids.get(&wanted).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::Anchors;
    use crate::document::Document;

    fn anchors(source: &str) -> Anchors {
        Anchors::of(&Document::parse(source.as_bytes().to_vec()).expect("parses"))
    }

    #[test]
    fn anchors_follow_github() {
        let found = anchors(
            "# Cache: one hour\n\n## `Code` and *emphasis*\n\n## Repeat\n\n## Repeat\n\n> ### In a quote\n\n## Ünïcode ჯგუფი 1.2\n\nText.\n",
        );
        for anchor in [
            "cache-one-hour",
            "code-and-emphasis",
            "repeat",
            "repeat-1",
            "in-a-quote",
            "ünïcode-ჯგუფი-12",
        ] {
            assert!(found.contains(anchor), "{anchor}");
        }
        assert!(!found.contains("text"));
        assert!(!found.contains("Repeat"));
    }

    #[test]
    fn a_link_in_another_case_finds_its_heading() {
        let found = anchors("# Title\n\n## Decision\n");
        assert_eq!(found.matching_case("Decision"), Some("decision"));
        assert_eq!(found.matching_case("Choice"), None);
        assert_eq!(Anchors::default().matching_case("x"), None);
    }
}
