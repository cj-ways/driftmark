//! Lines and columns for findings. A finding names a 1-based line and a
//! 1-based column counted in characters, as editors count them, while the
//! document model works in byte offsets.

use crate::document::markdown::LineIndex;

/// The lines of one file's text.
pub(super) struct Lines<'t> {
    text: &'t str,
    index: LineIndex,
}

impl<'t> Lines<'t> {
    pub(super) fn new(text: &'t str) -> Lines<'t> {
        Lines {
            text,
            index: LineIndex::new(text),
        }
    }

    /// The line and column of byte `offset`. An offset inside a character
    /// counts as that character.
    pub(super) fn at(&self, offset: usize) -> (usize, usize) {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.index.line_of(offset);
        let start = self.index.line_start(line).unwrap_or(0);
        (line, self.text[start..offset].chars().count() + 1)
    }

    /// The text of 1-based line `line`, without its line ending. Empty for a
    /// line the text does not have.
    pub(super) fn text(&self, line: usize) -> &'t str {
        let Some(start) = self.index.line_start(line) else {
            return "";
        };
        let end = self.index.line_start(line + 1).unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    /// The 1-based line that holds byte `offset`.
    pub(super) fn line_of(&self, offset: usize) -> usize {
        self.index.line_of(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::Lines;

    #[test]
    fn columns_count_characters_from_one() {
        let lines = Lines::new("ab\nтест x\r\nlast");
        assert_eq!(lines.at(0), (1, 1));
        assert_eq!(lines.at(2), (1, 3));
        assert_eq!(lines.at(3), (2, 1));
        // `x` comes after four two-byte letters and a space.
        assert_eq!(lines.at(12), (2, 6));
        assert_eq!(lines.at(15), (3, 1));
        assert_eq!(lines.at(99), (3, 5));
    }

    #[test]
    fn an_offset_inside_a_character_counts_as_that_character() {
        let lines = Lines::new("aé b");
        assert_eq!(lines.at(2), (1, 2));
        assert_eq!(lines.at(3), (1, 3));
    }

    #[test]
    fn line_text_drops_the_line_ending() {
        let lines = Lines::new("one\r\ntwo\nthree");
        assert_eq!(lines.text(1), "one");
        assert_eq!(lines.text(2), "two");
        assert_eq!(lines.text(3), "three");
        assert_eq!(lines.text(4), "");
        assert_eq!(lines.text(0), "");
        assert_eq!(lines.line_of(5), 2);
    }
}
