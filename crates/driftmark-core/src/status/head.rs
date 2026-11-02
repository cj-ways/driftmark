//! The part of a document the status view needs.
//!
//! The view reads a document's front matter, its title and its masthead,
//! which all come before its first section. Parsing only that part, instead
//! of the whole file, makes the view several times cheaper on a large
//! repository. The cut is made only where the head reads exactly as it does
//! in the whole document; anywhere else the whole document is read.

use crate::document::MAX_FILE_SIZE;
use crate::rules::TypeRule;

/// The part of `source` the status view needs for a document of type
/// `rule`: its head, up to the first `##` heading after its title, when
/// that is safe, or else the whole source.
///
/// The head is the whole source when:
///
/// - the type reads a value from a section, such as adr-tools' `## Status`,
///   which may come after the head;
/// - the source is too large or not UTF-8, so the parse gives the same
///   error a reading page shows;
/// - no `# ` title line comes before a `## ` line, or no blank line comes
///   before that `## ` line;
/// - front matter is not closed, or a code fence is open at the cut;
/// - a line of the head starts with `<`, which may open an HTML block or
///   comment that runs past the cut;
/// - a line after the cut starts with `[` and holds `]:`, which may define a
///   link or a footnote that the title or the masthead uses.
#[must_use]
pub fn needed<'a>(source: &'a [u8], rule: Option<&TypeRule>) -> &'a [u8] {
    let reads_sections = rule.is_some_and(|rule| {
        [&rule.status, &rule.date, &rule.superseded_by]
            .into_iter()
            .flatten()
            .any(|location| location.section.is_some())
    });
    if reads_sections || source.len() > MAX_FILE_SIZE {
        return source;
    }
    match std::str::from_utf8(source).ok().and_then(head_end) {
        Some(end) => &source[..end],
        None => source,
    }
}

/// A fence that opened a code block: its character and its length.
type Fence = (u8, usize);

/// The byte where the head of `text` ends: the start of the first `## `
/// line after a `# ` title line, after a blank line, outside front matter
/// and code fences. `None` when there is no such line, or when cutting
/// there could change how the head reads.
fn head_end(text: &str) -> Option<usize> {
    let mut lines = Lines { text, at: 0 };
    if text.starts_with("---") {
        let opening = text.split('\n').next().unwrap_or(text);
        if opening.trim_end() != "---" {
            return None;
        }
        lines.next();
        lines.find(|(_, line)| line.trim_end() == "---")?;
    }
    let mut fence: Option<Fence> = None;
    let mut titled = false;
    let mut blank_before = false;
    for (start, line) in lines.by_ref() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let indent = line.len() - line.trim_start_matches(' ').len();
        let opened = (indent <= 3).then(|| fence_of(&line[indent..])).flatten();
        if let Some((mark, length)) = fence {
            let closes = opened.is_some_and(|(own, own_length)| {
                own == mark
                    && own_length >= length
                    && line[indent..].trim_matches(own as char).trim().is_empty()
            });
            if closes {
                fence = None;
            }
            blank_before = false;
            continue;
        }
        if let Some(found) = opened {
            fence = Some(found);
            blank_before = false;
            continue;
        }
        if indent <= 3 && line[indent..].starts_with('<') {
            return None;
        }
        if titled && blank_before && line.starts_with("## ") {
            let rest = &text[start..];
            let defines = rest.lines().any(|line| {
                let line = line.trim_start_matches(' ');
                line.starts_with('[') && line.contains("]:")
            });
            return (!defines).then_some(start);
        }
        if line.starts_with("# ") || line == "#" {
            titled = true;
        }
        blank_before = line.trim().is_empty();
    }
    None
}

/// The fence a line opens, after its indentation: three or more backticks
/// or tildes. An info string after backticks may hold no backtick.
fn fence_of(line: &str) -> Option<Fence> {
    let mark = *line
        .as_bytes()
        .first()
        .filter(|mark| matches!(mark, b'`' | b'~'))?;
    let length = line.bytes().take_while(|byte| *byte == mark).count();
    let info = &line[length..];
    (length >= 3 && !(mark == b'`' && info.contains('`'))).then_some((mark, length))
}

/// The lines of a text with the byte each starts at, without their line
/// feeds.
struct Lines<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<(usize, &'a str)> {
        if self.at >= self.text.len() {
            return None;
        }
        let start = self.at;
        let rest = &self.text[start..];
        let line = rest.split('\n').next().unwrap_or(rest);
        self.at = start + line.len() + 1;
        Some((start, line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::rules::Rules;

    fn head(text: &str) -> &str {
        std::str::from_utf8(needed(text.as_bytes(), None)).unwrap()
    }

    #[test]
    fn the_head_ends_at_the_first_section_after_the_title() {
        let text = "---\nstatus: accepted\n# not a title\n---\n\n# Cache\n\n> Status: Accepted\n\nIntro.\n\n## Decision\n\nKeep it.\n";
        assert_eq!(
            head(text),
            "---\nstatus: accepted\n# not a title\n---\n\n# Cache\n\n> Status: Accepted\n\nIntro.\n\n"
        );
        assert_eq!(head("# A\r\n\r\n## B\r\n"), "# A\r\n\r\n");
        assert_eq!(head("#\n\n## B\n"), "#\n\n");
        assert_eq!(head("Intro\n\n# A\n\n## B\n\n## C\n"), "Intro\n\n# A\n\n");
    }

    #[test]
    fn the_whole_source_is_read_when_a_cut_is_not_safe() {
        for text in [
            "# A\n\n## B\n\n[ref]: https://example.com\n",
            "# A [x][^1]\n\n## B\n\n   [^1]: a note\n",
            "# A\n<!--\n\n## B\n-->\n",
            "   <details>\n# A\n\n## B\n",
            "# A\n## B\n",
            "## B\n\n# A\n",
            "# A\n\n```\n\n## B\n```\n",
            "# A\n\n~~~~\n\n## B\n~~~\n",
            "# A\n\n```\n```x\n\n## B\n",
            "---\nstatus: open\n\n# A\n\n## B\n",
            "--- \nx: y\n",
            "---x\n# A\n\n## B\n",
            "No title.\n\n## B\n",
            "",
            "    # A\n\n## B\n",
        ] {
            assert_eq!(head(text), text, "{text:?}");
        }
        // A closed fence before the title, and fences that only look like
        // fences, do not stop the cut.
        assert_eq!(
            head("```\n# x\n```\n# A\n\n## B\n"),
            "```\n# x\n```\n# A\n\n"
        );
        assert_eq!(
            head("~~~\n``` \n~~~ \n# A\n\n## B\n"),
            "~~~\n``` \n~~~ \n# A\n\n"
        );
        assert_eq!(head("``x``\n# A\n\n## B\n"), "``x``\n# A\n\n");
        assert_eq!(head("```a`b\n# A\n\n## B\n"), "```a`b\n# A\n\n");
        assert_eq!(head("~~~a`b\n~~~\n# A\n\n## B\n"), "~~~a`b\n~~~\n# A\n\n");
        assert_eq!(
            head("`````\n````\n`````\n# A\n\n## B\n"),
            "`````\n````\n`````\n# A\n\n"
        );
        assert_eq!(head("# A\n\n    <x>\n\n## B\n"), "# A\n\n    <x>\n\n");
        assert_eq!(
            head("# A\n\nx [y]: z\n\n## B\n\n x ]: [\n"),
            "# A\n\nx [y]: z\n\n"
        );
    }

    #[test]
    fn large_text_and_bytes_are_read_whole() {
        let large = format!("# A\n\n## B\n{}", "x".repeat(MAX_FILE_SIZE));
        assert_eq!(needed(large.as_bytes(), None).len(), large.len());
        let bytes = b"# A\n\n## B\n\xff\n";
        assert_eq!(needed(bytes, None), bytes);
        let exact = format!("# A\n\n## B\n{}", "x".repeat(MAX_FILE_SIZE - 10));
        assert_eq!(exact.len(), MAX_FILE_SIZE);
        assert_eq!(needed(exact.as_bytes(), None), b"# A\n\n");
    }

    #[test]
    fn a_type_that_reads_a_section_gets_the_whole_document() {
        let rules = Rules::parse(
            "[[type]]\nname = \"adr\"\npaths = [\"adr/*.md\"]\nstatus = { section = \"Status\" }\n\n[[type]]\nname = \"d\"\npaths = [\"d/*.md\"]\nstatus = { front-matter = \"status\" }\ndate = { front-matter = \"date\" }\n\n[[type]]\nname = \"s\"\npaths = [\"s/*.md\"]\nsuperseded-by = { section = \"Replaced by\" }\n\n[[type]]\nname = \"t\"\npaths = [\"t/*.md\"]\ndate = { section = \"Date\" }\n",
        )
        .unwrap();
        let text = b"# A\n\n## Status\n\nAccepted\n";
        for (path, cut) in [
            ("adr/1.md", false),
            ("d/1.md", true),
            ("s/1.md", false),
            ("t/1.md", false),
        ] {
            let got = needed(text, rules.type_for(path));
            assert_eq!(got.len() < text.len(), cut, "{path}");
        }
    }

    #[test]
    fn the_head_reads_as_the_whole_document_does() {
        let rules = Rules::parse(
            "[masthead]\nkeys = [\"Created\"]\n\n[[type]]\nname = \"d\"\npaths = [\"*.md\"]\nstatus = { front-matter = \"status\", masthead = \"Status\" }\ndate = { masthead = \"Created\" }\nsuperseded-by = { masthead = \"Superseded by\" }\n",
        )
        .unwrap();
        let text = "---\nstatus: superseded\n---\n\n# Store **files** on disk\n\n> Status: Superseded | Superseded by: [B](b.md)\n\n**Created**: 2026-01-02\n\n- a list\n- of items\n\n## Decision\n\nKeep [it][r].\n\n## Notes\n";
        let head = needed(text.as_bytes(), rules.type_for("a.md"));
        assert!(head.len() < text.len());
        let whole = Document::parse_with(text.as_bytes().to_vec(), rules.masthead_keys()).unwrap();
        let part = Document::parse_with(head.to_vec(), rules.masthead_keys()).unwrap();
        assert_eq!(part.meta.title, whole.meta.title);
        let mut whole_class = rules.classify("a.md", &whole);
        let mut part_class = rules.classify("a.md", &part);
        whole_class.decision_blocks.clear();
        part_class.decision_blocks.clear();
        assert_eq!(part_class, whole_class);
        assert_eq!(part_class.date.as_deref(), Some("2026-01-02"));
    }
}
