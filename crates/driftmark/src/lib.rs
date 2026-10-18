//! The parts of the `driftmark` program that touch the outside world.
//!
//! The rules live in `driftmark-core`, which reads no files and runs no git.
//! This library supplies what the core asks for. It is a library beside the
//! program so its tests can build real repos and check the readers from
//! outside, the way the commands will use them.

pub mod git;
