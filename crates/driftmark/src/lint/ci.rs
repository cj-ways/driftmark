//! The two forms of `driftmark lint` that a CI host shows next to the lines
//! of a merge request: GitHub Actions workflow commands (`--format github`)
//! and a GitLab Code Quality report (`--format gitlab`).
//!
//! `docs/features/ci.md` says how the GitHub Action and the GitLab template
//! use them.

use std::collections::HashMap;
use std::fmt::Write as _;

use driftmark_core::lint::{Finding, Severity};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::FileReport;

/// How many hexadecimal digits of SHA-256 a Code Quality fingerprint keeps:
/// 128 bits, the length of the MD5 fingerprints GitLab's own examples use.
const FINGERPRINT_DIGITS: usize = 32;

/// The findings as GitHub Actions workflow commands, one per line, then the
/// summary line of the text output.
///
/// GitHub turns each `::error` or `::warning` line into an annotation on its
/// file and line. Every value is escaped as GitHub's own toolkit escapes it,
/// so a message or a path cannot end the command early or start another one:
/// a line break in a message would otherwise let a document's text run any
/// workflow command.
#[must_use]
pub fn to_github(reports: &[FileReport]) -> String {
    let mut out = String::new();
    for report in reports {
        for finding in &report.findings {
            let _ = writeln!(
                out,
                "::{} file={},line={},col={},title={}::{}",
                finding.severity.name(),
                property(&report.path),
                finding.line,
                finding.column,
                property(&format!("driftmark lint: {}", finding.rule)),
                data(&finding.message),
            );
        }
    }
    out.push_str(&super::summary(reports));
    out.push('\n');
    out
}

/// A command's message, escaped as `escapeData` in `@actions/core` does:
/// `%`, carriage returns and line feeds.
fn data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// A command's property value, escaped as `escapeProperty` in
/// `@actions/core` does: as a message, plus `:` and `,`, which separate
/// properties.
fn property(text: &str) -> String {
    data(text).replace(':', "%3A").replace(',', "%2C")
}

/// The findings as a GitLab Code Quality report: a JSON array with one issue
/// per finding.
///
/// Each issue has the fields GitLab requires: `description`, `check_name`,
/// `fingerprint`, `severity` and `location` with `path` and `lines.begin`.
/// An error is `major` and a warning `minor`.
///
/// GitLab marks a finding as new or fixed by comparing fingerprints with the
/// target branch's report. So the fingerprint names the file, the rule, the
/// message and how many equal findings came before it in the file, but not
/// the line: a finding that only moved down when lines were added above it
/// stays the same finding.
#[must_use]
pub fn to_gitlab(reports: &[FileReport]) -> serde_json::Value {
    let mut issues = Vec::new();
    for report in reports {
        let mut seen: HashMap<(&str, &str), usize> = HashMap::new();
        for finding in &report.findings {
            let rule = finding.rule.id();
            let occurrence = seen.entry((rule, finding.message.as_str())).or_default();
            issues.push(json!({
                "description": format!("{rule}: {}", finding.message),
                "check_name": rule,
                "fingerprint": fingerprint(&report.path, finding, *occurrence),
                "severity": gitlab_severity(finding.severity),
                "location": {
                    "path": report.path,
                    "lines": { "begin": finding.line },
                },
            }));
            *occurrence += 1;
        }
    }
    serde_json::Value::Array(issues)
}

/// GitLab's severity for a finding's severity: `info`, `minor`, `major`,
/// `critical` and `blocker` are GitLab's names, lowest first.
fn gitlab_severity(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "major",
        Severity::Warning => "minor",
    }
}

/// The first [`FINGERPRINT_DIGITS`] hexadecimal digits of the SHA-256 of
/// the path, the rule, the message and the occurrence, each ended by a zero
/// byte so that no two different tuples run together into the same bytes.
fn fingerprint(path: &str, finding: &Finding, occurrence: usize) -> String {
    let mut hash = Sha256::new();
    for part in [
        path,
        finding.rule.id(),
        finding.message.as_str(),
        &occurrence.to_string(),
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    let mut hex = String::with_capacity(FINGERPRINT_DIGITS);
    for byte in hash.finalize().iter().take(FINGERPRINT_DIGITS / 2) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use driftmark_core::lint::Rule;

    use super::*;

    fn finding(rule: Rule, severity: Severity, line: usize, message: &str) -> Finding {
        Finding {
            rule,
            severity,
            line,
            column: 3,
            message: message.to_owned(),
        }
    }

    fn reports() -> Vec<FileReport> {
        vec![
            FileReport {
                path: "docs/a, b: c.md".to_owned(),
                findings: vec![
                    finding(Rule::Title, Severity::Error, 1, "first"),
                    finding(Rule::Link, Severity::Error, 4, "100% broken\r\nnext"),
                ],
            },
            FileReport {
                path: "docs/clean.md".to_owned(),
                findings: Vec::new(),
            },
            FileReport {
                path: "notes/x.md".to_owned(),
                findings: vec![finding(Rule::CodeLanguage, Severity::Warning, 9, "warn")],
            },
        ]
    }

    #[test]
    fn github_commands_escape_every_value() {
        assert_eq!(
            to_github(&reports()),
            "\
::error file=docs/a%2C b%3A c.md,line=1,col=3,title=driftmark lint%3A title::first
::error file=docs/a%2C b%3A c.md,line=4,col=3,title=driftmark lint%3A link::100%25 broken%0D%0Anext
::warning file=notes/x.md,line=9,col=3,title=driftmark lint%3A code-language::warn
driftmark lint: 3 files checked, 2 errors and 1 warning in 2 files
"
        );
    }

    #[test]
    fn a_message_cannot_start_a_second_command() {
        let reports = vec![FileReport {
            path: "a.md".to_owned(),
            findings: vec![finding(
                Rule::Link,
                Severity::Error,
                1,
                "the link `x\n::add-mask::secret` is broken",
            )],
        }];
        let out = to_github(&reports);
        assert_eq!(out.lines().filter(|line| line.starts_with("::")).count(), 1);
        assert!(out.contains("x%0A::add-mask::secret"), "{out}");
    }

    #[test]
    fn no_findings_print_only_the_summary() {
        let clean = vec![FileReport {
            path: "a.md".to_owned(),
            findings: Vec::new(),
        }];
        assert_eq!(
            to_github(&clean),
            "driftmark lint: 1 file checked, no problems found\n"
        );
        assert_eq!(to_gitlab(&clean), json!([]));
    }

    #[test]
    fn gitlab_issues_carry_every_required_field() {
        let report = to_gitlab(&reports());
        let issues = report.as_array().unwrap();
        assert_eq!(issues.len(), 3);
        assert_eq!(
            issues[0],
            json!({
                "description": "title: first",
                "check_name": "title",
                "fingerprint": issues[0]["fingerprint"],
                "severity": "major",
                "location": {"path": "docs/a, b: c.md", "lines": {"begin": 1}},
            })
        );
        assert_eq!(issues[2]["severity"], "minor");
        assert_eq!(issues[2]["check_name"], "code-language");
        assert_eq!(issues[2]["location"]["lines"]["begin"], 9);
        for issue in issues {
            let print = issue["fingerprint"].as_str().unwrap();
            assert_eq!(print.len(), FINGERPRINT_DIGITS);
            assert!(print.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn fingerprints_ignore_the_line_but_keep_equal_findings_apart() {
        let at = |line| finding(Rule::Link, Severity::Error, line, "same message");
        let one = |findings| {
            to_gitlab(&[FileReport {
                path: "a.md".to_owned(),
                findings,
            }])
        };
        let moved = one(vec![at(40)]);
        let first = one(vec![at(4)]);
        assert_eq!(moved[0]["fingerprint"], first[0]["fingerprint"]);
        let twice = one(vec![at(4), at(5)]);
        assert_eq!(twice[0]["fingerprint"], first[0]["fingerprint"]);
        assert_ne!(twice[0]["fingerprint"], twice[1]["fingerprint"]);
        let other_file = to_gitlab(&[FileReport {
            path: "b.md".to_owned(),
            findings: vec![at(4)],
        }]);
        assert_ne!(other_file[0]["fingerprint"], first[0]["fingerprint"]);
        let other_rule = one(vec![finding(
            Rule::Anchor,
            Severity::Error,
            4,
            "same message",
        )]);
        assert_ne!(other_rule[0]["fingerprint"], first[0]["fingerprint"]);
    }

    #[test]
    fn fields_cannot_run_together_into_one_fingerprint() {
        let a = fingerprint("ab", &finding(Rule::Link, Severity::Error, 1, "c"), 0);
        let b = fingerprint("a", &finding(Rule::Link, Severity::Error, 1, "bc"), 0);
        assert_ne!(a, b);
        assert_eq!(
            fingerprint("a.md", &finding(Rule::Title, Severity::Error, 1, "m"), 0),
            // printf 'a.md\0title\0m\0000\0' | sha256sum | cut -c1-32
            "361028ab6627a365d7aec84f5b019194",
            "the fingerprint of a finding never changes between versions"
        );
    }
}
