//! The GPUI-free terminal state that Brindle and its server share: the
//! shell-integration parser with its OSC 133 marks, the OSC scanner and the
//! OSC 7501 program status records, the replay renderer, and the server's
//! per-session emulator.

pub mod emulator;
pub mod notice;
pub mod osc;
pub mod replay;
pub mod shell_integration;
pub mod status;

/// Re-exported so the server and its tests name the same `Term` types
/// without a dependency of their own.
pub use alacritty_terminal;
pub use notice::{Notice, Source};
