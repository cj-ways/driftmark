//! Who may ask the server, and what every answer carries.
//!
//! The server always has a token (`token.rs`). A browser presents it once as
//! `?token=`; the server answers with an `HttpOnly`, `SameSite=Strict`
//! cookie and sends the browser to the same address without the token, so
//! it does not stay in the history or in a shared link.
//!
//! - **Every write** (any method but `GET` and `HEAD`) needs the token
//!   cookie, on every address (decision 0012). Without it the answer is
//!   `401`, with a sentence that says how to get it.
//! - **Reads on this machine** (`127.0.0.1` or `::1`, with a token made at
//!   start) need no token, but must name a local host, such as
//!   `localhost:4170`. A web page elsewhere that points its own name at
//!   `127.0.0.1` (DNS rebinding) is refused, because its requests carry its
//!   own name.
//! - **Reads on any other address, or with a token that was given**, need
//!   the token too.
//! - **Requests with a body** must be JSON and must not come from another
//!   site's page.
//! - **Every answer** carries a content security policy that allows only the
//!   program's own scripts and styles, and no CORS header.

use std::net::{IpAddr, SocketAddr};

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use percent_encoding::percent_decode_str;

/// How long a browser keeps the token cookie: 30 days.
pub(crate) const COOKIE_SECONDS: u32 = 30 * 24 * 60 * 60;

/// The content security policy of every answer: the page may load only the
/// program's own scripts, styles, images and fonts, connect only to this
/// server, and may not be framed.
pub(crate) const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

/// What the server asks of every request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Access {
    /// The token every write needs.
    token: String,
    /// Whether reads need the token too: on an address beyond this machine,
    /// or when the token was given rather than made at start. Otherwise
    /// reads need a local `Host` instead.
    guard_reads: bool,
    /// The cookie's name. It holds the port, because a browser sends a
    /// host's cookies to every port, and two servers must not share one.
    cookie: String,
}

/// What to do with one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Answer it.
    Pass,
    /// It brought the right token in the address: set the cookie and send
    /// the browser to `location`.
    Remember {
        /// The same address without the token.
        location: String,
        /// The `Set-Cookie` value.
        cookie: String,
    },
    /// It has no token, or a wrong one.
    NeedToken {
        /// Whether a token was given and was wrong.
        wrong: bool,
        /// Whether the request writes, so its answer is JSON for a script.
        write: bool,
    },
    /// It names a host other than this machine, on a server bound to this
    /// machine only.
    OtherHost,
}

impl Access {
    /// The rules for a server bound to `addr` with `token`. `given` says
    /// whether the token was given, rather than made at start.
    pub(crate) fn new(addr: SocketAddr, token: String, given: bool) -> Access {
        Access {
            token,
            guard_reads: given || !addr.ip().is_loopback(),
            cookie: format!("driftmark-{}", addr.port()),
        }
    }

    /// The cookie's name, such as `driftmark-4170`.
    pub(crate) fn cookie_name(&self) -> &str {
        &self.cookie
    }

    /// Decides what to do with a request.
    pub(crate) fn check(&self, method: &Method, uri: &Uri, headers: &HeaderMap) -> Verdict {
        let write = !matches!(*method, Method::GET | Method::HEAD);
        if !self.guard_reads
            && !headers
                .get(header::HOST)
                .and_then(|host| host.to_str().ok())
                .is_some_and(is_local_host)
        {
            return Verdict::OtherHost;
        }
        if let Some(given) = query_token(uri) {
            if !write && same(given.as_bytes(), self.token.as_bytes()) {
                return Verdict::Remember {
                    location: without_token(uri),
                    cookie: format!(
                        "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={COOKIE_SECONDS}",
                        self.cookie, self.token
                    ),
                };
            }
            return Verdict::NeedToken { wrong: true, write };
        }
        let cookies = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='));
        let mut wrong = false;
        for (name, value) in cookies {
            if name == self.cookie {
                if same(value.as_bytes(), self.token.as_bytes()) {
                    return Verdict::Pass;
                }
                wrong = true;
            }
        }
        if write || self.guard_reads {
            Verdict::NeedToken { wrong, write }
        } else {
            Verdict::Pass
        }
    }
}

/// Why a write without the token was refused, and how to get it: the body
/// of its `401`, which the page shows in place of the failed save.
/// `cookie` is the cookie's name, for a program without a browser.
pub(crate) fn token_to_write(wrong: bool, cookie: &str) -> String {
    let first = if wrong {
        "The token this browser holds is not this server's token: the server was started again with a new one."
    } else {
        "Saving notes and seen marks needs the reader's token."
    };
    format!(
        "{first} Open the address that driftmark serve printed when it started, the one that ends in ?token=, in this browser, then try again. A new tab keeps this page as it is. A program without a browser sends the token in the cookie {cookie}."
    )
}

/// Whether a `Host` header names this machine: `localhost`, a name under
/// `.localhost`, an address in `127.0.0.0/8`, or `[::1]`, with any port.
/// The port is not checked, so a tunnel to another local port still works.
fn is_local_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map_or(rest, |(inside, _)| inside)
    } else {
        host.rsplit_once(':').map_or(host, |(name, _)| name)
    };
    let name = name.to_ascii_lowercase();
    name == "localhost"
        || name.ends_with(".localhost")
        || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The `token` value in the query, decoded.
fn query_token(uri: &Uri) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        (name == "token").then(|| percent_decode_str(value).decode_utf8_lossy().into_owned())
    })
}

/// The request's path and query without the `token` part.
fn without_token(uri: &Uri) -> String {
    let rest: Vec<&str> = uri
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|pair| !pair.is_empty() && pair.split('=').next() != Some("token"))
        .collect();
    if rest.is_empty() {
        uri.path().to_owned()
    } else {
        format!("{}?{}", uri.path(), rest.join("&"))
    }
}

/// Compares two secrets in time that does not depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// Whether a request with a body may be handled. The server's own pages
/// send JSON from their own origin; a page on another site is refused before
/// it reaches a route. Requests without a body pass. A request without an
/// `Origin`, such as one from a command-line tool, passes here; it needed
/// the token cookie already, and it counts as the API.
///
/// # Errors
///
/// The status and the reason to refuse with.
pub(crate) fn check_body(method: &Method, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    if !matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        return Ok(());
    }
    let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let json = text(header::CONTENT_TYPE.as_str()).is_some_and(|kind| {
        kind.split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "this server takes requests with a body only as JSON".to_owned(),
        ));
    }
    if let Some(origin) = text(header::ORIGIN.as_str()) {
        let own = text(header::HOST.as_str()).is_some_and(|host| {
            origin
                .strip_prefix("http://")
                .is_some_and(|rest| rest.eq_ignore_ascii_case(host))
        });
        if !own {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "requests from `{origin}` are refused; only this server's own pages may send them"
                ),
            ));
        }
    }
    if let Some(site) = text("sec-fetch-site")
        && !matches!(site, "same-origin" | "none")
    {
        return Err((
            StatusCode::FORBIDDEN,
            format!("requests from a {site} page are refused"),
        ));
    }
    Ok(())
}

/// Adds the security headers every answer carries. No CORS header is ever
/// added, so a page on another site cannot read an answer.
pub(crate) fn add_security_headers(headers: &mut HeaderMap) {
    let set = [
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (
            header::HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        ),
        (
            header::HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ),
    ];
    for (name, value) in set {
        headers.insert(name, HeaderValue::from_static(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123";

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn check(
        access: &Access,
        method: &Method,
        uri: &str,
        pairs: &[(&'static str, &str)],
    ) -> Verdict {
        access.check(method, &uri.parse().unwrap(), &headers(pairs))
    }

    #[test]
    fn a_local_server_answers_only_to_local_host_names() {
        let access = Access::new("127.0.0.1:4170".parse().unwrap(), TOKEN.into(), false);
        for host in [
            "localhost:4170",
            "LOCALHOST",
            "docs.localhost:9",
            "127.0.0.1:4170",
            "127.1.2.3",
            "[::1]:4170",
            "[::1]",
        ] {
            assert_eq!(
                check(&access, &Method::GET, "/", &[("host", host)]),
                Verdict::Pass,
                "{host}"
            );
        }
        for host in [
            "evil.example:4170",
            "100.64.0.1:4170",
            "[::2]:1",
            "localhost.evil",
            "",
        ] {
            assert_eq!(
                check(&access, &Method::GET, "/", &[("host", host)]),
                Verdict::OtherHost,
                "{host}"
            );
        }
        assert_eq!(check(&access, &Method::GET, "/", &[]), Verdict::OtherHost);
        let cookie = format!("driftmark-4170={TOKEN}");
        assert_eq!(
            check(
                &access,
                &Method::POST,
                "/api/notes",
                &[("host", "evil.example"), ("cookie", &cookie)]
            ),
            Verdict::OtherHost,
            "the host is checked before the cookie"
        );
    }

    #[test]
    fn a_local_server_reads_freely_and_writes_with_the_cookie_only() {
        let access = Access::new("127.0.0.1:4170".parse().unwrap(), TOKEN.into(), false);
        let host = ("host", "localhost:4170");
        let good = format!("driftmark-4170={TOKEN}");
        assert_eq!(check(&access, &Method::GET, "/", &[host]), Verdict::Pass);
        assert_eq!(
            check(
                &access,
                &Method::HEAD,
                "/",
                &[host, ("cookie", "driftmark-4170=old")]
            ),
            Verdict::Pass,
            "an old cookie does not stop a read"
        );
        for method in [Method::POST, Method::PATCH, Method::PUT, Method::DELETE] {
            assert_eq!(
                check(&access, &method, "/api/notes", &[host]),
                Verdict::NeedToken {
                    wrong: false,
                    write: true
                },
                "{method}"
            );
            assert_eq!(
                check(
                    &access,
                    &method,
                    "/api/notes",
                    &[host, ("cookie", "driftmark-4170=old")]
                ),
                Verdict::NeedToken {
                    wrong: true,
                    write: true
                },
                "{method}"
            );
            assert_eq!(
                check(&access, &method, "/api/notes", &[host, ("cookie", &good)]),
                Verdict::Pass,
                "{method}"
            );
        }
        assert_eq!(
            check(
                &access,
                &Method::POST,
                &format!("/api/seen?token={TOKEN}"),
                &[host]
            ),
            Verdict::NeedToken {
                wrong: true,
                write: true
            },
            "a write cannot bring the token in its address"
        );
        assert_eq!(
            check(&access, &Method::GET, &format!("/?token={TOKEN}"), &[host]),
            Verdict::Remember {
                location: "/".into(),
                cookie: format!(
                    "driftmark-4170={TOKEN}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"
                ),
            },
            "the printed link sets the cookie here too"
        );
        assert_eq!(access.cookie_name(), "driftmark-4170");
    }

    #[test]
    fn a_given_token_holds_for_reads_on_this_machine_too() {
        let access = Access::new("127.0.0.1:4170".parse().unwrap(), TOKEN.into(), true);
        assert_eq!(
            check(&access, &Method::GET, "/", &[("host", "evil.example")]),
            Verdict::NeedToken {
                wrong: false,
                write: false
            }
        );
    }

    #[test]
    fn the_token_in_the_address_becomes_a_cookie() {
        let access = Access::new("0.0.0.0:4170".parse().unwrap(), TOKEN.into(), false);
        let host = ("host", "100.64.0.1:4170");
        assert_eq!(
            check(
                &access,
                &Method::GET,
                &format!("/doc/main/-/a.md?x=1&token={TOKEN}&y"),
                &[host]
            ),
            Verdict::Remember {
                location: "/doc/main/-/a.md?x=1&y".into(),
                cookie: format!(
                    "driftmark-4170={TOKEN}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"
                ),
            }
        );
        assert_eq!(
            check(&access, &Method::HEAD, &format!("/?token={TOKEN}"), &[host]),
            Verdict::Remember {
                location: "/".into(),
                cookie: format!(
                    "driftmark-4170={TOKEN}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"
                ),
            }
        );
        assert_eq!(
            check(&access, &Method::GET, "/?token=nope", &[host]),
            Verdict::NeedToken {
                wrong: true,
                write: false
            }
        );
        assert_eq!(
            check(&access, &Method::POST, &format!("/?token={TOKEN}"), &[host]),
            Verdict::NeedToken {
                wrong: true,
                write: true
            }
        );
    }

    #[test]
    fn the_cookie_lets_requests_through() {
        let access = Access::new("[::]:9000".parse().unwrap(), TOKEN.into(), false);
        let good = format!("a=b; driftmark-9000={TOKEN}");
        assert_eq!(
            check(&access, &Method::GET, "/", &[("cookie", &good)]),
            Verdict::Pass
        );
        let split = [
            ("cookie", "a=b"),
            ("cookie", &format!("driftmark-9000={TOKEN}")),
        ];
        assert_eq!(check(&access, &Method::GET, "/", &split), Verdict::Pass);
        assert_eq!(
            check(
                &access,
                &Method::GET,
                "/",
                &[("cookie", "driftmark-9000=old")]
            ),
            Verdict::NeedToken {
                wrong: true,
                write: false
            }
        );
        assert_eq!(
            check(
                &access,
                &Method::GET,
                "/",
                &[("cookie", &format!("driftmark-4170={TOKEN}"))]
            ),
            Verdict::NeedToken {
                wrong: false,
                write: false
            },
            "another port's cookie"
        );
        assert_eq!(
            check(&access, &Method::GET, "/", &[]),
            Verdict::NeedToken {
                wrong: false,
                write: false
            }
        );
    }

    #[test]
    fn a_refused_write_says_how_to_get_the_token() {
        let missing = token_to_write(false, "driftmark-4170");
        assert!(missing.starts_with("Saving notes and seen marks needs the reader's token."));
        assert!(missing.contains("ends in ?token="), "{missing}");
        assert!(
            missing.ends_with("in the cookie driftmark-4170."),
            "{missing}"
        );
        let old = token_to_write(true, "driftmark-9");
        assert!(old.starts_with("The token this browser holds is not this server's token"));
    }

    #[test]
    fn secrets_compare_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"abcd"));
        assert!(!same(b"", b"a"));
    }

    #[test]
    fn bodies_must_be_json_from_this_server() {
        let ok = check_body(
            &Method::POST,
            &headers(&[
                ("host", "localhost:4170"),
                ("origin", "http://localhost:4170"),
                ("content-type", "application/json; charset=utf-8"),
                ("sec-fetch-site", "same-origin"),
            ]),
        );
        assert_eq!(ok, Ok(()));
        assert_eq!(
            check_body(
                &Method::PUT,
                &headers(&[("content-type", "application/json")])
            ),
            Ok(()),
            "a tool sends no origin"
        );
        assert_eq!(check_body(&Method::GET, &HeaderMap::new()), Ok(()));
        let refused = |pairs: &[(&'static str, &str)]| {
            check_body(&Method::POST, &headers(pairs)).unwrap_err().0
        };
        assert_eq!(
            refused(&[("content-type", "text/plain")]),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(refused(&[]), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let json = ("content-type", "application/json");
        let host = ("host", "localhost:4170");
        assert_eq!(
            refused(&[json, host, ("origin", "http://evil.example")]),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            refused(&[json, host, ("origin", "null")]),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            refused(&[json, ("origin", "http://localhost:4170")]),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            refused(&[json, host, ("sec-fetch-site", "cross-site")]),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn every_answer_gets_the_policy_and_no_cors() {
        let mut map = HeaderMap::new();
        add_security_headers(&mut map);
        assert_eq!(map[header::CONTENT_SECURITY_POLICY], CSP);
        assert_eq!(map[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(map.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }
}
