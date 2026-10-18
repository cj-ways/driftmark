//! The rules file of a range: `driftmark.toml` at the repository root, on the
//! head side. The head is the newer version, so a change to the rules file
//! inside the range already applies to the documents it changes.

use driftmark_core::git::{GitError, GitRead, ObjectId, RepoPath, Snapshot};
use driftmark_core::rules::{Rules, RulesError};

use crate::lint::RULES_FILE;

/// The rules at the head side: from the head commit, or from the working tree
/// when the head is the working tree. No rules file means the default rules.
///
/// # Errors
///
/// A sentence for the user that says where the file was read and, when it is
/// broken, on which line.
pub(crate) fn read(git: &dyn GitRead, head: &Snapshot) -> Result<Rules, String> {
    let Some(path) = RepoPath::new(RULES_FILE) else {
        // A fixed name, always a valid path.
        return Ok(Rules::default());
    };
    let (place, bytes) = match head {
        Snapshot::Commit(commit) => {
            let place = format!("{RULES_FILE} at commit {}", short(commit));
            match git.blob_at(commit, &path) {
                Ok(None) => return Ok(Rules::default()),
                Ok(Some(blob)) => (place, git.read_blob(&blob)),
                Err(error) => return Err(format!("{place} could not be looked up: {error}")),
            }
        }
        Snapshot::Worktree => match git.read_worktree_file(&path) {
            Err(GitError::MissingFile { .. }) => return Ok(Rules::default()),
            bytes => (format!("{RULES_FILE} in the working tree"), bytes),
        },
    };
    let bytes = bytes.map_err(|error| format!("{place} could not be read: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{place} is not UTF-8 text"))?;
    Rules::parse(&text).map_err(|error| describe(&place, &text, &error))
}

/// The first 12 characters of a commit id, enough to find it again.
pub(crate) fn short(id: &ObjectId) -> &str {
    let hex = id.as_hex();
    hex.get(..12).unwrap_or(hex)
}

/// The error as one message that names the line. A TOML error names it
/// already. A type or `[lint]` problem names the table, so the line is where
/// that table starts.
fn describe(place: &str, text: &str, error: &RulesError) -> String {
    let line = match error {
        RulesError::Syntax(_) => None,
        RulesError::Invalid { number, .. } => table_line(text, "[[type]]", *number),
        RulesError::Lint(_) => table_line(text, "[lint]", 1),
    };
    let message = error.to_string();
    let message = message.trim_end();
    match line {
        Some(line) => format!("{place}, line {line}: {message}"),
        None => format!("{place}: {message}"),
    }
}

/// The line, from 1, of the `nth` table header written as `header`, with any
/// spaces and a comment allowed around it.
fn table_line(text: &str, header: &str, nth: usize) -> Option<usize> {
    let is_header = |line: &str| {
        let code = line.split('#').next().unwrap_or_default();
        code.chars()
            .filter(|c| !c.is_whitespace())
            .eq(header.chars())
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| is_header(line))
        .nth(nth.checked_sub(1)?)
        .map(|(index, _)| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_name_the_line_of_their_table() {
        let text = "\
# made-up rules
[[type]]
name = \"decision\"
paths = [\"docs/decisions/*.md\"]

[[ type ]]  # the second type
name = \"feature\"
paths = []

[lint]
allowed-html = [\"<b>\"]
";
        let error = Rules::parse(text).unwrap_err();
        assert_eq!(
            describe("driftmark.toml in the working tree", text, &error),
            "driftmark.toml in the working tree, line 6: the rules file's type 2 (\"feature\"): it lists no paths"
        );
        let lint_only = &text[text.find("[lint]").unwrap()..];
        let error = Rules::parse(lint_only).unwrap_err();
        let message = describe("here", lint_only, &error);
        assert!(
            message.starts_with("here, line 1: the rules file's [lint] table: "),
            "{message}"
        );
        let error = Rules::parse("stauts = 1\n").unwrap_err();
        let message = describe("here", "stauts = 1\n", &error);
        assert!(
            message
                .starts_with("here: the rules file could not be read: TOML parse error at line 1"),
            "{message}"
        );
    }

    #[test]
    fn a_header_that_is_not_found_names_no_line() {
        assert_eq!(table_line("type = []\n", "[[type]]", 1), None);
        assert_eq!(table_line("[[type]]\n", "[[type]]", 0), None);
        assert_eq!(table_line("[[type]]\n[[type]]\n", "[[type]]", 2), Some(2));
    }

    #[test]
    fn short_ids_keep_twelve_characters() {
        let id = ObjectId::from_hex("0123456789abcdef0123456789abcdef01234567").unwrap();
        assert_eq!(short(&id), "0123456789ab");
    }
}
