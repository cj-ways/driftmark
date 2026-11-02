//! Where a document's links lead inside an exported file.
//!
//! Every exported document is a page of the file with its own id, and the
//! ids inside it start with that id and a `-` (`driftmark_core::render::
//! reading_prefixed`). So a link to another exported document becomes
//! `#R3`, and a link to one of its sections `#R3-install`. A link to a
//! document the file does not hold, or to any other file, and every image,
//! shows as text: the file reaches nothing outside itself.

use std::collections::HashMap;

use driftmark_core::render::{LinkKind, LinkResolver, Resolved};
use percent_encoding::percent_decode_str;

/// The exported documents' page ids by path, from the repository root.
pub(crate) type Pages = HashMap<String, String>;

/// The links of one document, which sits at `from`.
pub(crate) struct Links<'a> {
    /// The folders of the document's path, which a relative link starts in.
    folders: Vec<&'a str>,
    /// Where each exported document is in the file.
    pages: &'a Pages,
}

impl<'a> Links<'a> {
    /// The links of the document at `from`.
    pub(crate) fn new(from: &'a str, pages: &'a Pages) -> Links<'a> {
        let mut folders: Vec<&str> = from.split('/').collect();
        folders.pop();
        Links { folders, pages }
    }

    /// The repository path a link's file part names, or `None` when it
    /// climbs above the repository root.
    fn target(&self, file: &str) -> Option<String> {
        let (mut parts, rest) = match file.strip_prefix('/') {
            Some(from_root) => (Vec::new(), from_root),
            None => (self.folders.clone(), file),
        };
        for part in rest.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                part => parts.push(part),
            }
        }
        Some(parts.join("/"))
    }
}

impl LinkResolver for Links<'_> {
    fn resolve(&self, address: &str, kind: LinkKind) -> Resolved {
        if kind == LinkKind::Image {
            return Resolved::Text;
        }
        let (file, place) = match address.split_once('#') {
            Some((file, place)) => (file, Some(place)),
            None => (address, None),
        };
        let file = file.split('?').next().unwrap_or_default();
        let decoded = percent_decode_str(file).decode_utf8_lossy();
        let Some(path) = self.target(&decoded) else {
            return Resolved::Text;
        };
        match (self.pages.get(&path), place) {
            (Some(page), Some(place)) if !place.is_empty() => {
                Resolved::Url(format!("#{page}-{place}"))
            }
            (Some(page), _) => Resolved::Url(format!("#{page}")),
            (None, _) => Resolved::Text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> Pages {
        [
            ("docs/a.md", "R1"),
            ("docs/sub/b.md", "R2"),
            ("README.md", "R3"),
            ("docs/with space.md", "R4"),
        ]
        .into_iter()
        .map(|(path, id)| (path.to_owned(), id.to_owned()))
        .collect()
    }

    #[test]
    fn links_to_exported_documents_lead_to_their_pages() {
        let pages = pages();
        let links = Links::new("docs/sub/b.md", &pages);
        for (address, expected) in [
            ("../a.md", Resolved::Url("#R1".into())),
            ("../a.md#install", Resolved::Url("#R1-install".into())),
            ("../a.md#", Resolved::Url("#R1".into())),
            ("./b.md?plain=1", Resolved::Url("#R2".into())),
            ("/README.md", Resolved::Url("#R3".into())),
            ("../../README.md", Resolved::Url("#R3".into())),
            ("../with%20space.md", Resolved::Url("#R4".into())),
            ("", Resolved::Text),
            ("c.md", Resolved::Text),
            ("../../../outside.md", Resolved::Text),
            ("../../src/main.rs", Resolved::Text),
        ] {
            assert_eq!(
                links.resolve(address, LinkKind::Link),
                expected,
                "{address}"
            );
        }
        assert_eq!(links.resolve("../a.md", LinkKind::Image), Resolved::Text);
        let top = Links::new("README.md", &pages);
        assert_eq!(
            top.resolve("docs/a.md", LinkKind::Link),
            Resolved::Url("#R1".into())
        );
        assert_eq!(top.resolve("../a.md", LinkKind::Link), Resolved::Text);
    }
}
