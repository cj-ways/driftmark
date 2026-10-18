//! The page script and style sheet, built into the program.
//!
//! Each file is served at `/static/<hash>/<name>`, where the hash is taken
//! from its bytes. A new build with other bytes gets another address, so the
//! browser may keep a file for a year without ever showing an old one.

use super::fingerprint;

/// One built-in file.
pub(crate) struct Asset {
    /// The file name in the address.
    pub name: &'static str,
    /// Its media type.
    pub content_type: &'static str,
    /// Its bytes.
    pub bytes: &'static [u8],
    /// The hash in its address: 16 hex digits of a hash of its bytes.
    pub hash: String,
}

/// The files the pages load.
pub(crate) struct Assets {
    files: Vec<Asset>,
}

impl Assets {
    /// The files built into this program.
    pub(crate) fn built_in() -> Assets {
        let file = |name, content_type, bytes: &'static [u8]| Asset {
            name,
            content_type,
            bytes,
            hash: format!("{:016x}", fingerprint(bytes)),
        };
        Assets {
            files: vec![
                file(
                    "page.js",
                    "text/javascript; charset=utf-8",
                    include_bytes!("../../assets/page.js"),
                ),
                file(
                    "page.css",
                    "text/css; charset=utf-8",
                    include_bytes!("../../assets/page.css"),
                ),
            ],
        }
    }

    /// The file with this name.
    pub(crate) fn get(&self, name: &str) -> Option<&Asset> {
        self.files.iter().find(|file| file.name == name)
    }

    /// The address a page links to for the file, or an address that finds
    /// nothing when there is no such file.
    pub(crate) fn address(&self, name: &str) -> String {
        let hash = self.get(name).map_or("missing", |file| file.hash.as_str());
        format!("/static/{hash}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_carry_the_hash_of_the_bytes() {
        let assets = Assets::built_in();
        let js = assets.get("page.js").unwrap();
        assert_eq!(js.hash.len(), 16);
        assert_eq!(
            assets.address("page.js"),
            format!("/static/{}/page.js", js.hash)
        );
        assert_ne!(js.hash, assets.get("page.css").unwrap().hash);
        assert_eq!(assets.address("nothing.js"), "/static/missing/nothing.js");
    }
}
