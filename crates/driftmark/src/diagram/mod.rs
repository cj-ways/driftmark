//! Draws the diagrams of a reading page on the server: Mermaid with merman,
//! D2 with the `d2` program when it is installed.
//!
//! [`Drawer`] is the program's [`DiagramRender`]. Every drawing is cleaned
//! before it reaches a page ([`clean`]), because the SVG goes into the page as
//! it is and a diagram's text comes from the repository. Drawings are kept by
//! a hash of the diagram's language and text, so a page opened again, or the
//! same diagram in another document, costs nothing.
//!
//! A diagram the server cannot draw stays code marked as undrawn. The page
//! script then draws Mermaid in the browser with mermaid.js. D2 has no
//! browser fallback and stays code.

mod cache;
mod clean;
mod d2;
mod mermaid;

use std::sync::Arc;

use driftmark_core::document::DiagramLanguage;
use driftmark_core::render::{Diagram, DiagramRender};

pub use clean::{CleanError, clean};

/// How many bytes of finished drawings the drawer keeps. The private
/// corpus's 195 Mermaid blocks make about 14 MB of SVG at the median size,
/// so this keeps every diagram of a large repository that a day's reading
/// opens.
pub const CACHE_BYTES: usize = 32 * 1024 * 1024;

/// What one drawing gave: the clean SVG, or why there is none. `Ok(None)`
/// means the language is not drawn here.
pub type Drawn = Result<Option<Arc<str>>, String>;

/// Draws diagrams with merman and `d2`, and keeps what it drew.
pub struct Drawer {
    mermaid: Option<mermaid::Mermaid>,
    d2: Option<d2::D2>,
    cache: cache::Drawings,
}

impl Drawer {
    /// A drawer with merman, measuring text with the shipped Liberation Sans,
    /// and with `d2` when it is on the `PATH`.
    ///
    /// When the font files cannot be written to the cache folder, merman is
    /// left out: its built-in width guess cuts off text in most diagrams
    /// (`docs/working/version-1/spikes/mermaid.md`), so the browser draws
    /// them instead. `problem` then says why.
    #[must_use]
    pub fn new() -> Drawer {
        Drawer {
            mermaid: mermaid::Mermaid::new().ok(),
            d2: d2::D2::find(),
            cache: cache::Drawings::new(CACHE_BYTES),
        }
    }

    /// A drawer that draws nothing, so every diagram is left to the browser.
    #[must_use]
    pub fn none() -> Drawer {
        Drawer {
            mermaid: None,
            d2: None,
            cache: cache::Drawings::new(0),
        }
    }

    /// Whether Mermaid is drawn on the server.
    #[must_use]
    pub fn draws_mermaid(&self) -> bool {
        self.mermaid.is_some()
    }

    /// Whether D2 is drawn on the server.
    #[must_use]
    pub fn draws_d2(&self) -> bool {
        self.d2.is_some()
    }

    /// Draws one diagram, from the cache when it was drawn before.
    ///
    /// # Errors
    ///
    /// Why the diagram could not be drawn, or could not be made safe; the
    /// page shows the message beside the code.
    pub fn drawing(&self, language: DiagramLanguage, code: &str) -> Drawn {
        let prefix = format!("dm{:012x}", cache::hash(language, code) >> 16);
        let draw = || -> Drawn {
            let svg = match language {
                DiagramLanguage::Mermaid => match &self.mermaid {
                    Some(mermaid) => mermaid.draw(code, &prefix)?,
                    None => return Ok(None),
                },
                DiagramLanguage::D2 => match &self.d2 {
                    Some(d2) => d2.draw(code)?,
                    None => return Ok(None),
                },
            };
            match clean(&svg, &prefix) {
                Ok(clean) => Ok(Some(Arc::from(clean))),
                Err(error) => Err(format!("the drawing could not be made safe: {error}")),
            }
        };
        self.cache.get_or_draw(language, code, draw)
    }
}

impl Default for Drawer {
    fn default() -> Drawer {
        Drawer::new()
    }
}

impl DiagramRender for Drawer {
    fn draw(&self, diagram: &Diagram<'_>) -> Result<Option<String>, String> {
        self.drawing(diagram.language, diagram.code)
            .map(|svg| svg.map(|svg| svg.to_string()))
    }
}

#[cfg(test)]
mod tests;
