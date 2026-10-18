//! A document's outline: its sections, each opened by a heading outside any
//! container, with the blocks and source bytes it covers.
//!
//! A tool that reads one part of a long document asks for a section instead
//! of the whole text. The MCP connector's `document_read` does, and a note
//! shows the section around its quote the same way.

use std::ops::Range;

use super::{BlockKind, Document};
use crate::lint::Anchors;

/// One section: a heading outside any container, and everything after it up
/// to the next heading of the same level or a higher one.
///
/// Sections nest: a `##` section holds the `###` sections under it. A
/// heading inside a quote, a list item, an alert or a footnote opens no
/// section, as in the heading paths of the document model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The heading's block index.
    pub block: usize,
    /// The heading's level, 1 to 6.
    pub level: u8,
    /// The heading's plain text.
    pub title: String,
    /// The titles of the headings above it, outermost first, then its own.
    pub path: Vec<String>,
    /// The heading's anchor, as GitHub and the reading page make it, without
    /// `#`.
    pub anchor: String,
    /// The blocks it covers: the heading and every block before the next
    /// heading outside any container whose level is the same or higher.
    pub blocks: Range<usize>,
    /// Its source bytes: from the heading's first byte up to that next
    /// heading, without the white space in between.
    pub bytes: Range<usize>,
}

/// The sections of one document, in reading order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    /// The sections, in the order their headings stand. A section comes
    /// before the sections it holds.
    pub sections: Vec<Section>,
    /// The anchor of every heading block, containers included, by block
    /// index; `None` for every other block.
    anchors: Vec<Option<String>>,
}

impl Outline {
    /// The outline of `document`.
    #[must_use]
    pub fn of(document: &Document) -> Outline {
        let anchors = Anchors::by_block(document);
        let opens = |at: usize| {
            let block = &document.blocks[at];
            match block.kind {
                BlockKind::Heading { level } if block.containers.is_empty() => Some(level),
                _ => None,
            }
        };
        let mut sections = Vec::new();
        for (at, block) in document.blocks.iter().enumerate() {
            let Some(level) = opens(at) else {
                continue;
            };
            let end = (at + 1..document.blocks.len())
                .find(|&next| opens(next).is_some_and(|next| next <= level))
                .unwrap_or(document.blocks.len());
            let limit = document
                .blocks
                .get(end)
                .map_or(document.source.len(), |next| next.range.start);
            let text = &document.source[block.range.start..limit];
            let mut path = block.heading_path.to_vec();
            path.push(block.text.clone());
            sections.push(Section {
                block: at,
                level,
                title: block.text.clone(),
                path,
                anchor: anchors[at].clone().unwrap_or_default(),
                blocks: at..end,
                bytes: block.range.start..block.range.start + text.trim_end().len(),
            });
        }
        Outline { sections, anchors }
    }

    /// The innermost section that holds block `block`, or `None` for a block
    /// before the first heading, and for an index past the end.
    #[must_use]
    pub fn containing(&self, block: usize) -> Option<&Section> {
        self.sections
            .iter()
            .rev()
            .find(|section| section.blocks.contains(&block))
    }

    /// The section a heading's anchor names, without `#`: the heading's own
    /// section, or, for a heading inside a container, the section around it.
    /// An anchor written in another case finds its heading when no heading
    /// has it exactly, because anchors are lower case.
    #[must_use]
    pub fn find(&self, anchor: &str) -> Option<&Section> {
        let at = |wanted: &str| {
            self.anchors
                .iter()
                .position(|id| id.as_deref() == Some(wanted))
        };
        let block = at(anchor).or_else(|| at(&anchor.to_lowercase()))?;
        self.containing(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &str) -> (Document, Outline) {
        let document = Document::parse(source.as_bytes().to_vec()).expect("parses");
        let outline = Outline::of(&document);
        (document, outline)
    }

    /// Each section as `anchor level blocks: text`, to compare in one go.
    fn shown(document: &Document, outline: &Outline) -> Vec<String> {
        outline
            .sections
            .iter()
            .map(|section| {
                format!(
                    "{} {} {}..{} {:?} {:?}",
                    section.anchor,
                    section.level,
                    section.blocks.start,
                    section.blocks.end,
                    section.path,
                    &document.source[section.bytes.clone()]
                )
            })
            .collect()
    }

    #[test]
    fn sections_nest_by_level_and_end_at_the_next_heading_of_their_level() {
        let (document, outline) = read(
            "---\nstatus: accepted\n---\n\n# Cache\n\nIntro.\n\n## Decision\n\nKeep it.\n\n### Detail\n\nSmall.\n\n## Notes\n\nLast.  \n\n\n",
        );
        assert_eq!(
            shown(&document, &outline),
            [
                "cache 1 1..9 [\"Cache\"] \"# Cache\\n\\nIntro.\\n\\n## Decision\\n\\nKeep it.\\n\\n### Detail\\n\\nSmall.\\n\\n## Notes\\n\\nLast.\"",
                "decision 2 3..7 [\"Cache\", \"Decision\"] \"## Decision\\n\\nKeep it.\\n\\n### Detail\\n\\nSmall.\"",
                "detail 3 5..7 [\"Cache\", \"Decision\", \"Detail\"] \"### Detail\\n\\nSmall.\"",
                "notes 2 7..9 [\"Cache\", \"Notes\"] \"## Notes\\n\\nLast.\"",
            ]
        );
        let decision = &outline.sections[1];
        assert_eq!((decision.block, decision.level), (3, 2));
        assert_eq!(decision.title, "Decision");
    }

    #[test]
    fn a_skipped_level_still_nests_and_a_higher_heading_ends_it() {
        let (document, outline) = read("# A\n\n### Deep\n\nText.\n\n## B\n\nMore.\n");
        assert_eq!(
            shown(&document, &outline),
            [
                "a 1 0..5 [\"A\"] \"# A\\n\\n### Deep\\n\\nText.\\n\\n## B\\n\\nMore.\"",
                "deep 3 1..3 [\"A\", \"Deep\"] \"### Deep\\n\\nText.\"",
                "b 2 3..5 [\"A\", \"B\"] \"## B\\n\\nMore.\"",
            ]
        );
    }

    #[test]
    fn headings_in_containers_open_no_section_but_their_anchor_finds_the_one_around_them() {
        let (_, outline) =
            read("# Guide\n\n## Steps\n\n> ### Quoted\n>\n> Text.\n\n- ## In a list\n");
        let anchors: Vec<&str> = outline
            .sections
            .iter()
            .map(|section| section.anchor.as_str())
            .collect();
        assert_eq!(anchors, ["guide", "steps"]);
        assert_eq!(outline.find("quoted").map(|s| s.block), Some(1));
        assert_eq!(outline.find("in-a-list").map(|s| s.block), Some(1));
        assert_eq!(outline.find("steps").map(|s| s.block), Some(1));
    }

    #[test]
    fn anchors_follow_github_and_match_in_another_case() {
        let (_, outline) = read("# Plan\n\n## Repeat\n\nOne.\n\n## Repeat\n\nTwo.\n");
        let repeat = outline.find("repeat-1").expect("the second heading");
        assert_eq!((repeat.block, repeat.anchor.as_str()), (3, "repeat-1"));
        assert_eq!(outline.find("Repeat").map(|s| s.block), Some(1));
        assert_eq!(outline.find("REPEAT-1").map(|s| s.block), Some(3));
        assert_eq!(outline.find("missing"), None);
        assert_eq!(outline.find(""), None);
    }

    #[test]
    fn the_exact_anchor_wins_over_one_in_another_case() {
        // `## A` gets `a` and `## a` gets `a-1`; asking for `a-1` in upper
        // case finds the second heading, and `a` finds the first.
        let (_, outline) = read("## A\n\n## a\n");
        assert_eq!(outline.find("a").map(|s| s.block), Some(0));
        assert_eq!(outline.find("A-1").map(|s| s.block), Some(1));
    }

    #[test]
    fn blocks_before_the_first_heading_belong_to_no_section() {
        let (_, outline) = read("---\nk: v\n---\n\nPreamble.\n\n# Title\n\nBody.\n");
        assert_eq!(outline.containing(0), None);
        assert_eq!(outline.containing(1), None);
        assert_eq!(outline.containing(2).map(|s| s.block), Some(2));
        assert_eq!(outline.containing(3).map(|s| s.block), Some(2));
        assert_eq!(outline.containing(4), None);
    }

    #[test]
    fn the_innermost_section_holds_a_block() {
        let (_, outline) = read("# T\n\n## S\n\n### U\n\nText.\n\n## V\n");
        assert_eq!(outline.containing(3).map(|s| s.block), Some(2));
        assert_eq!(outline.containing(1).map(|s| s.block), Some(1));
        assert_eq!(outline.containing(4).map(|s| s.block), Some(4));
    }

    #[test]
    fn setext_headings_open_sections_and_a_document_without_headings_has_none() {
        let (document, outline) = read("Title\n=====\n\nText.\n\nPart\n----\n\nMore.\n");
        assert_eq!(
            shown(&document, &outline),
            [
                "title 1 0..4 [\"Title\"] \"Title\\n=====\\n\\nText.\\n\\nPart\\n----\\n\\nMore.\"",
                "part 2 2..4 [\"Title\", \"Part\"] \"Part\\n----\\n\\nMore.\"",
            ]
        );
        let (_, empty) = read("Just text.\n\n- a list\n");
        assert_eq!(empty, Outline::default().with_anchors(2));
        assert_eq!(empty.containing(0), None);
        assert_eq!(empty.find("just-text"), None);
    }

    impl Outline {
        /// An outline with no sections and `blocks` blocks without anchors.
        fn with_anchors(mut self, blocks: usize) -> Outline {
            self.anchors = vec![None; blocks];
            self
        }
    }
}
