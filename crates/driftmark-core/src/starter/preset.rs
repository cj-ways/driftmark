//! The `starter` preset: the rules file of the starter layout.
//!
//! It is an ordinary rules file (`docs/features/engine/rules-file.md`), so
//! `driftmark init` writes it as the repository's `driftmark.toml`, and it can
//! join the other built-in presets as it is. It types every file of the
//! layout, says what each status means, marks `## Decision` sections, and
//! writes out the writing check's settings.

/// The preset's name.
pub const NAME: &str = "starter";

/// Where `init` writes the preset: the rules file at the repository root.
pub const FILE: &str = "driftmark.toml";

/// The preset's text, as `init` writes it.
pub const RULES: &str = include_str!("../../starter/driftmark.toml");
