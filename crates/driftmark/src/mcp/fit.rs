//! Size limits: every answer fits an agent's context.
//!
//! Claude Code warns when one tool answer passes 10,000 tokens, and saves
//! any text answer longer than 50,000 characters to a file instead of
//! showing it (docs/features/commands/mcp.md, "Size limits"). So each answer
//! here has a limit in characters, 16,000 unless the call asks for another,
//! and never more than 40,000. An answer that would be longer is cut: it
//! keeps the first part of its list or its text, and a `cut` object says
//! what was left out and gives the arguments of the call that continues.

use serde_json::{Map, Value, json};

/// The limit of an answer when the call names none, in characters. The
/// answers measured 2.0 to 2.7 characters a token with Claude's tokenizer,
/// so this is about 6,000 to 8,000 tokens: under Claude Code's warning at
/// 10,000.
pub(crate) const DEFAULT_CHARS: usize = 16_000;

/// The smallest limit a call may ask for. Below this, the parts of an
/// answer that are never cut, such as a range and its totals, may not fit.
pub(crate) const MIN_CHARS: usize = 2_000;

/// The largest limit a call may ask for: under the 50,000 characters past
/// which Claude Code saves an answer to a file, and at 2.0 characters a
/// token, under its 25,000-token limit.
pub(crate) const MAX_CHARS: usize = 40_000;

/// How many characters a text holds, counted as Claude Code counts them:
/// in UTF-16 units, so an emoji outside the basic plane counts two.
pub(crate) fn chars(text: &str) -> usize {
    text.encode_utf16().count()
}

/// How many characters a value takes as compact JSON.
pub(crate) fn size(value: &Value) -> usize {
    chars(&value.to_string())
}

/// How many characters `text` takes inside a JSON string, quotes not
/// counted: each character once, a quote, a backslash and the usual control
/// characters twice (`\n`), and any other control character six times
/// (`\u001f`), as `serde_json` writes them.
pub(crate) fn json_chars(text: &str) -> usize {
    text.chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if u32::from(c) < 0x20 => 6,
            c => c.len_utf16(),
        })
        .sum()
}

/// The most of a list that fits in `limit` characters.
///
/// `build(n)` makes the whole answer with the first `n` of `available`
/// items, and adds its `cut` when `n` is below `available`. `sizes` holds
/// each item's own size, so the first guess is close; the answer is then
/// built again with one item fewer until it fits. It returns the answer and
/// how many items it holds. When even no item fits, the answer with none is
/// returned, and the caller's final check refuses it.
pub(crate) fn most_that_fit(
    limit: usize,
    sizes: &[usize],
    build: impl Fn(usize) -> Value,
) -> (Value, usize) {
    let all = build(sizes.len());
    if size(&all) <= limit {
        return (all, sizes.len());
    }
    let room = limit.saturating_sub(size(&build(0)));
    let mut used = 0;
    let mut shown = sizes
        .iter()
        .take_while(|&&item| {
            used += item + 1;
            used <= room
        })
        .count();
    loop {
        let answer = build(shown);
        if shown == 0 || size(&answer) <= limit {
            return (answer, shown);
        }
        shown -= 1;
    }
}

/// The `cut` object: why part of the answer was left out, and, when more
/// can be asked for, the arguments of the call that continues.
pub(crate) fn cut(reason: &str, next: Option<Map<String, Value>>) -> Value {
    match next {
        Some(next) => json!({"reason": reason, "next": next}),
        None => json!({"reason": reason}),
    }
}

/// The part of `text` that fits in `limit` characters as a JSON string,
/// from its start: whole lines when at least one fits, otherwise as many
/// characters as fit. Returns the length in bytes.
pub(crate) fn text_that_fits(text: &str, limit: usize) -> usize {
    let mut used = 0;
    let mut end = 0;
    for line in text.split_inclusive('\n') {
        let cost = json_chars(line);
        if used + cost > limit {
            break;
        }
        used += cost;
        end += line.len();
    }
    if end > 0 || text.is_empty() {
        return end;
    }
    // Not even the first line fits, such as a long line of minified data:
    // cut inside it, at a character.
    let mut used = 0;
    text.char_indices()
        .find(|&(_, c)| {
            used += json_chars(c.encode_utf8(&mut [0; 4]));
            used > limit
        })
        .map_or(text.len(), |(at, _)| at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_count_what_json_writes() {
        for text in [
            "plain",
            "quote \" and \\ slash",
            "lines\nand\ttabs\r",
            "\u{1}\u{1f}\u{8}\u{c}",
            "ჯგუფი and 中文",
            "emoji 👩‍💻 ✅",
            "",
        ] {
            let written = serde_json::to_string(text).unwrap();
            assert_eq!(json_chars(text) + 2, chars(&written), "{text:?}");
            assert_eq!(size(&json!(text)), chars(&written));
        }
        assert_eq!(chars("👩"), 2);
    }

    #[test]
    fn a_list_keeps_the_most_items_that_fit() {
        let items: Vec<Value> = (0..50)
            .map(|n| json!({"n": n, "pad": "x".repeat(40)}))
            .collect();
        let sizes: Vec<usize> = items.iter().map(size).collect();
        let build = |n: usize| {
            let mut answer = json!({"items": items[..n]});
            if n < items.len() {
                answer["cut"] = cut(&format!("{n} shown"), None);
            }
            answer
        };
        let (all, shown) = most_that_fit(100_000, &sizes, build);
        assert_eq!(shown, 50);
        assert!(all.get("cut").is_none());
        let (some, shown) = most_that_fit(1_000, &sizes, build);
        assert!(size(&some) <= 1_000, "{}", size(&some));
        assert!(size(&build(shown + 1)) > 1_000);
        assert!(shown > 0 && shown < 50);
        assert_eq!(some["cut"]["reason"], format!("{shown} shown"));
        let (none, shown) = most_that_fit(10, &sizes, build);
        assert_eq!(shown, 0);
        assert_eq!(none["items"], json!([]));
    }

    #[test]
    fn the_first_guess_is_corrected_when_the_cut_grows() {
        // The cut grows with the items shown, so a guess made from the
        // answer with no item holds too many, and the answer is built again
        // with fewer.
        let item = json!("xxxxx");
        let sizes = vec![size(&item); 10];
        let build = |n: usize| {
            let mut answer = json!({"items": vec![item.clone(); n]});
            if n < 10 {
                answer["cut"] = json!("x".repeat(20 + n * 9));
            }
            answer
        };
        let (answer, shown) = most_that_fit(80, &sizes, build);
        assert!(size(&answer) <= 80);
        assert!(shown < 9);
        assert!(size(&build(shown + 1)) > 80, "{shown}");
        let room = 80 - size(&build(0));
        assert!(
            room / (size(&item) + 1) > shown,
            "the first guess was too high"
        );
    }

    #[test]
    fn text_is_cut_after_whole_lines_or_inside_one_long_line() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(text_that_fits(text, 100), text.len());
        assert_eq!(text_that_fits(text, 10), "one\ntwo\n".len());
        assert_eq!(text_that_fits(text, 9), "one\n".len());
        assert_eq!(text_that_fits("abcdef", 4), 4);
        assert_eq!(text_that_fits("ab\"cd", 3), 2);
        assert_eq!(text_that_fits("ჯგუფი", 2), "ჯგ".len());
        assert_eq!(text_that_fits("", 0), 0);
        assert_eq!(text_that_fits("abc", 0), 0);
        assert_eq!(text_that_fits("abc", 3), 3);
    }

    #[test]
    fn a_cut_names_its_next_call_when_there_is_one() {
        let mut next = Map::new();
        next.insert("offset".into(), json!(20));
        assert_eq!(
            cut("more", Some(next)),
            json!({"reason": "more", "next": {"offset": 20}})
        );
        assert_eq!(cut("none", None), json!({"reason": "none"}));
    }
}
