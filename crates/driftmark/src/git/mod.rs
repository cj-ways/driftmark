//! Reading git, two ways.
//!
//! [`GixReader`] reads the repo with the gix library, in this process.
//! [`CommandReader`] runs the `git` command. Both answer the questions of
//! [`driftmark_core::git::GitRead`], and tests check that they answer alike.
//! [`Git`] is what the commands use: gix first, and the `git` command when gix
//! cannot open the repo or read an object (decision 0008, item 1).

mod choose;
mod command;
mod gitoxide;
pub(crate) mod worktree;

pub use choose::Git;
pub use command::CommandReader;
pub use gitoxide::GixReader;
