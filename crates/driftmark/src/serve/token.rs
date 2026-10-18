//! Where the reader's token comes from (decision 0012).
//!
//! Every request that writes notes or seen marks needs the token, whatever
//! address the server listens on, so a server always has one. It comes from
//! exactly one source, the first of these that is given:
//!
//! 1. `--token <value>`;
//! 2. `--token-file <path>`, read once at start;
//! 3. the `DRIFTMARK_TOKEN` environment variable;
//! 4. otherwise a random one, made at start.
//!
//! The file and the variable keep the token out of the process list, where
//! any user of the machine can read a command line. On Unix a token file
//! that other users can read or change is refused.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;

/// The environment variable that can hold the token.
pub(crate) const VARIABLE: &str = "DRIFTMARK_TOKEN";

/// The longest token file read, in bytes. A token is a short line.
const MOST_BYTES: u64 = 4096;

/// The shortest token accepted. A token is the only thing between the
/// network and the documents, so a short one that can be guessed is
/// refused.
pub(crate) const MIN_LEN: usize = 16;

/// Where the token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenSource {
    /// `--token`.
    Option,
    /// The file `--token-file` names.
    File,
    /// The `DRIFTMARK_TOKEN` environment variable.
    Environment,
    /// Made at start, because none was given.
    Made,
}

impl TokenSource {
    /// The source, as the start message names it.
    pub(crate) fn words(self) -> &'static str {
        match self {
            TokenSource::Option => "--token",
            TokenSource::File => "--token-file",
            TokenSource::Environment => VARIABLE,
            TokenSource::Made => "a new random token",
        }
    }
}

/// The server's token and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    /// The token.
    pub value: String,
    /// Where it came from.
    pub from: TokenSource,
}

/// The token from the first source given: `option` (`--token`, already
/// checked), the file at `file`, the environment's `variable`, or a new
/// random one.
///
/// # Errors
///
/// A sentence for the user when the file cannot be read, other users can
/// read or change it, or the file's or the variable's token is not a valid
/// token.
pub(crate) fn choose(
    option: Option<&str>,
    file: Option<&Path>,
    variable: Option<OsString>,
    make: impl FnOnce() -> Result<String, String>,
) -> Result<Token, String> {
    if let Some(value) = option {
        return Ok(Token {
            value: value.to_owned(),
            from: TokenSource::Option,
        });
    }
    if let Some(path) = file {
        let text = read_private(path)?;
        let value = checked(text.trim(), &format!("the token in `{}`", path.display()))?;
        return Ok(Token {
            value,
            from: TokenSource::File,
        });
    }
    if let Some(text) = variable.filter(|text| !text.is_empty()) {
        let text = text
            .into_string()
            .map_err(|_| format!("the token in {VARIABLE} is not valid text"))?;
        let value = checked(text.trim(), &format!("the token in {VARIABLE}"))?;
        return Ok(Token {
            value,
            from: TokenSource::Environment,
        });
    }
    Ok(Token {
        value: make()?,
        from: TokenSource::Made,
    })
}

/// Checks a token: it goes into a web address and a cookie as it is, so it
/// may hold only characters that need no escaping there, and it must be
/// long enough not to be guessed. `what` names it in the sentence.
///
/// # Errors
///
/// A sentence that says what is wrong.
pub(crate) fn checked(token: &str, what: &str) -> Result<String, String> {
    let plain = token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'));
    if !plain {
        return Err(format!(
            "{what} may hold only letters, digits, `-`, `_`, `.` and `~`"
        ));
    }
    if token.len() < MIN_LEN {
        return Err(format!(
            "{what} needs at least {MIN_LEN} characters; leave it out to get a random one"
        ));
    }
    Ok(token.to_owned())
}

/// The text of a token file that only its owner can read and change.
fn read_private(path: &Path) -> Result<String, String> {
    let shown = path.display();
    // Windows refuses to open a folder ("Access is denied"), so a folder is
    // named as one before opening. The check on the open handle below still
    // guards against the path changing in between.
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(format!(
            "`--token-file` names `{shown}`, which is not a file"
        ));
    }
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("`--token-file` cannot read `{shown}`: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("`--token-file` cannot read `{shown}`: {error}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "`--token-file` names `{shown}`, which is not a file"
        ));
    }
    open_to_others(&metadata, path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MOST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("`--token-file` cannot read `{shown}`: {error}"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MOST_BYTES {
        return Err(format!(
            "`{shown}` is longer than {MOST_BYTES} bytes; a token file holds the token alone"
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("the token in `{shown}` is not valid text"))
}

/// On Unix: refuses a file that its group or other users can read or
/// change, as `ssh` refuses such a key. Anyone who can read the token can
/// act as the reader.
#[cfg(unix)]
fn open_to_others(metadata: &std::fs::Metadata, path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o066 != 0 {
        return Err(format!(
            "other users can read or change `{}` (its mode is {mode:03o}); make it private with `chmod 600 {}`",
            path.display(),
            path.display()
        ));
    }
    Ok(())
}

/// Elsewhere, files carry no such mode; the file is read as it is.
#[cfg(not(unix))]
fn open_to_others(_metadata: &std::fs::Metadata, _path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const GOOD: &str = "0123456789abcdef-token";

    fn never() -> Result<String, String> {
        Err("no random token was asked for".to_owned())
    }

    struct Dir(PathBuf);

    impl Dir {
        fn new(label: &str) -> Dir {
            let dir = std::env::temp_dir()
                .join(format!("driftmark-token-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }

        fn file(&self, name: &str, text: &[u8], mode: u32) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, text).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            let _ = mode;
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_first_source_given_wins() {
        let dir = Dir::new("order");
        let file = dir.file("token", format!("{GOOD}-file\n").as_bytes(), 0o600);
        let from = |option, file: Option<&Path>, variable: Option<&str>| {
            choose(option, file, variable.map(OsString::from), || {
                Ok("made-at-start-0123456789".to_owned())
            })
            .unwrap()
        };
        let token = from(Some(GOOD), Some(&file), Some("ignored-ignored-ignored"));
        assert_eq!(
            (token.value.as_str(), token.from),
            (GOOD, TokenSource::Option)
        );
        let token = from(None, Some(&file), Some("ignored-ignored-ignored"));
        assert_eq!(token.value, format!("{GOOD}-file"));
        assert_eq!(token.from, TokenSource::File);
        let token = from(None, None, Some(" from-the-environment-1 "));
        assert_eq!(
            (token.value.as_str(), token.from),
            ("from-the-environment-1", TokenSource::Environment)
        );
        let token = from(None, None, Some(""));
        assert_eq!(
            token.from,
            TokenSource::Made,
            "an empty variable counts as unset"
        );
        let token = from(None, None, None);
        assert_eq!(
            (token.value.as_str(), token.from),
            ("made-at-start-0123456789", TokenSource::Made)
        );
        assert_eq!(
            choose(None, None, None, || Err("no randomness".into())),
            Err("no randomness".to_owned())
        );
        for (from, words) in [
            (TokenSource::Option, "--token"),
            (TokenSource::File, "--token-file"),
            (TokenSource::Environment, "DRIFTMARK_TOKEN"),
            (TokenSource::Made, "a new random token"),
        ] {
            assert_eq!(from.words(), words);
        }
    }

    #[test]
    fn bad_tokens_from_a_file_or_the_environment_are_refused() {
        let dir = Dir::new("bad");
        let short = dir.file("short", b"tiny\n", 0o600);
        let error = choose(None, Some(&short), None, never).unwrap_err();
        assert!(error.contains("needs at least 16 characters"), "{error}");
        assert!(error.starts_with("the token in `"), "{error}");
        let error = choose(None, None, Some("has spaces in the middle".into()), never).unwrap_err();
        assert_eq!(
            error,
            "the token in DRIFTMARK_TOKEN may hold only letters, digits, `-`, `_`, `.` and `~`"
        );
        let missing = dir.0.join("missing");
        let error = choose(None, Some(&missing), None, never).unwrap_err();
        assert!(error.starts_with("`--token-file` cannot read `"), "{error}");
        let error = choose(None, Some(&dir.0), None, never).unwrap_err();
        assert!(error.ends_with("which is not a file"), "{error}");
        let long = dir.file("long", &[b'a'; 5000], 0o600);
        let error = choose(None, Some(&long), None, never).unwrap_err();
        assert!(error.contains("longer than 4096 bytes"), "{error}");
        let odd = dir.file("odd", &[0xff; 20], 0o600);
        let error = choose(None, Some(&odd), None, never).unwrap_err();
        assert!(error.ends_with("is not valid text"), "{error}");
        assert_eq!(checked(GOOD, "x"), Ok(GOOD.to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn a_token_file_others_can_read_is_refused() {
        let dir = Dir::new("mode");
        for mode in [0o644, 0o640, 0o604, 0o620, 0o602] {
            let path = dir.file(&format!("t{mode:o}"), GOOD.as_bytes(), mode);
            let error = choose(None, Some(&path), None, never).unwrap_err();
            assert!(
                error.starts_with("other users can read or change `")
                    && error.contains(&format!("its mode is {mode:03o}"))
                    && error.ends_with(&format!("`chmod 600 {}`", path.display())),
                "{mode:o}: {error}"
            );
        }
        for mode in [0o600, 0o400, 0o700] {
            let path = dir.file(&format!("ok{mode:o}"), GOOD.as_bytes(), mode);
            assert_eq!(
                choose(None, Some(&path), None, never).map(|token| token.value),
                Ok(GOOD.to_owned()),
                "{mode:o}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_variable_that_is_not_text_is_refused() {
        use std::os::unix::ffi::OsStringExt;
        let odd = OsString::from_vec(vec![0xff, 0xfe]);
        assert_eq!(
            choose(None, None, Some(odd), never),
            Err("the token in DRIFTMARK_TOKEN is not valid text".to_owned())
        );
    }
}
