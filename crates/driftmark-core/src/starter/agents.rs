//! The block of agent instructions that `driftmark init` prints for a file
//! such as `AGENTS.md` or `CLAUDE.md`, and appends to one with
//! `--agent-file`.
//!
//! The block starts and ends with an HTML comment, which renders nothing.
//! The first one marks a file that already holds the block, so it is never
//! added twice, even after the text between the two was edited.

/// The first line of the block, which marks it.
pub const AGENT_BLOCK_START: &str = "<!-- driftmark: starter layout -->";

/// The block, from its first line to its last, with a final line break.
pub const AGENT_BLOCK: &str = "\
<!-- driftmark: starter layout -->

## Documents

This repository keeps its documents in `docs/`, one folder per kind:

- `docs/decisions/`: numbered decision records. A record is written once. A later record
  replaces it instead of an edit.
- `docs/standards/`: rules that stay true. Change them in place.
- `docs/features/`: the decided target of each feature.
- `docs/working/<topic>/`: active work, with a `plan.md` and a `status.md`.
- `docs/archive/`: finished working folders, moved here whole.

Start every new document with `driftmark new <kind> \"<title>\"`. The kind is `decision`,
`standard`, `feature`, `plan` or `status`. The command prints the new file's path. Replace each
`<!-- … -->` comment in the file with your text.

Write as `docs/standards/writing.md` says. After you edit a document, run `driftmark lint` and
fix what it reports.

<!-- driftmark: end of starter layout -->
";

/// What a file that does not exist yet starts with, before the block, so the
/// new file has the title and short version the writing check asks for.
const NEW_FILE_HEAD: &str = "\
# Agent instructions

> Short version: how AI agents work in this repository.

";

/// The text `init --write` adds at the end of an agent instructions file
/// whose current text is `existing`, or `None` when the file was never made.
/// Returns `None` instead when the file already holds the block.
///
/// - A missing or empty file gets a title, a short version and the block.
/// - Any other file gets a blank line, then the block. Its last line is
///   ended first when it has no line break.
/// - A file that uses Windows line breaks gets them in the added text too.
#[must_use]
pub fn agent_addition(existing: Option<&str>) -> Option<String> {
    let text = existing.unwrap_or_default();
    if text.contains(AGENT_BLOCK_START) {
        return None;
    }
    if text.trim().is_empty() {
        return Some(format!("{NEW_FILE_HEAD}{AGENT_BLOCK}"));
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let breaks = if text.ends_with(&newline.repeat(2)) {
        0
    } else if text.ends_with(newline) {
        1
    } else {
        2
    };
    Some(newline.repeat(breaks) + &AGENT_BLOCK.replace('\n', newline))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_is_marked_at_both_ends() {
        assert!(AGENT_BLOCK.starts_with(&format!("{AGENT_BLOCK_START}\n\n## ")));
        assert!(AGENT_BLOCK.ends_with("\n\n<!-- driftmark: end of starter layout -->\n"));
        assert_eq!(AGENT_BLOCK.matches(AGENT_BLOCK_START).count(), 1);
    }

    #[test]
    fn the_block_fits_a_code_block_without_long_lines() {
        // Its lines stay under the writing check's limit for code lines, so
        // the block can be shown in a `markdown` code block.
        for line in AGENT_BLOCK.lines() {
            assert!(line.chars().count() <= 100, "{line}");
        }
    }

    #[test]
    fn a_new_file_gets_a_title_first() {
        let whole = agent_addition(None).expect("a new file gets the block");
        assert!(whole.starts_with(
            "# Agent instructions\n\n> Short version: how AI agents work in this repository.\n\n<!-- driftmark: starter layout -->\n"
        ));
        assert!(whole.ends_with(AGENT_BLOCK));
        assert_eq!(agent_addition(Some("")), Some(whole.clone()));
        assert_eq!(agent_addition(Some(" \n\n")), Some(whole));
    }

    #[test]
    fn the_block_follows_a_blank_line() {
        let after = |text: &str| agent_addition(Some(text)).expect("the block is missing");
        assert_eq!(after("# Agents\n"), format!("\n{AGENT_BLOCK}"));
        assert_eq!(after("# Agents\n\n"), AGENT_BLOCK);
        assert_eq!(after("# Agents"), format!("\n\n{AGENT_BLOCK}"));
    }

    #[test]
    fn windows_line_breaks_are_kept() {
        let block = AGENT_BLOCK.replace('\n', "\r\n");
        let after = |text: &str| agent_addition(Some(text)).expect("the block is missing");
        assert_eq!(after("# Agents\r\n"), format!("\r\n{block}"));
        assert_eq!(after("# Agents\r\n\r\n"), block);
        assert_eq!(
            after("# Agents\r\n\r\nUse tabs."),
            format!("\r\n\r\n{block}")
        );
    }

    #[test]
    fn a_file_with_the_block_gets_nothing() {
        let edited = "# Agents\n\n<!-- driftmark: starter layout -->\n\nOur own words.\n";
        assert_eq!(agent_addition(Some(edited)), None);
        let whole = agent_addition(None).expect("a new file gets the block");
        assert_eq!(agent_addition(Some(&whole)), None);
    }
}
