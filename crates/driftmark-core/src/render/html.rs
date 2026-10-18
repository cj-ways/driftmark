//! Raw HTML inside a document: reading its tags, and the allow-list that
//! decides which of them reach a page.
//!
//! The tags follow the grammar the Markdown standard gives raw HTML.
//! Anything that starts with `<` but is not a complete tag, comment or
//! declaration is text. The
//! writer in `reading.rs` turns the pieces into HTML: an allowed tag with its
//! allowed attributes, any other tag escaped so it shows as text, and
//! comments dropped.

/// Elements that can appear on a page when the rules file's `allowed-html`
/// names them. None of them runs code, loads a frame or changes how the
/// browser parses what follows, so naming `script` or `iframe` in the rules
/// file still shows them as text.
pub(crate) const RENDERABLE: [&str; 58] = [
    "a",
    "abbr",
    "b",
    "bdi",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// The renderable elements that may sit inside a paragraph. Any other
/// element in a paragraph would end the paragraph in the browser and move
/// the rest of its text out of the block, so there it is shown as text.
const PHRASING: [&str; 29] = [
    "a", "abbr", "b", "bdi", "br", "cite", "code", "del", "dfn", "em", "i", "img", "ins", "kbd",
    "mark", "q", "rp", "rt", "ruby", "s", "samp", "small", "span", "strong", "sub", "sup", "u",
    "var", "wbr",
];

/// Elements that have no closing tag.
const VOID: [&str; 4] = ["br", "hr", "img", "wbr"];

/// Where raw HTML stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// An HTML block, between other blocks.
    Flow,
    /// Inline HTML, inside a paragraph, a heading or a table cell.
    Phrasing,
}

/// The elements a page shows for one document: the rules file's list, kept
/// to the renderable ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Allowed {
    names: Vec<&'static str>,
}

impl Allowed {
    /// The renderable elements among `names`, which are compared in lower
    /// case.
    pub(crate) fn new(names: &[String]) -> Allowed {
        let names = RENDERABLE
            .into_iter()
            .filter(|element| names.iter().any(|name| name.eq_ignore_ascii_case(element)))
            .collect();
        Allowed { names }
    }

    /// The element's name as a static string when a page may show it here.
    pub(crate) fn element(&self, name: &str, place: Place) -> Option<&'static str> {
        let found = self
            .names
            .iter()
            .copied()
            .find(|element| *element == name)?;
        (place == Place::Flow || PHRASING.contains(&found)).then_some(found)
    }
}

/// Whether an element has no closing tag.
pub(crate) fn is_void(name: &str) -> bool {
    VOID.contains(&name)
}

/// What an allowed attribute holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Value {
    /// Text, written back escaped.
    Text,
    /// An address, checked and resolved like a Markdown link or image.
    Address,
    /// A flag such as `open`, written without a value.
    Flag,
}

/// The attributes kept on an allowed element. Every other attribute is
/// dropped: no `style`, no `class`, no `id` or `name` (which could replace
/// the page's own elements), and no event handlers.
pub(crate) fn attribute(element: &str, name: &str) -> Option<Value> {
    match (element, name) {
        (_, "title")
        | ("img", "alt" | "width" | "height")
        | ("td" | "th", "colspan" | "rowspan")
        | ("ol", "start") => Some(Value::Text),
        ("a", "href") | ("img", "src") => Some(Value::Address),
        ("details", "open") => Some(Value::Flag),
        _ => None,
    }
}

/// One piece of raw HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token<'h> {
    /// Text between tags, as written.
    Text(&'h str),
    /// An opening tag.
    Open {
        /// The element's name in lower case.
        name: String,
        /// The attributes in written order: the name in lower case, and the
        /// value as written, without quotes.
        attributes: Vec<(String, Option<&'h str>)>,
        /// The tag as written.
        source: &'h str,
    },
    /// A closing tag.
    Close {
        /// The element's name in lower case.
        name: String,
        /// The tag as written.
        source: &'h str,
    },
    /// A comment, a declaration, a processing instruction or a CDATA
    /// section. It renders nothing.
    Hidden,
}

/// Splits raw HTML into text, tags and hidden parts.
pub(crate) fn tokens(html: &str) -> Vec<Token<'_>> {
    let mut found = Vec::new();
    let mut text_start = 0;
    let mut at = 0;
    while let Some(offset) = html[at..].find('<') {
        let start = at + offset;
        match construct(&html[start..]) {
            Some((token, length)) => {
                if start > text_start {
                    found.push(Token::Text(&html[text_start..start]));
                }
                found.push(token);
                at = start + length;
                text_start = at;
            }
            None => at = start + 1,
        }
    }
    if html.len() > text_start {
        found.push(Token::Text(&html[text_start..]));
    }
    found
}

/// The tag, comment or declaration at the start of `rest`, which starts with
/// `<`, and its length in bytes.
fn construct(rest: &str) -> Option<(Token<'_>, usize)> {
    let hidden = |end: &str, from: usize| {
        let length = rest[from..]
            .find(end)
            .map_or(rest.len(), |at| from + at + end.len());
        Some((Token::Hidden, length))
    };
    if rest.starts_with("<!-->") {
        return Some((Token::Hidden, 5));
    }
    if rest.starts_with("<!--->") {
        return Some((Token::Hidden, 6));
    }
    if rest.starts_with("<!--") {
        return hidden("-->", 4);
    }
    if rest.starts_with("<![CDATA[") {
        return hidden("]]>", 9);
    }
    if rest.starts_with("<?") {
        return hidden("?>", 2);
    }
    if rest.starts_with("<!") && rest[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
        return hidden(">", 2);
    }
    if let Some(after) = rest.strip_prefix("</") {
        let name_length = tag_name(after)?;
        let end = 2 + name_length + spaces(&after[name_length..]);
        return rest[end..].starts_with('>').then(|| {
            let token = Token::Close {
                name: rest[2..2 + name_length].to_ascii_lowercase(),
                source: &rest[..=end],
            };
            (token, end + 1)
        });
    }
    open_tag(rest)
}

/// An opening tag at the start of `rest`.
fn open_tag(rest: &str) -> Option<(Token<'_>, usize)> {
    let name_length = tag_name(&rest[1..])?;
    let name = rest[1..=name_length].to_ascii_lowercase();
    let mut at = 1 + name_length;
    let mut attributes = Vec::new();
    loop {
        let space = spaces(&rest[at..]);
        let after_space = at + space;
        let tail = &rest[after_space..];
        if let Some(end) = tail
            .strip_prefix("/>")
            .map(|_| 2)
            .or_else(|| tail.starts_with('>').then_some(1))
        {
            let length = after_space + end;
            let token = Token::Open {
                name,
                attributes,
                source: &rest[..length],
            };
            return Some((token, length));
        }
        if space == 0 {
            return None;
        }
        let (attribute, length) = attribute_at(tail)?;
        attributes.push(attribute);
        at = after_space + length;
    }
}

/// One attribute at the start of `rest`: its name in lower case, its value
/// without quotes, and its length in bytes.
fn attribute_at(rest: &str) -> Option<((String, Option<&str>), usize)> {
    let name_length = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')))
        .unwrap_or(rest.len());
    let starts_well = rest.starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '_' | ':'));
    if !starts_well {
        return None;
    }
    let name = rest[..name_length].to_ascii_lowercase();
    let before_equals = name_length + spaces(&rest[name_length..]);
    let Some(after_equals) = rest[before_equals..].strip_prefix('=') else {
        return Some(((name, None), name_length));
    };
    let value_start = before_equals + 1 + spaces(after_equals);
    let value_rest = &rest[value_start..];
    let (value, length) = if let Some(quote @ ('"' | '\'')) = value_rest.chars().next() {
        let close = value_rest[1..].find(quote)?;
        (&value_rest[1..=close], close + 2)
    } else {
        let length = value_rest
            .find(|c: char| c.is_ascii_whitespace() || "\"'=<>`".contains(c))
            .unwrap_or(value_rest.len());
        if length == 0 {
            return None;
        }
        (&value_rest[..length], length)
    };
    Some(((name, Some(value)), value_start + length))
}

/// The length of a tag name at the start of `rest`: a letter, then letters,
/// digits and `-`.
fn tag_name(rest: &str) -> Option<usize> {
    rest.starts_with(|c: char| c.is_ascii_alphabetic())
        .then(|| {
            rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(rest.len())
        })
}

/// The length of the white space at the start of `rest`.
fn spaces(rest: &str) -> usize {
    rest.len()
        - rest
            .trim_start_matches(|c: char| c.is_ascii_whitespace())
            .len()
}

/// Writes text from raw HTML so that it shows as written: `<`, `>` and `"`
/// are escaped, and `&` is escaped unless it starts a character reference,
/// which the browser then decodes as the author meant.
pub(crate) fn push_text(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(at) = rest.find(['<', '>', '"', '&', '\0']) {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        match rest.as_bytes()[at] {
            b'<' => out.push_str("&lt;"),
            b'>' => out.push_str("&gt;"),
            b'"' => out.push_str("&quot;"),
            b'\0' => out.push('\u{fffd}'),
            _ if is_reference(after) => out.push('&'),
            _ => out.push_str("&amp;"),
        }
        rest = after;
    }
    out.push_str(rest);
}

/// Whether `after`, the text after an `&`, starts with a complete character
/// reference: `#` and digits, `#x` and hex digits, or a name, then `;`.
fn is_reference(after: &str) -> bool {
    let Some((body, _)) = after.split_once(';') else {
        return false;
    };
    let digits = |text: &str, hex: bool| {
        (1..=8).contains(&text.len())
            && text.chars().all(|c| {
                if hex {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_digit()
                }
            })
    };
    match body.strip_prefix('#') {
        Some(number) => match number.strip_prefix(['x', 'X']) {
            Some(hex) => digits(hex, true),
            None => digits(number, false),
        },
        None => {
            (2..=32).contains(&body.len())
                && body.starts_with(|c: char| c.is_ascii_alphabetic())
                && body.chars().all(|c| c.is_ascii_alphanumeric())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open<'h>(name: &str, attributes: &[(&str, Option<&'h str>)], source: &'h str) -> Token<'h> {
        Token::Open {
            name: name.to_owned(),
            attributes: attributes
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value))
                .collect(),
            source,
        }
    }

    #[test]
    fn tags_text_and_hidden_parts_are_told_apart() {
        let html =
            "<DIV class=\"a b\"\n  data-x='1' hidden open=yes/>text</Div >&amp;<!-- c --><br>";
        assert_eq!(
            tokens(html),
            [
                open(
                    "div",
                    &[
                        ("class", Some("a b")),
                        ("data-x", Some("1")),
                        ("hidden", None),
                        ("open", Some("yes/"))
                    ],
                    "<DIV class=\"a b\"\n  data-x='1' hidden open=yes/>"
                ),
                Token::Text("text"),
                Token::Close {
                    name: "div".to_owned(),
                    source: "</Div >"
                },
                Token::Text("&amp;"),
                Token::Hidden,
                open("br", &[], "<br>"),
            ]
        );
    }

    #[test]
    fn self_closing_tags_and_spaced_equals_are_read() {
        assert_eq!(
            tokens("<img src = \"a.png\" />"),
            [open(
                "img",
                &[("src", Some("a.png"))],
                "<img src = \"a.png\" />"
            )]
        );
        assert_eq!(tokens("<x a/>"), [open("x", &[("a", None)], "<x a/>")]);
    }

    #[test]
    fn hidden_parts_end_where_html_says() {
        for html in [
            "<!---->",
            "<!-->",
            "<!--->",
            "<!-- a -- b -->",
            "<?php echo 1 ?>",
            "<!DOCTYPE html>",
            "<![CDATA[ x < y ]]>",
        ] {
            assert_eq!(tokens(html), [Token::Hidden], "{html:?}");
        }
        assert_eq!(tokens("<!-- open"), [Token::Hidden]);
        assert_eq!(
            tokens("<!---->after"),
            [Token::Hidden, Token::Text("after")]
        );
    }

    #[test]
    fn broken_tags_are_text() {
        for html in [
            "a < b",
            "<",
            "<1x>",
            "<a",
            "<a b",
            "<a b=>",
            "<a b=\"x>",
            "<a b='x>",
            "<a=1>",
            "<a\"b>",
            "</",
            "</a",
            "</a b>",
            "</1>",
            "<!1>",
            "<a b=c d=`x`>",
        ] {
            assert_eq!(tokens(html), [Token::Text(html)], "{html:?}");
        }
        assert_eq!(
            tokens("x <y <b>"),
            [Token::Text("x <y "), open("b", &[], "<b>")]
        );
    }

    #[test]
    fn the_allow_list_keeps_renderable_elements_only() {
        let names = ["DETAILS", "script", "kbd", "iframe", "summary", "img"].map(str::to_owned);
        let allowed = Allowed::new(&names);
        assert_eq!(allowed.element("details", Place::Flow), Some("details"));
        assert_eq!(allowed.element("details", Place::Phrasing), None);
        assert_eq!(allowed.element("kbd", Place::Phrasing), Some("kbd"));
        assert_eq!(allowed.element("img", Place::Phrasing), Some("img"));
        assert_eq!(allowed.element("script", Place::Flow), None);
        assert_eq!(allowed.element("iframe", Place::Flow), None);
        assert_eq!(allowed.element("span", Place::Phrasing), None);
        assert!(is_void("br") && is_void("img") && !is_void("details"));
    }

    #[test]
    fn only_a_few_attributes_are_kept() {
        assert_eq!(attribute("abbr", "title"), Some(Value::Text));
        assert_eq!(attribute("img", "alt"), Some(Value::Text));
        assert_eq!(attribute("img", "width"), Some(Value::Text));
        assert_eq!(attribute("img", "height"), Some(Value::Text));
        assert_eq!(attribute("td", "colspan"), Some(Value::Text));
        assert_eq!(attribute("th", "rowspan"), Some(Value::Text));
        assert_eq!(attribute("ol", "start"), Some(Value::Text));
        assert_eq!(attribute("a", "href"), Some(Value::Address));
        assert_eq!(attribute("img", "src"), Some(Value::Address));
        assert_eq!(attribute("details", "open"), Some(Value::Flag));
        for (element, name) in [
            ("a", "src"),
            ("img", "href"),
            ("div", "style"),
            ("div", "class"),
            ("div", "id"),
            ("img", "name"),
            ("img", "onerror"),
            ("ol", "open"),
            ("li", "start"),
            ("div", "alt"),
        ] {
            assert_eq!(attribute(element, name), None, "{element} {name}");
        }
    }

    #[test]
    fn an_ampersand_with_no_semicolon_after_it_is_escaped() {
        let mut out = String::new();
        push_text(&mut out, "a & b &x");
        assert_eq!(out, "a &amp; b &amp;x");
    }

    #[test]
    fn text_keeps_references_and_escapes_the_rest() {
        let mut out = String::new();
        push_text(
            &mut out,
            "a < b > c \"q\" &amp; &#38; &#x26; &AMP; & &x; &#; &#x; &#123456789; &toolongnameeeeeeeeeeeeeeeeeeeeeeee; &a-b; \0",
        );
        assert_eq!(
            out,
            "a &lt; b &gt; c &quot;q&quot; &amp; &#38; &#x26; &AMP; &amp; &amp;x; &amp;#; &amp;#x; &amp;#123456789; &amp;toolongnameeeeeeeeeeeeeeeeeeeeeeee; &amp;a-b; \u{fffd}"
        );
    }
}
