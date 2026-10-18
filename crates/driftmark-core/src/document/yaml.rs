//! Reading YAML front matter into metadata values.
//!
//! The parser's events are turned into values here instead of through a YAML
//! document library, for two reasons. Every scalar stays text exactly as
//! written, so `007`, `1.10` and `2026-10-18` keep their spelling. And an
//! alias is copied only within a budget, so a "billion laughs" front matter
//! (aliases of aliases that double in size at each step) cannot exhaust
//! memory. saphyr-parser itself refuses nesting deeper than a few hundred
//! levels.

use std::collections::HashMap;

use saphyr_parser::{Event, Parser};

use super::MetaValue;

/// The most values one front matter may hold, counting every copy an alias
/// makes. Real front matter holds a few dozen.
pub(crate) const MAX_VALUES: usize = 10_000;

/// A collection still being read.
enum Frame {
    List {
        anchor: usize,
        items: Vec<MetaValue>,
    },
    Map {
        anchor: usize,
        entries: Vec<(String, MetaValue)>,
        key: Option<String>,
    },
}

/// Reads the YAML text of a front matter into its top-level keys and values,
/// in written order. Empty YAML gives no keys.
///
/// The error is a sentence for the reader: invalid YAML, a top level that is
/// not a mapping, a key that is not text, or more than [`MAX_VALUES`] values.
pub(crate) fn read(yaml: &str) -> Result<Vec<(String, MetaValue)>, String> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut anchors: HashMap<usize, MetaValue> = HashMap::new();
    let mut count = 0;
    let mut root = None;
    for event in Parser::new_from_str(yaml) {
        // The parser repeats its first error forever, so stop at it.
        // The YAML starts on the line after the opening `---`, so its line 1
        // is line 2 of the file.
        let (event, _) = event.map_err(|error| {
            let line = error.marker().line() + 1;
            format!(
                "the front matter is not valid YAML: {}, on line {line} of the file",
                error.info()
            )
        })?;
        // A collection's items were counted as they were read, so closing it
        // adds one value. An alias adds every value of its copy.
        let (value, anchor, added) = match event {
            Event::Scalar(text, _, anchor, _) => (MetaValue::Text(text.into_owned()), anchor, 1),
            Event::Alias(id) => {
                let copy = anchors
                    .get(&id)
                    .cloned()
                    .unwrap_or(MetaValue::Text(String::new()));
                let added = size(&copy);
                (copy, 0, added)
            }
            Event::SequenceStart(anchor, _) => {
                stack.push(Frame::List {
                    anchor,
                    items: Vec::new(),
                });
                continue;
            }
            Event::MappingStart(anchor, _) => {
                stack.push(Frame::Map {
                    anchor,
                    entries: Vec::new(),
                    key: None,
                });
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => {
                // The parser balances starts and ends, so the stack is never
                // empty here.
                match stack.pop().unwrap_or(Frame::List {
                    anchor: 0,
                    items: Vec::new(),
                }) {
                    Frame::List { anchor, items } => (MetaValue::List(items), anchor, 1),
                    Frame::Map {
                        anchor, entries, ..
                    } => (MetaValue::Map(entries), anchor, 1),
                }
            }
            Event::DocumentEnd => break,
            _ => continue,
        };
        count += added;
        if count > MAX_VALUES {
            return Err(format!(
                "the front matter holds more than {MAX_VALUES} values once its aliases are copied"
            ));
        }
        if anchor != 0 {
            anchors.insert(anchor, value.clone());
        }
        match stack.last_mut() {
            None => root = Some(value),
            Some(Frame::List { items, .. }) => items.push(value),
            Some(Frame::Map { entries, key, .. }) => match key.take() {
                Some(key) => entries.push((key, value)),
                None => match value {
                    MetaValue::Text(text) => *key = Some(text),
                    _ => return Err("a front matter key is not plain text".to_owned()),
                },
            },
        }
    }
    match root {
        None => Ok(Vec::new()),
        Some(MetaValue::Map(entries)) => Ok(entries),
        Some(_) => Err("the front matter is not a list of keys and values".to_owned()),
    }
}

/// How many values `value` holds, itself included.
fn size(value: &MetaValue) -> usize {
    let mut count = 0;
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        count += 1;
        match value {
            MetaValue::Text(_) => {}
            MetaValue::List(items) => pending.extend(items),
            MetaValue::Map(entries) => pending.extend(entries.iter().map(|(_, value)| value)),
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    fn text(value: &str) -> MetaValue {
        MetaValue::Text(value.to_owned())
    }

    #[test]
    fn scalars_stay_text_as_written() {
        let entries = read("status: accepted\nversion: 1.10\nid: 007\ndate: 2026-10-18\nempty:\n")
            .expect("valid YAML");
        assert_eq!(
            entries,
            vec![
                ("status".to_owned(), text("accepted")),
                ("version".to_owned(), text("1.10")),
                ("id".to_owned(), text("007")),
                ("date".to_owned(), text("2026-10-18")),
                ("empty".to_owned(), text("")),
            ]
        );
    }

    #[test]
    fn lists_and_maps_nest() {
        let entries =
            read("tags: [a, b]\nowner:\n  name: Ana\n  teams:\n    - core\n").expect("valid YAML");
        assert_eq!(
            entries,
            vec![
                (
                    "tags".to_owned(),
                    MetaValue::List(vec![text("a"), text("b")])
                ),
                (
                    "owner".to_owned(),
                    MetaValue::Map(vec![
                        ("name".to_owned(), text("Ana")),
                        ("teams".to_owned(), MetaValue::List(vec![text("core")])),
                    ])
                ),
            ]
        );
    }

    #[test]
    fn aliases_copy_their_anchor() {
        let entries = read("base: &b [x, y]\nsame: *b\none: &o z\nalso: *o\n").expect("valid YAML");
        assert_eq!(
            entries[1],
            (
                "same".to_owned(),
                MetaValue::List(vec![text("x"), text("y")])
            )
        );
        assert_eq!(entries[3], ("also".to_owned(), text("z")));
    }

    #[test]
    fn empty_yaml_has_no_keys() {
        assert_eq!(read(""), Ok(Vec::new()));
        assert_eq!(read("# only a comment\n"), Ok(Vec::new()));
    }

    #[test]
    fn only_the_first_yaml_document_counts() {
        assert_eq!(
            read("a: 1\n...\n---\nb: 2\n"),
            Ok(vec![("a".to_owned(), text("1"))])
        );
    }

    #[test]
    fn invalid_yaml_is_a_clear_error() {
        assert_eq!(
            read("a: b\nc: [1\nd: e\n"),
            Err("the front matter is not valid YAML: illegal placement of ':' indicator, on line 4 of the file".to_owned())
        );
    }

    #[test]
    fn the_top_level_must_be_keys_and_values() {
        assert_eq!(
            read("- a\n- b\n"),
            Err("the front matter is not a list of keys and values".to_owned())
        );
        assert_eq!(
            read("just text\n"),
            Err("the front matter is not a list of keys and values".to_owned())
        );
    }

    #[test]
    fn keys_must_be_text() {
        assert_eq!(
            read("? [a, b]\n: c\n"),
            Err("a front matter key is not plain text".to_owned())
        );
    }

    #[test]
    fn aliases_cannot_blow_up() {
        let mut yaml = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for level in 1..6 {
            let previous = level - 1;
            let items = vec![format!("*a{previous}"); 10].join(", ");
            writeln!(yaml, "a{level}: &a{level} [{items}]").expect("writing to a string");
        }
        let error = read(&yaml).expect_err("too many values");
        assert!(error.contains("more than 10000 values"), "{error}");
    }

    #[test]
    fn the_budget_allows_exactly_its_limit() {
        // 4,998 entries of two values, a key with a one-item list (three
        // values) and the top-level map: 10,000 values.
        let entries = (0..4998).fold(String::new(), |mut entries, n| {
            writeln!(entries, "k{n}: v").expect("writing to a string");
            entries
        });
        let exact = format!("{entries}z: [a]\n");
        assert_eq!(read(&exact).map(|entries| entries.len()), Ok(4999));
        let over = format!("{entries}z: [a, b]\n");
        assert!(read(&over).is_err());
    }

    #[test]
    fn size_counts_every_nested_value() {
        let value = MetaValue::Map(vec![
            ("a".to_owned(), MetaValue::List(vec![text("x"), text("y")])),
            ("b".to_owned(), text("z")),
        ]);
        assert_eq!(size(&value), 5);
    }
}
