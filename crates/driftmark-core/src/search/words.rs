//! Where a text splits into words, and the form in which two words are
//! compared.
//!
//! Search splits text at Unicode's word boundaries (UAX #29), so it works for
//! any script that writes spaces or punctuation between words: English,
//! Georgian, Greek, Cyrillic and the rest. The change view has its own word
//! rule, which keeps `read-only` whole so an edit inside it shows as one
//! change. Search splits `read-only` into two words instead, so that `read`
//! finds it.

use unicode_segmentation::UnicodeSegmentation;

/// The longest word kept, in bytes after folding. A longer run of letters is
/// almost always a hash, a key or an inline image, and keeping it would only
/// grow the index. It still takes a place in its block, so the words on
/// either side of it are not next to each other.
pub const MAX_WORD_BYTES: usize = 100;

/// The words of `text` with the byte where each starts, in order.
///
/// A word is a stretch that Unicode's word boundaries leave whole and that
/// holds at least one letter or digit. So `don't`, `snake_case`, `v0.89` and
/// `a.md` are one word each, and `read-only` and `docs/a` are two.
/// Punctuation, spaces and emoji are not words. Chinese and Japanese text has
/// no spaces, and each ideograph is a word of its own.
pub(crate) fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    each_word(text, |at, word| out.push((at, word)));
    out
}

/// Calls `found` with each word of `text`, as [`words`] lists them, without
/// collecting them first.
///
/// Splitting at Unicode's word boundaries is the slowest part of building an
/// index, and most text is ASCII. So the text is cut into chunks, each ending
/// after ASCII white space that an ASCII character or the end follows.
/// Unicode puts a word boundary on both sides of such white space, so no
/// chunk splits a word. A chunk that is all ASCII goes through
/// [`ascii_words`], which applies the same rules five times faster; any
/// other chunk goes through the full Unicode rules.
pub(crate) fn each_word<'t>(text: &'t str, mut found: impl FnMut(usize, &'t str)) {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        let mut end = start;
        let mut ascii = true;
        while let Some(&byte) = bytes.get(end) {
            end += 1;
            ascii &= byte.is_ascii();
            if byte.is_ascii_whitespace() && bytes.get(end).is_none_or(u8::is_ascii) {
                break;
            }
        }
        let chunk = &text[start..end];
        if ascii {
            ascii_words(chunk, start, &mut found);
        } else {
            for (at, word) in chunk.unicode_word_indices() {
                found(start + at, word);
            }
        }
        start = end;
    }
}

/// The words of an ASCII chunk by the rules of Unicode's word boundaries
/// (UAX #29) that ASCII can meet: letters, digits and `_` stay together;
/// `:`, `.` and `'` join two letters, as in `a.md` and `don't`; `,`, `;`,
/// `.` and `'` join two digits, as in `3,000` and `3.14`. A run of `_`
/// alone is not a word. `base` is where the chunk starts in its text.
fn ascii_words<'t>(chunk: &'t str, base: usize, found: &mut impl FnMut(usize, &'t str)) {
    let bytes = chunk.as_bytes();
    let mut at = 0;
    while let Some(&first) = bytes.get(at) {
        if !joinable(first) {
            at += 1;
            continue;
        }
        let start = at;
        let mut letters = false;
        loop {
            while let Some(&byte) = bytes.get(at).filter(|&&byte| joinable(byte)) {
                letters |= byte != b'_';
                at += 1;
            }
            match (bytes.get(at), bytes.get(at + 1)) {
                (Some(&mid), Some(&after)) if joins(bytes[at - 1], mid, after) => at += 1,
                _ => break,
            }
        }
        if letters {
            found(base + start, &chunk[start..at]);
        }
    }
}

/// Letters, digits and `_`, which always stay together.
fn joinable(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether `mid` joins the characters on either side of it into one word.
fn joins(before: u8, mid: u8, after: u8) -> bool {
    let letters = before.is_ascii_alphabetic()
        && after.is_ascii_alphabetic()
        && matches!(mid, b':' | b'.' | b'\'');
    let digits = before.is_ascii_digit()
        && after.is_ascii_digit()
        && matches!(mid, b',' | b';' | b'.' | b'\'');
    letters || digits
}

/// Writes `word` into `out` in the form words are compared in, replacing
/// what `out` held.
///
/// Every letter takes its lower-case form, so `Cache`, `CACHE` and `cache`
/// are one word, and Georgian capitals (Mtavruli) match the ordinary
/// letters. Three more letters are folded the way Unicode case folding
/// does: `ß` and `ẞ` become `ss`, and the Greek final sigma `ς` becomes `σ`.
/// The typographic apostrophe `’` becomes `'`, so `don’t` and `don't` match.
pub(crate) fn fold_into(word: &str, out: &mut String) {
    out.clear();
    if word.is_ascii() {
        out.push_str(word);
        out.make_ascii_lowercase();
        return;
    }
    for character in word.chars() {
        match character {
            'ß' | 'ẞ' => out.push_str("ss"),
            'ς' => out.push('σ'),
            '\u{2019}' => out.push('\''),
            other => out.extend(other.to_lowercase()),
        }
    }
}

/// `word` in the form words are compared in.
pub(crate) fn fold(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    fold_into(word, &mut out);
    out
}

/// The words of `text` as search indexes them: in order, folded, and
/// without the words longer than [`MAX_WORD_BYTES`].
#[must_use]
pub fn terms(text: &str) -> Vec<String> {
    words(text)
        .into_iter()
        .map(|(_, word)| fold(word))
        .filter(|term| term.len() <= MAX_WORD_BYTES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str) -> Vec<(usize, &str)> {
        words(text)
    }

    #[test]
    fn words_follow_unicode_word_boundaries() {
        assert_eq!(
            split("Don't read-only snake_case v0.89 docs/a.md, 3.14!"),
            [
                (0, "Don't"),
                (6, "read"),
                (11, "only"),
                (16, "snake_case"),
                (27, "v0.89"),
                (33, "docs"),
                (38, "a.md"),
                (44, "3.14"),
            ]
        );
        assert_eq!(split("ქართული ტექსტი."), [(0, "ქართული"), (22, "ტექსტი")]);
        assert_eq!(split("👩‍💻 dev 🇬🇪 ✅"), [(12, "dev")]);
        assert_eq!(
            split("文章です"),
            [(0, "文"), (3, "章"), (6, "で"), (9, "す")]
        );
        assert_eq!(split("  -- … "), []);
    }

    #[test]
    fn the_fast_path_splits_exactly_as_unicode_does() {
        let pieces = [
            "a", "Z", "q", "0", "9", "_", ":", ".", "'", ",", ";", " ", "  ", "\t", "\n", "\r",
            "\u{b}", "\u{c}", "-", "/", "\"", "é", "’", "👍", "\u{301}", "\u{200d}", "ქ", "文",
            "ä", "\u{a0}",
        ];
        let mut seed: u64 = 0x5EED;
        let mut next = |below: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(seed >> 33).unwrap() % below
        };
        for round in 0..20_000 {
            // Every fifth text is ASCII only, to test the fast path alone.
            let choices = if round % 5 == 0 { 21 } else { pieces.len() };
            let text: String = (0..next(24)).map(|_| pieces[next(choices)]).collect();
            let expected: Vec<(usize, &str)> = text.unicode_word_indices().collect();
            assert_eq!(words(&text), expected, "{text:?}");
        }
    }

    #[test]
    fn ascii_rules_join_and_split_as_unicode_does() {
        let cases: [(&str, &[&str]); 9] = [
            ("e.g. a:b x_y __ _a", &["e.g", "a:b", "x_y", "_a"]),
            ("3,000;5 1.2.3 4'5", &["3,000;5", "1.2.3", "4'5"]),
            (
                "a.1 1.a a,b 1:2 a;b",
                &["a", "1", "1", "a", "a", "b", "1", "2", "a", "b"],
            ),
            ("a_.b a._b", &["a_", "b", "a", "_b"]),
            ("end. 'quoted' x'", &["end", "quoted", "x"]),
            ("a\u{b}b a\u{c}b", &["a", "b", "a", "b"]),
            ("café’s naïve", &["café’s", "naïve"]),
            ("x \u{301}y", &["x", "y"]),
            ("", &[]),
        ];
        for (text, expected) in cases {
            let found: Vec<&str> = words(text).into_iter().map(|(_, word)| word).collect();
            assert_eq!(found, expected, "{text:?}");
            let unicode: Vec<&str> = text.unicode_words().collect();
            assert_eq!(found, unicode, "{text:?}");
        }
    }

    #[test]
    fn folding_lowers_case_in_every_script() {
        assert_eq!(fold("Cache"), "cache");
        assert_eq!(fold("HTTP2"), "http2");
        assert_eq!(fold("ᲥᲐᲠᲗᲣᲚᲘ"), "ქართული");
        assert_eq!(fold("ΟΔΟΣ"), "οδοσ");
        assert_eq!(fold("οδος"), "οδοσ");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("GROẞ"), "gross");
        assert_eq!(fold("Don’t"), "don't");
        assert_eq!(fold("Ünïcode"), "ünïcode");
        assert_eq!(fold("İ"), "i\u{307}");
    }

    #[test]
    fn terms_are_the_folded_words_that_are_kept() {
        let long = "x".repeat(MAX_WORD_BYTES + 1);
        let exact = "Y".repeat(MAX_WORD_BYTES);
        assert_eq!(
            terms(&format!("Straße {long} {exact} ✅")),
            ["strasse".to_owned(), "y".repeat(MAX_WORD_BYTES)]
        );
    }

    #[test]
    fn folding_into_a_buffer_replaces_what_it_held() {
        let mut out = String::from("old text");
        fold_into("New", &mut out);
        assert_eq!(out, "new");
        fold_into("Ωμέγα", &mut out);
        assert_eq!(out, "ωμέγα");
    }
}
