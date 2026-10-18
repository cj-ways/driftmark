//! Reading a raw commit object.

use super::ObjectId;

/// What driftmark needs from a commit: its parents and when it was committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// The parent commits, first parent first. A merge has two or more; the
    /// first commit of a history has none.
    pub parents: Vec<ObjectId>,
    /// When the commit was made, in seconds since 1970-01-01 00:00 UTC. This
    /// is the committer date, the one `git log --since` uses; it changes when a
    /// commit is rebased, unlike the author date.
    pub committed_at: i64,
}

impl CommitInfo {
    /// Reads a raw commit object, as [`super::GitRead::read_commit`] returns
    /// it. Returns `None` when the bytes are not a commit that git would write.
    ///
    /// Only the header is read, up to the first empty line. Signature lines
    /// that continue on the next line start with a space, so they never look
    /// like a `parent` or `committer` line.
    #[must_use]
    pub fn parse(raw: &[u8]) -> Option<Self> {
        let mut parents = Vec::new();
        let mut committed_at = None;
        for line in raw
            .split(|&b| b == b'\n')
            .take_while(|line| !line.is_empty())
        {
            if let Some(hex) = line.strip_prefix(b"parent ") {
                parents.push(ObjectId::from_hex(std::str::from_utf8(hex).ok()?)?);
            } else if let Some(person) = line.strip_prefix(b"committer ")
                && committed_at.is_none()
            {
                committed_at = Some(commit_time(person)?);
            }
        }
        Some(Self {
            parents,
            committed_at: committed_at?,
        })
    }
}

/// The root tree of a raw commit object, from the `tree` line git writes
/// first. Returns `None` when the first line is not a `tree` line.
///
/// Two commits with the same tree hold the same files, so anything made from
/// every file of a commit, such as a search index, can be kept by its tree
/// and shared.
#[must_use]
pub fn tree_of(raw: &[u8]) -> Option<ObjectId> {
    let first = raw.split(|&b| b == b'\n').next().unwrap_or_default();
    ObjectId::from_hex(std::str::from_utf8(first.strip_prefix(b"tree ")?).ok()?)
}

/// The time in a `committer` line: `Name <email> 1760000000 +0200`. The name
/// and email may hold almost anything, so the time is read after the last `>`.
fn commit_time(person: &[u8]) -> Option<i64> {
    let after_email = &person[person.iter().rposition(|&b| b == b'>')? + 1..];
    let mut words = std::str::from_utf8(after_email)
        .ok()?
        .split_ascii_whitespace();
    words.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n";

    fn hex(c: char) -> String {
        c.to_string().repeat(40)
    }

    #[test]
    fn reads_parents_in_order_and_the_committer_date() {
        let raw = format!(
            "{TREE}parent {}\nparent {}\nauthor A <a@x> 100 +0000\n\
             committer C > D <c@x> 1760000000 +0400\n\nparent {}\ncommitter X <x> 5 +0000\n",
            hex('1'),
            hex('2'),
            hex('3'),
        );
        let info = CommitInfo::parse(raw.as_bytes()).unwrap();
        assert_eq!(info.committed_at, 1_760_000_000);
        let parents: Vec<_> = info.parents.iter().map(ObjectId::as_hex).collect();
        assert_eq!(parents, [hex('1'), hex('2')]);
    }

    #[test]
    fn a_root_commit_has_no_parents_and_signatures_are_skipped() {
        let raw = format!(
            "{TREE}author A <a@x> 1 +0000\ncommitter C <c@x> -5 -0100\n\
             gpgsig -----BEGIN-----\n parent {}\n committer Z <z> 9 +0000\n -----END-----\n\nmsg\n",
            hex('9'),
        );
        let info = CommitInfo::parse(raw.as_bytes()).unwrap();
        assert_eq!(
            info,
            CommitInfo {
                parents: Vec::new(),
                committed_at: -5
            }
        );
    }

    #[test]
    fn the_first_committer_line_counts() {
        let raw = format!("{TREE}committer A <a> 7 +0000\ncommitter B <b> 8 +0000\n");
        assert_eq!(CommitInfo::parse(raw.as_bytes()).unwrap().committed_at, 7);
    }

    #[test]
    fn a_header_ends_at_the_end_of_the_bytes_too() {
        let raw = format!("{TREE}committer A <a> 3 +0000");
        assert_eq!(CommitInfo::parse(raw.as_bytes()).unwrap().committed_at, 3);
    }

    #[test]
    fn broken_commits_are_refused() {
        let bad = [
            format!("{TREE}author A <a> 1 +0000\n"),
            format!("{TREE}committer A <a> soon +0000\n"),
            format!("{TREE}committer A <a>\n"),
            format!("{TREE}committer A a 1 +0000\n"),
            format!("{TREE}committer A <a> \u{0}1 +0000\n"),
            format!("{TREE}parent nothex\ncommitter A <a> 1 +0000\n"),
            format!("{TREE}\ncommitter A <a> 1 +0000\n"),
        ];
        for raw in bad {
            assert_eq!(CommitInfo::parse(raw.as_bytes()), None, "{raw:?}");
        }
        let not_utf8_parent = b"parent \xff\ncommitter A <a> 1 +0000\n";
        assert_eq!(CommitInfo::parse(not_utf8_parent), None);
        let not_utf8_time = b"committer A <a> \xff1 +0000\n";
        assert_eq!(CommitInfo::parse(not_utf8_time), None);
    }

    #[test]
    fn the_tree_is_read_from_the_first_line() {
        let raw = format!("{TREE}committer A <a> 3 +0000\n");
        assert_eq!(
            tree_of(raw.as_bytes()).unwrap().as_hex(),
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
        assert_eq!(
            tree_of(b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904"),
            tree_of(raw.as_bytes())
        );
        assert_eq!(
            tree_of(b"committer A <a> 3 +0000\ntree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"),
            None
        );
        assert_eq!(tree_of(b"tree \xff\n"), None);
        assert_eq!(tree_of(b"tree nothex\n"), None);
        assert_eq!(tree_of(b""), None);
    }

    #[test]
    fn a_name_with_odd_bytes_does_not_matter() {
        let raw = b"committer \xff\xfe <\xff> 42 +0000\n";
        assert_eq!(CommitInfo::parse(raw).unwrap().committed_at, 42);
    }
}
