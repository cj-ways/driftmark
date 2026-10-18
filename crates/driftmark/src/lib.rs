//! The parts of the `driftmark` program that touch the outside world, and its
//! commands.
//!
//! The rules live in `driftmark-core`, which reads no files and runs no git.
//! This library supplies what the core asks for. It is a library beside the
//! program so its tests can build real repos and check the readers from
//! outside, the way the commands will use them.

pub mod diff;
pub mod git;
pub mod hook;
pub mod lint;
pub mod serve;
