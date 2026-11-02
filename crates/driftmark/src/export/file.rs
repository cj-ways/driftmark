//! One HTML file that opens from disk with no server and no network.
//!
//! Everything the pages need is inside it: the style sheet, the page
//! script, Prism for the languages its code uses, and the diagram font
//! when it shows a drawn diagram. Its Content-Security-Policy, in a
//! `<meta>` element before anything else, allows exactly those inline
//! blocks, named by their SHA-256 hashes, and the font as a `data:`
//! address. Nothing else may run or load: no other script, no style
//! attribute, no image, no request. `docs/features/commands/export.md`
//! states the whole contract.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

use crate::serve::assets::Assets;
use crate::serve::page::escape;

/// The style sheet of the server's pages.
const PAGE_CSS: &str = include_str!("../../assets/page.css");
/// What an exported file adds to it.
const EXPORT_CSS: &str = include_str!("../../assets/export.css");
/// The page script of an exported file.
pub(crate) const EXPORT_JS: &str = include_str!("../../assets/export.js");
/// The diagram font, Liberation Sans (decision 0010).
const FONT: &[u8] = include_bytes!("../../assets/fonts/liberation-sans-regular.woff2");

/// The marker of a drawn diagram in reading HTML: a file that holds one
/// carries the diagram font.
const DRAWN: &str = "<figure class=\"diagram\"";

/// What the file holds, besides its pages.
pub(crate) struct Parts<'a> {
    /// The file's title, without "driftmark".
    pub title: &'a str,
    /// What was exported: `doc`, `folder` or `changes`.
    pub kind: &'a str,
    /// The id of the page the brand links to: the file's first page.
    pub first: &'a str,
    /// The line about the file at the top of every page.
    pub about: &'a str,
    /// The pages.
    pub pages: &'a str,
    /// The search index as JSON, when the file has more than one document.
    pub search: Option<&'a str>,
    /// Prism's languages the pages use, by the name a code block writes,
    /// with each one's Prism name.
    pub languages: &'a [(String, String)],
    /// Prism's files to carry, in order, as Prism names.
    pub prism: &'a [String],
}

/// The style sheet of an exported file: the pages' own, with the diagram
/// font inside it as a `data:` address when `font` is set, and without it
/// otherwise, then the export's additions.
pub(crate) fn style_sheet(font: bool) -> String {
    let face_start = PAGE_CSS.find("@font-face").unwrap_or(0);
    let face_end = PAGE_CSS[face_start..]
        .find('}')
        .map_or(face_start, |end| face_start + end + 1);
    let face = if font {
        PAGE_CSS[face_start..face_end].replace(
            "__FONT__",
            &format!("data:font/woff2;base64,{}", STANDARD.encode(FONT)),
        )
    } else {
        String::new()
    };
    format!(
        "{}{face}{}\n{EXPORT_CSS}",
        &PAGE_CSS[..face_start],
        &PAGE_CSS[face_end..]
    )
}

/// The `'sha256-…'` source of one inline block, as a security policy names
/// it.
pub(crate) fn hash_source(text: &str) -> String {
    format!(
        "'sha256-{}'",
        STANDARD.encode(Sha256::digest(text.as_bytes()))
    )
}

/// JSON that can sit inside a `<script>` element: every `<` written as
/// `<`, so no text closes the element.
fn script_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .replace('<', "\\u003c")
}

/// Whether the pages show a drawn diagram, which needs the diagram font.
pub(crate) fn draws(pages: &str) -> bool {
    pages.contains(DRAWN)
}

/// The whole file.
pub(crate) fn file(parts: &Parts<'_>, assets: &Assets) -> String {
    let font = draws(parts.pages);
    let css = style_sheet(font);
    let mut scripts = vec![EXPORT_JS.to_owned()];
    if !parts.prism.is_empty() {
        for name in std::iter::once("core").chain(parts.prism.iter().map(String::as_str)) {
            if let Some(asset) = assets.get(&format!("prism-{name}.js")) {
                scripts.push(String::from_utf8_lossy(&asset.bytes).into_owned());
            }
        }
    }
    let mut policy = format!(
        "default-src 'none'; style-src {}; script-src",
        hash_source(&css)
    );
    for script in &scripts {
        policy.push(' ');
        policy.push_str(&hash_source(script));
    }
    if font {
        policy.push_str("; font-src data:");
    }
    policy.push_str("; base-uri 'none'; form-action 'none'");
    let settings = serde_json::json!({
        "kind": parts.kind,
        "first": parts.first,
        "prism": parts
            .languages
            .iter()
            .map(|(written, name)| (written.clone(), serde_json::Value::from(name.clone())))
            .collect::<serde_json::Map<_, _>>(),
    });
    let mut out = String::with_capacity(parts.pages.len() + css.len() + 64 * 1024);
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(
        out,
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">"
    );
    let _ = write!(
        out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <meta name=\"generator\" content=\"driftmark {}\">\n\
         <title>{} · driftmark</title>\n<style>{css}</style>\n",
        driftmark_core::version(),
        escape(parts.title)
    );
    for script in &scripts {
        out.push_str("<script>");
        out.push_str(script);
        out.push_str("</script>\n");
    }
    let _ = write!(
        out,
        "</head>\n<body data-export=\"{}\">\n\
         <a class=\"skip\" href=\"#Main\">Skip to the content</a>\n\
         <header class=\"top\"><a class=\"brand\" href=\"#{}\">driftmark</a>\
         <form class=\"search\" role=\"search\" hidden>\
         <label class=\"vh\" for=\"Search\">Search this file</label>\
         <input id=\"Search\" type=\"search\" placeholder=\"Search this file\" autocomplete=\"off\" spellcheck=\"false\">\
         <div class=\"results\" id=\"Results\" role=\"region\" aria-label=\"Search results\" aria-live=\"polite\" hidden></div></form>\
         <button type=\"button\" class=\"tbtn\" data-theme-button hidden><span aria-hidden=\"true\">◐</span> <span class=\"tbtn-word\">Theme</span></button>\
         </header>\n<main id=\"Main\" class=\"export\">\n<p class=\"xabout\">{}</p>\n",
        escape(parts.kind),
        escape(parts.first),
        parts.about
    );
    out.push_str(parts.pages);
    out.push_str("</main>\n");
    out.push_str("<script type=\"application/json\" id=\"ExportSettings\">");
    out.push_str(&script_json(&settings));
    out.push_str("</script>\n");
    if let Some(search) = parts.search {
        out.push_str("<script type=\"application/json\" id=\"SearchIndex\">");
        out.push_str(search);
        out.push_str("</script>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// The bytes a file holds besides its pages and its index: the style sheet,
/// the scripts and the frame. Prism and the font are counted where a page
/// first needs them.
pub(crate) fn frame_bytes() -> usize {
    style_sheet(false).len() + EXPORT_JS.len() + 4096
}

/// The bytes the diagram font adds to a file, as base64 in the style sheet.
pub(crate) fn font_bytes() -> usize {
    FONT.len().div_ceil(3) * 4 + 64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_sha256_in_base64() {
        // The hash a browser's console prints for an inline script holding
        // `alert('Hello, world.');`, as the CSP specification's example.
        assert_eq!(
            hash_source("alert('Hello, world.');"),
            "'sha256-qznLcsROx4GACP2dm0UCKCzCG+HiZ1guq6ZZDob/Tng='"
        );
    }

    #[test]
    fn the_font_is_inside_the_style_sheet_only_when_needed() {
        let without = style_sheet(false);
        assert!(!without.contains("@font-face"), "the face is dropped");
        assert!(!without.contains("url(\"__FONT__\")"));
        assert!(without.contains(".xabout"), "the export's rules follow");
        let with = style_sheet(true);
        assert!(with.contains("@font-face"));
        assert!(with.contains("url(\"data:font/woff2;base64,d09GMg"));
        assert!(!with.contains("url(\"__FONT__\")"));
        // The font as base64, plus its face rule, a few hundred bytes.
        assert!((with.len() - without.len()).abs_diff(font_bytes()) < 400);
    }

    #[test]
    fn the_file_allows_only_its_own_blocks() {
        let assets = Assets::built_in();
        let languages = vec![("sh".to_owned(), "bash".to_owned())];
        let prism = vec!["bash".to_owned()];
        let parts = Parts {
            title: "a <b>",
            kind: "doc",
            first: "R1",
            about: "About.",
            pages: "<section class=\"xpage\" id=\"R1\"><figure class=\"diagram\"></figure></section>\n",
            search: Some("{\"v\":1}"),
            languages: &languages,
            prism: &prism,
        };
        let html = file(&parts, &assets);
        let policy_start = html.find("content=\"default-src").unwrap() + "content=\"".len();
        let policy = &html[policy_start..policy_start + html[policy_start..].find('"').unwrap()];
        let css = style_sheet(true);
        let core =
            String::from_utf8_lossy(&assets.get("prism-core.js").unwrap().bytes).into_owned();
        let bash =
            String::from_utf8_lossy(&assets.get("prism-bash.js").unwrap().bytes).into_owned();
        assert_eq!(
            policy,
            format!(
                "default-src 'none'; style-src {}; script-src {} {} {}; font-src data:; base-uri 'none'; form-action 'none'",
                hash_source(&css),
                hash_source(EXPORT_JS),
                hash_source(&core),
                hash_source(&bash)
            )
        );
        assert!(html.contains(&format!("<style>{css}</style>")));
        assert!(html.contains("<title>a &lt;b&gt; · driftmark</title>"));
        assert!(html.contains("{\"kind\":\"doc\",\"first\":\"R1\",\"prism\":{\"sh\":\"bash\"}}"));
        assert!(
            html.contains(
                "<script type=\"application/json\" id=\"SearchIndex\">{\"v\":1}</script>"
            )
        );
        // The policy comes before every style and script.
        assert!(html.find("Content-Security-Policy").unwrap() < html.find("<style>").unwrap());
        // Without a diagram, code or index: no font, no Prism, no index.
        let plain = Parts {
            pages: "<section class=\"xpage\" id=\"R1\"></section>\n",
            search: None,
            languages: &[],
            prism: &[],
            ..parts
        };
        let html = file(&plain, &assets);
        assert!(!html.contains("font-src"));
        assert!(!html.contains("id=\"SearchIndex\""));
        assert_eq!(html.matches("<script>").count(), 1);
    }
}
