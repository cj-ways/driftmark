//! The starter layout that `driftmark init` sets up for a team with no docs
//! system (decision 0006): five folders, each with a README that says what
//! belongs in it, a writing standard, and the `starter` rules file. This
//! module also holds the templates that `driftmark new` fills in, and the
//! block of agent instructions that `init` prints.
//!
//! `docs/features/starter-layout.md` states the contract. This module only
//! holds the texts and fills them in. The program checks which files exist,
//! reads today's date and writes, so the core touches no file and no clock.
//!
//! The texts are files in the crate's `starter/` folder, laid out the way
//! `init` writes them. There they can be read, rendered and checked as they
//! are, and `include_str!` builds them into the program.

mod agents;
mod new;
pub mod preset;

#[cfg(test)]
mod tests;

pub use agents::{AGENT_BLOCK, AGENT_BLOCK_START, agent_addition};
pub use new::{
    DECISIONS_FOLDER, Kind, NewDocument, TitleError, new_document, next_decision_number, slug,
};

/// One part of the starter layout: a folder, or a file with the text `init`
/// writes into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A folder, relative to the repository root, without a final `/`.
    Folder(&'static str),
    /// A file, relative to the repository root, and its whole text.
    File {
        /// Where the file goes.
        path: &'static str,
        /// What it holds.
        text: &'static str,
    },
}

impl Part {
    /// The part's path from the repository root, with `/` between folders.
    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Part::Folder(path) | Part::File { path, .. } => path,
        }
    }
}

/// Every part of the starter layout, in the order `init` reports them: each
/// folder, then the files inside it, and the rules file last.
pub const LAYOUT: [Part; 12] = [
    Part::Folder("docs/decisions"),
    Part::File {
        path: "docs/decisions/README.md",
        text: include_str!("../../starter/docs/decisions/README.md"),
    },
    Part::Folder("docs/standards"),
    Part::File {
        path: "docs/standards/README.md",
        text: include_str!("../../starter/docs/standards/README.md"),
    },
    Part::File {
        path: "docs/standards/writing.md",
        text: include_str!("../../starter/docs/standards/writing.md"),
    },
    Part::Folder("docs/features"),
    Part::File {
        path: "docs/features/README.md",
        text: include_str!("../../starter/docs/features/README.md"),
    },
    Part::Folder("docs/working"),
    Part::File {
        path: "docs/working/README.md",
        text: include_str!("../../starter/docs/working/README.md"),
    },
    Part::Folder("docs/archive"),
    Part::File {
        path: "docs/archive/README.md",
        text: include_str!("../../starter/docs/archive/README.md"),
    },
    Part::File {
        path: preset::FILE,
        text: preset::RULES,
    },
];
