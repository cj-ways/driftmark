//! Which requests count as the reader's page (decision 0012).
//!
//! Every HTML page the server sends carries a new random marker in
//! `<meta name="driftmark-page">`. The page script sends it back in the
//! `Driftmark-Page` header with every write. A write counts as coming from
//! the page only when it has all three of:
//!
//! - the token cookie, which the access rule already asks of every write;
//! - `Sec-Fetch-Site: same-origin`, which a browser sets and a page on
//!   another site cannot;
//! - a marker this server handed out, less than [`LIFE`] ago, and still
//!   among the last [`MOST`] it handed out.
//!
//! Every other write with the token counts as the API. The marker is no
//! secret from a program that loads a page; it makes acting as the page a
//! deliberate act, and such an act still shows as the API unless it also
//! fakes a browser's headers.
//!
//! A page whose marker is old asks, with `Driftmark-Page-Only`, to be
//! refused rather than recorded as the API; it then loads a fresh marker
//! and tries again.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use driftmark_core::notes::Source;

/// The header the page script sends its marker in.
pub(crate) const HEADER: &str = "driftmark-page";

/// The header with which the page script asks to be refused, with `412`,
/// when its write would not count as the page.
pub(crate) const ONLY_HEADER: &str = "driftmark-page-only";

/// How long a marker counts: one hour after its page was sent.
pub(crate) const LIFE: Duration = Duration::from_hours(1);

/// How many markers the server keeps. The oldest goes first.
pub(crate) const MOST: usize = 1024;

/// The markers the server handed out, oldest first.
pub(crate) struct Markers {
    life: Duration,
    most: usize,
    issued: Mutex<VecDeque<(String, Instant)>>,
}

impl Markers {
    /// No markers yet; each lives `life` and at most `most` are kept.
    pub(crate) fn new(life: Duration, most: usize) -> Markers {
        Markers {
            life,
            most,
            issued: Mutex::new(VecDeque::new()),
        }
    }

    /// A new marker for one page, made with `random`; `None` when no random
    /// marker could be made, and then the page's writes count as the API.
    pub(crate) fn issue(&self, random: impl FnOnce() -> Result<String, String>) -> Option<String> {
        let marker = random().ok()?;
        self.keep(marker.clone(), Instant::now());
        Some(marker)
    }

    fn keep(&self, marker: String, now: Instant) {
        let mut issued = self.issued.lock().unwrap_or_else(PoisonError::into_inner);
        self.drop_old(&mut issued, now);
        while issued.len() >= self.most {
            issued.pop_front();
        }
        issued.push_back((marker, now));
    }

    fn drop_old(&self, issued: &mut VecDeque<(String, Instant)>, now: Instant) {
        while issued
            .front()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= self.life)
        {
            issued.pop_front();
        }
    }

    /// Whether `marker` was handed out and still counts.
    pub(crate) fn known(&self, marker: &str) -> bool {
        self.known_at(marker, Instant::now())
    }

    fn known_at(&self, marker: &str, now: Instant) -> bool {
        let mut issued = self.issued.lock().unwrap_or_else(PoisonError::into_inner);
        self.drop_old(&mut issued, now);
        !marker.is_empty() && issued.iter().any(|(kept, _)| kept == marker)
    }

    /// Where a write with these headers came from: the page, or the API.
    /// The access rule has already checked its token cookie.
    pub(crate) fn source(&self, headers: &HeaderMap) -> Source {
        let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let same_origin = text("sec-fetch-site") == Some("same-origin");
        if same_origin && text(HEADER).is_some_and(|marker| self.known(marker)) {
            Source::Page
        } else {
            Source::Api
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn counter() -> impl FnMut() -> Result<String, String> {
        let mut next = 0;
        move || {
            next += 1;
            Ok(format!("m{next}"))
        }
    }

    #[test]
    fn a_marker_counts_for_its_life_only() {
        let markers = Markers::new(Duration::from_secs(60), 10);
        let start = Instant::now();
        markers.keep("a".into(), start);
        markers.keep("b".into(), start + Duration::from_secs(30));
        assert!(markers.known_at("a", start + Duration::from_secs(59)));
        assert!(!markers.known_at("a", start + Duration::from_secs(60)));
        assert!(markers.known_at("b", start + Duration::from_secs(60)));
        assert!(!markers.known_at("b", start + Duration::from_secs(90)));
        assert!(!markers.known_at("", start));
        assert!(!markers.known_at("c", start));
    }

    #[test]
    fn only_the_last_markers_are_kept() {
        let markers = Markers::new(LIFE, 3);
        let mut random = counter();
        let made: Vec<String> = (0..4).filter_map(|_| markers.issue(&mut random)).collect();
        assert_eq!(made, ["m1", "m2", "m3", "m4"]);
        assert!(!markers.known("m1"), "the oldest went first");
        for marker in &made[1..] {
            assert!(markers.known(marker), "{marker}");
        }
        assert_eq!(markers.issue(|| Err("no randomness".into())), None);
        assert!(markers.known("m2"), "a failed marker drops nothing");
    }

    #[test]
    fn a_write_is_the_page_only_with_both_signs() {
        let markers = Markers::new(LIFE, MOST);
        let marker = markers.issue(|| Ok("m1".into())).unwrap();
        let headers = |pairs: &[(&'static str, &str)]| {
            let mut map = HeaderMap::new();
            for (name, value) in pairs {
                map.append(*name, HeaderValue::from_str(value).unwrap());
            }
            map
        };
        let site = ("sec-fetch-site", "same-origin");
        assert_eq!(
            markers.source(&headers(&[site, (HEADER, &marker)])),
            Source::Page
        );
        for pairs in [
            &[(HEADER, marker.as_str())][..],
            &[site],
            &[site, (HEADER, "m2")],
            &[("sec-fetch-site", "same-site"), (HEADER, &marker)],
            &[("sec-fetch-site", "none"), (HEADER, &marker)],
            &[],
        ] {
            assert_eq!(markers.source(&headers(pairs)), Source::Api, "{pairs:?}");
        }
    }
}
