//! Which link and image addresses a page may hold.
//!
//! The rule is an allow-list. An address is kept when it points into the
//! same page, into the repository, or to the web or an email address. Every
//! other scheme is refused, so `javascript:`, `data:`, `vbscript:` and `file:`
//! never reach a page, however they are spelled.

/// What an address is, for deciding what the page does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Address {
    /// `#part`: a place on the same page. It is kept as written.
    Fragment,
    /// A path such as `setup.md`, `../img/cat.png` or `/docs/a.md`, or an
    /// empty address. The [`super::LinkResolver`] decides where it leads.
    Relative,
    /// `http:`, `https:` or `mailto:`, or `//host/…`. It is kept as written.
    Web,
    /// Any other scheme. The address is dropped.
    Refused,
}

/// The schemes a page may link to.
const WEB_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Classifies an address the way a browser reads it.
///
/// A browser drops control characters and spaces at both ends, and tabs and
/// line breaks anywhere, before it looks for a scheme. So `java\tscript:`
/// is `javascript:` to a browser, and here too.
pub(crate) fn classify(address: &str) -> Address {
    let cleaned: String = address
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    if cleaned.starts_with('#') {
        return Address::Fragment;
    }
    if cleaned.starts_with("//") {
        return Address::Web;
    }
    match scheme(&cleaned) {
        None => Address::Relative,
        Some(scheme) if WEB_SCHEMES.contains(&scheme.as_str()) => Address::Web,
        Some(_) => Address::Refused,
    }
}

/// The scheme of an address in lower case, when it has one: a letter, then
/// letters, digits, `+`, `-` or `.`, then a colon.
fn scheme(address: &str) -> Option<String> {
    let (scheme, _) = address.split_once(':')?;
    let mut characters = scheme.chars();
    let starts_with_letter = characters.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_valid = characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    (starts_with_letter && rest_valid).then(|| scheme.to_ascii_lowercase())
}

/// Decodes the character references in an attribute value of raw HTML, as a
/// browser does before it reads the value as an address.
///
/// Numeric references and the five XML names (`amp`, `lt`, `gt`, `quot`,
/// `apos`) are decoded. Any other named reference gives `None`, because an
/// address that hides characters behind names such as `&colon;` is refused
/// rather than guessed at.
pub(crate) fn decode_attribute(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (decoded, used) = reference(after)?;
        out.push_str(&decoded);
        rest = &after[used..];
    }
    out.push_str(rest);
    Some(out)
}

/// The character a reference stands for and how many bytes after the `&`
/// it used. A lone `&` stands for itself. `None` for an unknown name.
fn reference(after: &str) -> Option<(String, usize)> {
    let name_length = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))
        .unwrap_or(after.len());
    let name = &after[..name_length];
    let used = name_length + usize::from(after[name_length..].starts_with(';'));
    if let Some(number) = name.strip_prefix('#') {
        let value = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => number.parse::<u32>().ok(),
        };
        let character = value.and_then(char::from_u32).unwrap_or('\u{fffd}');
        return Some((character.to_string(), used));
    }
    let character = match name {
        "" => return Some(("&".to_owned(), 0)),
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    };
    Some((character.to_owned(), used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_classified_as_a_browser_reads_them() {
        for (address, expected) in [
            ("#install", Address::Fragment),
            (" \u{1}#x", Address::Fragment),
            ("setup.md#install", Address::Relative),
            ("../img/cat.png", Address::Relative),
            ("/docs/a.md", Address::Relative),
            ("", Address::Relative),
            ("a:b/c", Address::Refused),
            ("1x:y", Address::Relative),
            ("a b:c", Address::Relative),
            ("x+y.z-w:q", Address::Refused),
            ("https://example.com/a", Address::Web),
            ("HTTP://EXAMPLE.COM", Address::Web),
            ("mailto:team@example.com", Address::Web),
            ("//example.com/x", Address::Web),
            ("javascript:alert(1)", Address::Refused),
            ("JavaScript:alert(1)", Address::Refused),
            ("java\tscript:alert(1)", Address::Refused),
            ("java\nscript:alert(1)", Address::Refused),
            ("java\rscript:alert(1)", Address::Refused),
            ("\u{0} javascript:alert(1) ", Address::Refused),
            ("data:image/png;base64,AAAA", Address::Refused),
            ("vbscript:msgbox", Address::Refused),
            ("file:///etc/passwd", Address::Refused),
        ] {
            assert_eq!(classify(address), expected, "{address:?}");
        }
    }

    #[test]
    fn attribute_values_are_decoded_before_they_are_read() {
        assert_eq!(
            decode_attribute("a&amp;b&lt;&gt;&quot;&apos;").as_deref(),
            Some("a&b<>\"'")
        );
        assert_eq!(
            decode_attribute("&#106;ava&#X73;cript&#x3a;").as_deref(),
            Some("javascript:")
        );
        assert_eq!(decode_attribute("&#106").as_deref(), Some("j"));
        assert_eq!(decode_attribute("x & y").as_deref(), Some("x & y"));
        assert_eq!(decode_attribute("x&").as_deref(), Some("x&"));
        assert_eq!(decode_attribute("&#;").as_deref(), Some("\u{fffd}"));
        assert_eq!(decode_attribute("&#xD800;").as_deref(), Some("\u{fffd}"));
        assert_eq!(
            decode_attribute("&#99999999999;").as_deref(),
            Some("\u{fffd}")
        );
        assert_eq!(decode_attribute("javascript&colon;x"), None);
        assert_eq!(decode_attribute("plain").as_deref(), Some("plain"));
    }
}
