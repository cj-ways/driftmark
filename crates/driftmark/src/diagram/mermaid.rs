//! Mermaid drawn by merman, measuring text with the Liberation Sans files
//! the program ships (decision 0008).
//!
//! The page draws diagram labels in the same font, so a label's box fits its
//! text. merman reads fonts only from files, so the four faces are written
//! once to a folder of the user's cache, named by a hash of their bytes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use merman::svg::RenderResourcePolicy;
use merman::svg::export::{NativeFontContext, NativeFontLimits, NativeFontOptions};
use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest};

/// The four Liberation Sans faces: regular, bold, italic and bold italic.
const FONTS: [(&str, &[u8]); 4] = [
    (
        "LiberationSans-Regular.ttf",
        include_bytes!("../../assets/fonts/LiberationSans-Regular.ttf"),
    ),
    (
        "LiberationSans-Bold.ttf",
        include_bytes!("../../assets/fonts/LiberationSans-Bold.ttf"),
    ),
    (
        "LiberationSans-Italic.ttf",
        include_bytes!("../../assets/fonts/LiberationSans-Italic.ttf"),
    ),
    (
        "LiberationSans-BoldItalic.ttf",
        include_bytes!("../../assets/fonts/LiberationSans-BoldItalic.ttf"),
    ),
];

/// How many text measurements one drawing may make. merman's default is
/// 4,096 for a font context's whole life, and a large ER diagram needs more
/// (merman issue 207): a made-up one of 340 tables with 2 columns failed at
/// the default. Each drawing gets a fresh budget through `for_operation`.
pub(super) const MAX_MEASUREMENTS: usize = 1 << 20;

/// How long one drawing may take. merman draws the median corpus diagram in
/// about 10 ms and the largest in under a second; a diagram that takes
/// longer is left to the browser.
const DEADLINE: Duration = Duration::from_secs(10);

/// Numbers each temporary font file, so writers on different threads of one
/// program never write the same one.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// merman with its fonts loaded.
pub(super) struct Mermaid {
    renderer: Renderer,
    fonts: NativeFontContext,
}

impl Mermaid {
    /// Writes the fonts to the cache folder when they are not there yet, and
    /// loads them.
    ///
    /// # Errors
    ///
    /// When the fonts cannot be written or loaded.
    pub(super) fn new() -> Result<Mermaid, String> {
        let folder = write_fonts(&font_folder())?;
        let mut options = NativeFontOptions::default().with_limits(NativeFontLimits {
            max_measurements: MAX_MEASUREMENTS,
            ..NativeFontLimits::default()
        });
        for (name, _) in FONTS {
            options = options.with_font_file(folder.join(name));
        }
        let fonts = options
            .load(&OperationControl::new())
            .map_err(|error| format!("the diagram fonts could not be loaded: {error}"))?;
        Ok(Mermaid {
            renderer: Renderer::new(),
            fonts,
        })
    }

    /// The diagram as merman's SVG, not yet cleaned. `id` starts every id
    /// in the drawing.
    ///
    /// # Errors
    ///
    /// merman's message when it cannot draw the diagram.
    pub(super) fn draw(&self, code: &str, id: &str) -> Result<String, String> {
        let control = OperationControl::new().with_deadline(DEADLINE);
        let mut request = SvgRequest::default();
        request.options.diagram_id = Some(id.to_owned());
        // Local documents are trusted input: the layout budget of the
        // default profile is too small for the largest ER diagrams, and the
        // deadline above bounds the time instead.
        request.environment = request
            .environment
            .with_resource_policy(RenderResourcePolicy::unbounded_for_trusted_input())
            .with_native_font_context(self.fonts.for_operation(&control))
            .map_err(|error| format!("the diagram fonts could not be used: {error}"))?;
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.renderer
                .render(RenderRequest::svg(code, control, request))
        }));
        match drawn {
            Ok(Ok(RenderOutput::Svg(Some(svg)))) => Ok(svg.svg().to_owned()),
            Ok(Ok(_)) => Err("merman found no diagram in this block".to_owned()),
            Ok(Err(error)) => Err(first_line(&error.to_string())),
            Err(_) => Err("merman stopped while drawing this diagram".to_owned()),
        }
    }
}

/// The first line of a message, at most 300 characters, for `data-error`.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    line.chars().take(300).collect()
}

/// Where the font files go: the user's cache folder, or the system's
/// temporary folder when there is none.
fn font_folder() -> PathBuf {
    let base = user_cache().unwrap_or_else(std::env::temp_dir);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (_, bytes) in FONTS {
        for &byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    base.join("driftmark").join(format!("fonts-{hash:016x}"))
}

/// The user's cache folder by the system's convention.
fn user_cache() -> Option<PathBuf> {
    let from = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if cfg!(windows) {
        return from("LOCALAPPDATA").map(PathBuf::from);
    }
    if cfg!(target_os = "macos") {
        return from("HOME").map(|home| PathBuf::from(home).join("Library/Caches"));
    }
    from("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| from("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

/// Writes each font file whose bytes are not in `folder` yet, through a
/// temporary name, so two servers starting at once never read half a file.
fn write_fonts(folder: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|error| {
        format!(
            "the diagram fonts could not be written to {}: {error}",
            folder.display()
        )
    })?;
    for (name, bytes) in FONTS {
        let path = folder.join(name);
        let written = || std::fs::read(&path).is_ok_and(|found| found == bytes);
        if written() {
            continue;
        }
        let temporary = folder.join(format!(
            "{name}.{}.{}.part",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        let moved =
            std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, &path));
        if let Err(error) = moved {
            let _ = std::fs::remove_file(&temporary);
            // Another writer may have put the font there first; on Windows,
            // a font file another program has open cannot be replaced.
            if !written() {
                return Err(format!(
                    "the diagram font {} could not be written: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(folder.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_are_written_once_and_kept() {
        let folder =
            std::env::temp_dir().join(format!("driftmark-fonts-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        write_fonts(&folder).unwrap();
        for (name, bytes) in FONTS {
            assert_eq!(std::fs::read(folder.join(name)).unwrap(), bytes);
        }
        std::fs::write(folder.join(FONTS[0].0), b"broken").unwrap();
        write_fonts(&folder).unwrap();
        assert_eq!(std::fs::read(folder.join(FONTS[0].0)).unwrap(), FONTS[0].1);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn writers_that_start_together_all_find_the_fonts() {
        let folder =
            std::env::temp_dir().join(format!("driftmark-fonts-together-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let start = std::sync::Barrier::new(8);
        let written: Vec<Result<PathBuf, String>> = std::thread::scope(|scope| {
            let writers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        write_fonts(&folder)
                    })
                })
                .collect();
            writers
                .into_iter()
                .map(|writer| writer.join().unwrap())
                .collect()
        });
        for result in written {
            assert_eq!(result, Ok(folder.clone()));
        }
        for (name, bytes) in FONTS {
            assert_eq!(std::fs::read(folder.join(name)).unwrap(), bytes);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn messages_keep_their_first_line() {
        assert_eq!(first_line("one\ntwo"), "one");
        assert_eq!(first_line(&"x".repeat(400)).len(), 300);
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn the_font_folder_is_named_by_the_fonts() {
        let folder = font_folder();
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("fonts-"), "{name}");
        assert_eq!(name.len(), "fonts-".len() + 16);
    }
}
