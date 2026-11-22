//! The GPUI-free terminal state that Brindle and its server share: the
//! shell-integration parser with its OSC 133 marks, the OSC scanner, kitty's
//! OSC 99 notification chunks and the OSC 7501 program status records, the OSC 9;4 progress, the OSC 3008
//! context stack, the OSC 22 pointer shape stacks, the replay renderer, and
//! the server's per-session emulator.

// railway R3, types R2: no `_ =>` on one of Brindle's own enums. railway R5,
// R9: a decoder never panics on bytes from outside, and non-test code has no
// `unwrap` or `expect`. Non-test code only.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod context;
pub mod emulator;
#[cfg(test)]
mod fuzz;
pub mod kitty;
pub mod notice;
pub mod osc;
pub mod pointer;
pub mod progress;
pub mod replay;
pub mod shell_integration;
pub mod status;

/// Re-exported so the server and its tests name the same `Term` types
/// without a dependency of their own.
pub use alacritty_terminal;
pub use notice::{Notice, Source};
