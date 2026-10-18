//! Makes a drawing safe to put into a page.
//!
//! A drawing's text comes from the repository, and merman's own guide says
//! its SVG needs cleaning before it reaches a page. So nothing passes as it
//! is: the SVG is read as XML and written out again from an allow-list.
//!
//! - **Elements.** Only shapes, text, groups, markers, gradients, clip paths,
//!   filters and `foreignObject` pass, and inside `foreignObject` only plain
//!   text elements such as `div`, `span`, `p` and `br`. Any other element is
//!   left out with everything inside it: `script`, `style`, `animate` and
//!   `set` (which can change a link), `image`, `iframe` and the rest.
//! - **Attributes.** Only those that draw. Event handlers (`on…`), `data-…`
//!   and anything unknown are left out.
//! - **Addresses.** A reference must point inside the drawing (`#id`,
//!   `url(#id)`); a link in `<a>` may also go to `http`, `https` or
//!   `mailto`. Anything else is left out, so no outside file is loaded.
//! - **Ids** all start with the drawing's own prefix, so a diagram can never
//!   take the id of a heading or of another diagram on the page.
//! - **`style` attributes** are blocked by the page's Content-Security-Policy,
//!   so they become what they mean without one: presentation attributes
//!   (`fill`, `stroke-width`) on shapes, and a few fixed classes on label
//!   text (`dm-wrap`, `dm-start`, `dm-end`) that the page style sheet reads.
//! - **The root `<svg>`** gets its natural width and height from its
//!   `viewBox` and a name for screen readers.

use std::borrow::Cow;
use std::fmt;

use quick_xml::Reader;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesRef, BytesStart, Event};

/// Why a drawing could not be made safe. The block then stays code, and
/// the browser draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanError(String);

impl fmt::Display for CleanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CleanError {}

fn error(message: impl Into<String>) -> CleanError {
    CleanError(message.into())
}

/// SVG elements a drawing may hold.
const SVG_ELEMENTS: &[&str] = &[
    "a",
    "circle",
    "clipPath",
    "defs",
    "desc",
    "ellipse",
    "feBlend",
    "feColorMatrix",
    "feComposite",
    "feDropShadow",
    "feFlood",
    "feGaussianBlur",
    "feMerge",
    "feMergeNode",
    "feOffset",
    "filter",
    "foreignObject",
    "g",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "title",
    "tspan",
    "use",
];

/// HTML elements a label inside `foreignObject` may hold.
const HTML_ELEMENTS: &[&str] = &[
    "b", "br", "code", "del", "div", "em", "i", "ins", "li", "ol", "p", "s", "small", "span",
    "strong", "sub", "sup", "u", "ul",
];

/// SVG attributes that draw, or name a part for screen readers.
const SVG_ATTRIBUTES: &[&str] = &[
    "alignment-baseline",
    "aria-describedby",
    "aria-hidden",
    "aria-label",
    "aria-labelledby",
    "aria-roledescription",
    "baseline-shift",
    "class",
    "clip-path",
    "clip-rule",
    "clipPathUnits",
    "color",
    "cx",
    "cy",
    "d",
    "display",
    "dominant-baseline",
    "dx",
    "dy",
    "fill",
    "fill-opacity",
    "fill-rule",
    "filter",
    "filterUnits",
    "flood-color",
    "flood-opacity",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "fr",
    "fx",
    "fy",
    "gradientTransform",
    "gradientUnits",
    "height",
    "href",
    "id",
    "in",
    "in2",
    "k1",
    "k2",
    "k3",
    "k4",
    "lengthAdjust",
    "letter-spacing",
    "marker-end",
    "marker-mid",
    "marker-start",
    "markerHeight",
    "markerUnits",
    "markerWidth",
    "mask",
    "maskContentUnits",
    "maskUnits",
    "mode",
    "offset",
    "opacity",
    "operator",
    "orient",
    "overflow",
    "paint-order",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "points",
    "preserveAspectRatio",
    "primitiveUnits",
    "r",
    "refX",
    "refY",
    "result",
    "role",
    "rotate",
    "rx",
    "ry",
    "shape-rendering",
    "spreadMethod",
    "startOffset",
    "stdDeviation",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "text-anchor",
    "text-decoration",
    "text-rendering",
    "textLength",
    "transform",
    "transform-origin",
    "type",
    "values",
    "vector-effect",
    "viewBox",
    "visibility",
    "width",
    "word-spacing",
    "writing-mode",
    "x",
    "x1",
    "x2",
    "xlink:href",
    "xml:space",
    "y",
    "y1",
    "y2",
];

/// Style properties that become presentation attributes of the same name.
const PRESENTATION: &[&str] = &[
    "alignment-baseline",
    "baseline-shift",
    "clip-path",
    "clip-rule",
    "color",
    "display",
    "dominant-baseline",
    "fill",
    "fill-opacity",
    "fill-rule",
    "filter",
    "flood-color",
    "flood-opacity",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "letter-spacing",
    "marker-end",
    "marker-mid",
    "marker-start",
    "mask",
    "opacity",
    "overflow",
    "paint-order",
    "shape-rendering",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "text-anchor",
    "text-decoration",
    "text-rendering",
    "vector-effect",
    "visibility",
    "word-spacing",
    "writing-mode",
];

/// Where an element sits: in the drawing, or in an HTML label inside a
/// `foreignObject`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Svg,
    Html,
}

/// One element written out, with the place its children sit in.
struct Open {
    name: String,
    inside: Place,
}

/// The cleaner's state while it reads.
struct Cleaner<'p> {
    prefix: &'p str,
    out: String,
    open: Vec<Open>,
    /// How deep the reader is inside an element that is left out.
    skipping: usize,
    root_seen: bool,
}

/// Cleans one drawing, as the module text says. `prefix` starts every id.
///
/// # Errors
///
/// When the drawing is not well-formed XML, its root is not `<svg>`, or it
/// uses an entity that XML does not define.
pub fn clean(svg: &str, prefix: &str) -> Result<String, CleanError> {
    let mut reader = Reader::from_str(svg);
    reader.config_mut().trim_text(false);
    let mut cleaner = Cleaner {
        prefix,
        out: String::with_capacity(svg.len() / 2),
        open: Vec::new(),
        skipping: 0,
        root_seen: false,
    };
    loop {
        let event = reader
            .read_event()
            .map_err(|problem| error(format!("the drawing is not well-formed XML: {problem}")))?;
        match event {
            Event::Start(element) => cleaner.start(&element, false)?,
            Event::Empty(element) => cleaner.start(&element, true)?,
            Event::End(_) => cleaner.end(),
            Event::Text(text) => {
                let text = text.xml_content(quick_xml::XmlVersion::Implicit1_0);
                cleaner.text(&text);
            }
            Event::CData(data) => {
                let text = data.into_inner();
                cleaner.text(&text);
            }
            Event::GeneralRef(reference) => {
                let character = resolve(&reference)?;
                cleaner.text(character.encode_utf8(&mut [0; 4]));
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }
    if !cleaner.root_seen {
        return Err(error("the drawing has no <svg> element"));
    }
    if !cleaner.open.is_empty() || cleaner.skipping > 0 {
        return Err(error("the drawing ends inside an element"));
    }
    Ok(cleaner.out)
}

/// The character an entity reference stands for: a number, one of XML's
/// five names, or `nbsp`, which HTML labels use.
fn resolve(reference: &BytesRef<'_>) -> Result<char, CleanError> {
    if let Some(character) = reference
        .resolve_char_ref()
        .map_err(|problem| error(format!("a character reference is wrong: {problem}")))?
    {
        return Ok(character);
    }
    let name = reference.clone().into_inner();
    match name.as_ref() {
        "lt" => Ok('<'),
        "gt" => Ok('>'),
        "amp" => Ok('&'),
        "quot" => Ok('"'),
        "apos" => Ok('\''),
        "nbsp" => Ok('\u{a0}'),
        other => Err(error(format!(
            "the drawing uses the unknown entity &{other};"
        ))),
    }
}

impl Cleaner<'_> {
    fn place(&self) -> Place {
        self.open.last().map_or(Place::Svg, |open| open.inside)
    }

    fn start(&mut self, element: &BytesStart<'_>, empty: bool) -> Result<(), CleanError> {
        if self.skipping > 0 {
            if !empty {
                self.skipping += 1;
            }
            return Ok(());
        }
        let name = element.name().as_ref().to_owned();
        let root = !self.root_seen;
        if root {
            if name != "svg" {
                return Err(error(format!("the drawing's root is <{name}>, not <svg>")));
            }
            self.root_seen = true;
        } else if self.open.is_empty() {
            return Err(error("the drawing has more than one root element"));
        }
        let place = self.place();
        let allowed = match place {
            Place::Svg => SVG_ELEMENTS.contains(&name.as_str()),
            Place::Html => HTML_ELEMENTS.contains(&name.as_str()),
        };
        if !allowed {
            if !empty {
                self.skipping = 1;
            }
            return Ok(());
        }
        let attributes = self.attributes(element, &name, place, root)?;
        self.out.push('<');
        self.out.push_str(&name);
        for (key, value) in &attributes {
            self.out.push(' ');
            self.out.push_str(key);
            self.out.push_str("=\"");
            escape_into(&mut self.out, value);
            self.out.push('"');
        }
        let inside = if name == "foreignObject" {
            Place::Html
        } else {
            place
        };
        match (empty, place) {
            // Inside `foreignObject` the browser reads HTML, where `<div/>`
            // does not close, so an empty element is written whole.
            (true, Place::Html) if name == "br" => self.out.push('>'),
            (true, Place::Html) => {
                self.out.push_str("></");
                self.out.push_str(&name);
                self.out.push('>');
            }
            (true, Place::Svg) => self.out.push_str("/>"),
            (false, _) => {
                self.out.push('>');
                self.open.push(Open { name, inside });
            }
        }
        Ok(())
    }

    fn end(&mut self) {
        if self.skipping > 0 {
            self.skipping -= 1;
            return;
        }
        if let Some(open) = self.open.pop()
            && open.name != "br"
        {
            self.out.push_str("</");
            self.out.push_str(&open.name);
            self.out.push('>');
        }
    }

    fn text(&mut self, text: &str) {
        if self.skipping == 0 && !self.open.is_empty() {
            escape_into(&mut self.out, text);
        }
    }

    /// The attributes to write: allowed ones, with ids and references
    /// checked, and `style` turned into what it means.
    fn attributes(
        &self,
        element: &BytesStart<'_>,
        name: &str,
        place: Place,
        root: bool,
    ) -> Result<Vec<(String, String)>, CleanError> {
        let mut kept: Vec<(String, String)> = Vec::new();
        let mut classes: Vec<String> = Vec::new();
        let mut view_box = None;
        let set = |kept: &mut Vec<(String, String)>, key: &str, value: String| {
            if let Some(found) = kept.iter_mut().find(|(have, _)| have == key) {
                found.1 = value;
            } else {
                kept.push((key.to_owned(), value));
            }
        };
        let mut styles: Vec<String> = Vec::new();
        for attribute in element.attributes() {
            let attribute: Attribute<'_> = attribute
                .map_err(|problem| error(format!("an attribute is not well-formed: {problem}")))?;
            let key = attribute.key.as_ref().to_owned();
            let value: Cow<'_, str> = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|problem| error(format!("an attribute value is wrong: {problem}")))?;
            match key.as_str() {
                "class" => classes.extend(value.split_whitespace().filter_map(class_name)),
                "style" => styles.push(value.into_owned()),
                "viewBox" if root => view_box = Some(value.into_owned()),
                _ if place == Place::Html => {}
                "id" => set(&mut kept, "id", self.id(&value)),
                "href" | "xlink:href" => {
                    if let Some(target) = self.address(&value, name == "a") {
                        set(&mut kept, &key, target);
                    }
                }
                _ if root && matches!(key.as_str(), "width" | "height") => {}
                _ if SVG_ATTRIBUTES.contains(&key.as_str()) => {
                    if let Some(value) = self.value(&value) {
                        set(&mut kept, &key, value);
                    }
                }
                _ => {}
            }
        }
        for style in styles {
            for (property, value) in declarations(&style) {
                match place {
                    Place::Svg if PRESENTATION.contains(&property.as_str()) => {
                        if let Some(value) = self.value(&value) {
                            set(&mut kept, &property, value);
                        }
                    }
                    Place::Svg => {}
                    Place::Html => {
                        let class = match (property.as_str(), value.as_str()) {
                            ("white-space", "break-spaces" | "normal" | "pre-wrap") => "dm-wrap",
                            ("text-align", "start" | "left") => "dm-start",
                            ("text-align", "end" | "right") => "dm-end",
                            _ => continue,
                        };
                        classes.push(class.to_owned());
                    }
                }
            }
        }
        if !classes.is_empty() {
            classes.dedup();
            kept.push(("class".to_owned(), classes.join(" ")));
        }
        if root {
            let size = view_box.as_deref().and_then(natural_size);
            if let Some(view_box) = view_box.as_deref().and_then(|found| self.value(found)) {
                kept.push(("viewBox".to_owned(), view_box));
            }
            if let Some((width, height)) = size {
                kept.push(("width".to_owned(), width));
                kept.push(("height".to_owned(), height));
            }
            let kind = kept
                .iter()
                .find(|(key, _)| key == "aria-roledescription")
                .map_or_else(String::new, |(_, kind)| kind.clone());
            if !kept.iter().any(|(key, _)| key == "aria-label") {
                kept.push(("aria-label".to_owned(), diagram_name(&kind)));
            }
            if !kept.iter().any(|(key, _)| key == "role") {
                kept.push(("role".to_owned(), "img".to_owned()));
            }
        }
        Ok(kept)
    }

    /// An id that starts with the drawing's prefix, made of characters that
    /// need no escaping in a CSS selector or an address.
    fn id(&self, id: &str) -> String {
        let safe: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if safe.starts_with(self.prefix) {
            safe
        } else {
            format!("{}-{safe}", self.prefix)
        }
    }

    /// A link or reference: `#id` inside the drawing, or, for a link in
    /// `<a>`, an address on the web or an email address.
    fn address(&self, address: &str, link: bool) -> Option<String> {
        let address = address.trim();
        if let Some(id) = address.strip_prefix('#') {
            return Some(format!("#{}", self.id(id)));
        }
        let scheme = address
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        let web = matches!(scheme.as_deref(), Some("http" | "https" | "mailto"));
        let plain = address
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace());
        (link && web && plain).then(|| address.to_owned())
    }

    /// An attribute value, with each `url(#id)` pointing inside the
    /// drawing. A value that refers to anything else, or that could escape a
    /// CSS value, is left out.
    fn value(&self, value: &str) -> Option<String> {
        let lower = value.to_ascii_lowercase();
        if lower.contains("expression(")
            || lower.contains("javascript:")
            || value.contains(['\\', '<', '>', ';', '{', '}'])
        {
            return None;
        }
        let mut out = String::new();
        let mut rest = value;
        while let Some(at) = rest.to_ascii_lowercase().find("url(") {
            out.push_str(&rest[..at]);
            let inner = &rest[at + 4..];
            let close = inner.find(')')?;
            let target = inner[..close].trim().trim_matches(['"', '\'']);
            let id = target.strip_prefix('#')?;
            out.push_str("url(#");
            out.push_str(&self.id(id));
            out.push(')');
            rest = &inner[close + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

/// A class name made only of letters, digits, `-` and `_`.
fn class_name(class: &str) -> Option<String> {
    (!class.is_empty()
        && class
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
    .then(|| class.to_owned())
}

/// The declarations of a `style` attribute, as lower-case property names and
/// trimmed values. `!important` is dropped.
fn declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let property = property.trim().to_ascii_lowercase();
            let value = value.trim().trim_end_matches("!important").trim();
            (!property.is_empty() && !value.is_empty()).then(|| (property, value.to_owned()))
        })
        .collect()
}

/// The width and height a `viewBox` gives, as numbers for attributes.
fn natural_size(view_box: &str) -> Option<(String, String)> {
    let numbers: Vec<f64> = view_box
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [_, _, width, height] = numbers[..] else {
        return None;
    };
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then(|| (format!("{}", width.ceil()), format!("{}", height.ceil())))
}

/// A name for screen readers from merman's `aria-roledescription`, such as
/// `flowchart-v2`: "Flowchart diagram".
fn diagram_name(kind: &str) -> String {
    let word: String = kind
        .split(['-', ' '])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphabetic)
        .collect();
    let name = match word.to_ascii_lowercase().as_str() {
        "" => return "Diagram".to_owned(),
        "er" => "Entity relationship".to_owned(),
        "flowchart" => "Flowchart".to_owned(),
        "sequence" => "Sequence".to_owned(),
        "class" => "Class".to_owned(),
        "statediagram" | "state" => "State".to_owned(),
        _ => {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        }
    };
    format!("{name} diagram")
}

/// Escapes text for an element's content or a quoted attribute value.
fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\0' => out.push('\u{fffd}'),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests;
