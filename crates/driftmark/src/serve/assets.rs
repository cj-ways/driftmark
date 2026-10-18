//! The files the pages load, built into the program.
//!
//! Each file is served at `/static/<hash>/<name>`, where the hash is taken
//! from its bytes. A new build with other bytes gets another address, so the
//! browser may keep a file for a year without ever showing an old one.
//!
//! - `page.css` and `page.js`: the look and the behaviour of every page.
//!   They name the other files by their hashed addresses, which are written
//!   into them when the server starts.
//! - `liberation-sans-regular.woff2`: the diagram font (decision 0010),
//!   loaded only by pages that show a drawn diagram.
//! - `prism-core.js` and one `prism-<language>.js` per language: code
//!   colouring, loaded only by pages with code, and only for the languages
//!   the page uses.
//! - `mermaid.js`: the browser fallback for a diagram the server could not
//!   draw, loaded only by pages with such a diagram. It is stored
//!   compressed (1.6 MB instead of 5.5 MB) and sent as it is to a browser
//!   that accepts gzip, which every browser does.

use std::fmt::Write as _;
use std::io::Read as _;

use axum::body::Bytes;

use super::fingerprint;

/// One built-in file.
pub(crate) struct Asset {
    /// The file name in the address.
    pub name: &'static str,
    /// Its media type.
    pub content_type: &'static str,
    /// Its bytes, compressed with gzip when `gzip` is set.
    pub bytes: Bytes,
    /// Whether `bytes` is gzip.
    pub gzip: bool,
    /// The hash in its address: 16 hex digits of a hash of its bytes.
    pub hash: String,
}

impl Asset {
    fn new(name: &'static str, content_type: &'static str, bytes: Bytes, gzip: bool) -> Asset {
        let hash = format!("{:016x}", fingerprint(&bytes));
        Asset {
            name,
            content_type,
            bytes,
            gzip,
            hash,
        }
    }

    /// The bytes to send: as stored when the browser accepts gzip or the
    /// file is not compressed, otherwise unpacked.
    pub(crate) fn body(&self, accepts_gzip: bool) -> Bytes {
        if !self.gzip || accepts_gzip {
            return self.bytes.clone();
        }
        let mut out = Vec::new();
        match flate2::read::GzDecoder::new(&self.bytes[..]).read_to_end(&mut out) {
            Ok(_) => Bytes::from(out),
            Err(_) => Bytes::new(),
        }
    }
}

/// The files the pages load.
pub(crate) struct Assets {
    files: Vec<Asset>,
}

const JS: &str = "text/javascript; charset=utf-8";

macro_rules! prism {
    ($name:literal) => {
        (
            concat!("prism-", $name, ".js"),
            include_bytes!(concat!(
                "../../assets/vendor/prism/prism-",
                $name,
                ".min.js"
            ))
            .as_slice(),
        )
    };
}

/// Prism's language files, by the name the page script asks for.
const PRISM: &[(&str, &[u8])] = &[
    prism!("bash"),
    prism!("batch"),
    prism!("c"),
    prism!("clike"),
    prism!("cmake"),
    prism!("cpp"),
    prism!("csharp"),
    prism!("css"),
    prism!("csv"),
    prism!("dart"),
    prism!("diff"),
    prism!("docker"),
    prism!("elixir"),
    prism!("erlang"),
    prism!("go"),
    prism!("gradle"),
    prism!("graphql"),
    prism!("groovy"),
    prism!("haskell"),
    prism!("hcl"),
    prism!("http"),
    prism!("ini"),
    prism!("java"),
    prism!("javascript"),
    prism!("json"),
    prism!("json5"),
    prism!("jsx"),
    prism!("kotlin"),
    prism!("latex"),
    prism!("lua"),
    prism!("makefile"),
    prism!("markdown"),
    prism!("markup"),
    prism!("nginx"),
    prism!("nix"),
    prism!("perl"),
    prism!("powershell"),
    prism!("promql"),
    prism!("properties"),
    prism!("protobuf"),
    prism!("python"),
    prism!("r"),
    prism!("regex"),
    prism!("ruby"),
    prism!("rust"),
    prism!("scala"),
    prism!("shell-session"),
    prism!("solidity"),
    prism!("sql"),
    prism!("swift"),
    prism!("toml"),
    prism!("tsx"),
    prism!("typescript"),
    prism!("verilog"),
    prism!("vim"),
    prism!("wasm"),
    prism!("xml-doc"),
    prism!("yaml"),
    prism!("zig"),
];

impl Assets {
    /// The files built into this program, with their addresses written into
    /// the style sheet and the page script.
    pub(crate) fn built_in() -> Assets {
        let mut files = vec![
            Asset::new(
                "liberation-sans-regular.woff2",
                "font/woff2",
                Bytes::from_static(include_bytes!(
                    "../../assets/fonts/liberation-sans-regular.woff2"
                )),
                false,
            ),
            Asset::new(
                "mermaid.js",
                JS,
                Bytes::from_static(include_bytes!(
                    "../../assets/vendor/mermaid/mermaid.min.js.gz"
                )),
                true,
            ),
            Asset::new(
                "prism-core.js",
                JS,
                Bytes::from_static(include_bytes!(
                    "../../assets/vendor/prism/prism-core.min.js"
                )),
                false,
            ),
        ];
        for (name, bytes) in PRISM {
            files.push(Asset::new(name, JS, Bytes::from_static(bytes), false));
        }
        let mut assets = Assets { files };
        let font = assets.address("liberation-sans-regular.woff2");
        let css = include_str!("../../assets/page.css").replace("__FONT__", &font);
        let mut addresses = String::from("{");
        for file in &assets.files {
            if std::path::Path::new(file.name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("js"))
            {
                let _ = write!(
                    addresses,
                    "\"{}\":\"/static/{}/{}\",",
                    file.name, file.hash, file.name
                );
            }
        }
        addresses.pop();
        addresses.push('}');
        let js = include_str!("../../assets/page.js").replace("__ASSETS__", &addresses);
        assets.files.push(Asset::new(
            "page.css",
            "text/css; charset=utf-8",
            Bytes::from(css.into_bytes()),
            false,
        ));
        assets.files.push(Asset::new(
            "page.js",
            JS,
            Bytes::from(js.into_bytes()),
            false,
        ));
        assets
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

    #[test]
    fn the_style_sheet_and_the_script_name_the_other_files() {
        let assets = Assets::built_in();
        let css = String::from_utf8(assets.get("page.css").unwrap().bytes.to_vec()).unwrap();
        assert!(css.contains(&assets.address("liberation-sans-regular.woff2")));
        assert!(!css.contains("__FONT__"));
        let js = String::from_utf8(assets.get("page.js").unwrap().bytes.to_vec()).unwrap();
        assert!(js.contains(&assets.address("mermaid.js")));
        assert!(js.contains(&assets.address("prism-tsx.js")));
        assert!(!js.contains("__ASSETS__"));
    }

    #[test]
    fn mermaid_unpacks_for_a_browser_without_gzip() {
        let assets = Assets::built_in();
        let mermaid = assets.get("mermaid.js").unwrap();
        assert!(mermaid.gzip);
        assert_eq!(mermaid.body(true).len(), mermaid.bytes.len());
        let plain = mermaid.body(false);
        assert!(plain.len() > 5_000_000);
        assert!(plain.starts_with(b"\"use strict\""));
        let css = assets.get("page.css").unwrap();
        assert_eq!(css.body(false).len(), css.bytes.len());
    }
}
