//! Finished drawings, kept by a hash of the diagram's language and text,
//! behind a byte cap.
//!
//! The same diagram drawn twice gives the same SVG, so nothing here ever goes
//! stale; entries leave only to stay under the cap, least recently used
//! first. Each entry keeps the diagram's text too, so two texts with the same
//! hash never share a drawing.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use driftmark_core::document::DiagramLanguage;

use super::Drawn;

/// A 64-bit FNV-1a hash of the language and the diagram's text. It is only a
/// key; the text is compared too.
pub(super) fn hash(language: DiagramLanguage, code: &str) -> u64 {
    let tag: &[u8] = match language {
        DiagramLanguage::Mermaid => b"mermaid\n",
        DiagramLanguage::D2 => b"d2\n",
    };
    tag.iter()
        .chain(code.as_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

struct Entry {
    language: DiagramLanguage,
    code: String,
    drawn: Drawn,
    bytes: usize,
    used: u64,
}

struct Inner {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    clock: u64,
}

/// The drawings, capped at a number of bytes.
pub(super) struct Drawings {
    cap: usize,
    inner: Mutex<Inner>,
}

impl Drawings {
    pub(super) fn new(cap: usize) -> Drawings {
        Drawings {
            cap,
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                bytes: 0,
                clock: 0,
            }),
        }
    }

    /// The drawing of this diagram: kept, or made by `draw` and kept. Two
    /// requests for the same new diagram at once may both draw it; the
    /// drawings are equal.
    pub(super) fn get_or_draw(
        &self,
        language: DiagramLanguage,
        code: &str,
        draw: impl FnOnce() -> Drawn,
    ) -> Drawn {
        let key = hash(language, code);
        {
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            inner.clock += 1;
            let now = inner.clock;
            if let Some(entry) = inner.entries.get_mut(&key)
                && entry.language == language
                && entry.code == code
            {
                entry.used = now;
                return entry.drawn.clone();
            }
        }
        let drawn = draw();
        let bytes = code.len()
            + match &drawn {
                Ok(Some(svg)) => svg.len(),
                Ok(None) => 0,
                Err(message) => message.len(),
            }
            + 64;
        if bytes > self.cap {
            return drawn;
        }
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.clock += 1;
        let used = inner.clock;
        if let Some(old) = inner.entries.insert(
            key,
            Entry {
                language,
                code: code.to_owned(),
                drawn: drawn.clone(),
                bytes,
                used,
            },
        ) {
            inner.bytes -= old.bytes;
        }
        inner.bytes += bytes;
        while inner.bytes > self.cap {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = inner.entries.remove(&oldest) {
                inner.bytes -= entry.bytes;
            }
        }
        drawn
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a_drawing_is_made_once_and_kept() {
        let drawings = Drawings::new(1024);
        let calls = Cell::new(0);
        let draw = || {
            calls.set(calls.get() + 1);
            Ok(Some(Arc::from("<svg/>")))
        };
        let first = drawings.get_or_draw(DiagramLanguage::Mermaid, "a", draw);
        let again = drawings.get_or_draw(DiagramLanguage::Mermaid, "a", || unreachable!());
        assert_eq!(first, again);
        assert_eq!(calls.get(), 1);
        let other = drawings.get_or_draw(DiagramLanguage::D2, "a", || Ok(None));
        assert_eq!(other, Ok(None));
    }

    #[test]
    fn the_cap_drops_the_least_recently_used() {
        let drawings = Drawings::new(3 * 70);
        for code in ["a", "b", "c"] {
            let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, code, || Ok(None));
        }
        // "a" is used again, so "b" is the oldest when "d" comes.
        let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, "a", || unreachable!());
        let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, "d", || Ok(None));
        let redrawn = Cell::new(false);
        let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, "b", || {
            redrawn.set(true);
            Ok(None)
        });
        assert!(redrawn.get());
        let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, "a", || unreachable!());
    }

    #[test]
    fn a_drawing_larger_than_the_cap_is_not_kept() {
        let drawings = Drawings::new(10);
        let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, "a", || Err("x".into()));
        let redrawn = Cell::new(false);
        let _ = drawings.get_or_draw(DiagramLanguage::Mermaid, "a", || {
            redrawn.set(true);
            Err("x".into())
        });
        assert!(redrawn.get());
    }

    #[test]
    fn hashes_tell_the_language_apart() {
        assert_ne!(
            hash(DiagramLanguage::Mermaid, "a"),
            hash(DiagramLanguage::D2, "a")
        );
        assert_eq!(
            hash(DiagramLanguage::Mermaid, "a"),
            hash(DiagramLanguage::Mermaid, "a")
        );
    }
}
