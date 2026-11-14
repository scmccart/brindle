//! The pseudoconsoles the terminal server's holder runs programs on
//! (Windows), and the command lines it starts them with.
//!
//! - [`cmdline`] quotes command lines and builds environment blocks. It is
//!   pure, so it builds and is tested on every platform.
//! - `api` resolves the ConPTY to use: the packaged one embedded in the
//!   binary (`build.rs`), loaded by absolute path from beside the exe or
//!   from an extracted copy, and only when its bytes match the embedded
//!   ones; otherwise the ConPTY built into Windows, with one warning. The
//!   caller chooses per pseudoconsole (a session whose environment has
//!   `BRINDLE_CONPTY=inbox` gets the built-in one), and each is resolved
//!   once, when first asked for.
//! - [`Pseudoconsole`] is created over pipe ends the caller made, with
//!   wcwidth glyph widths (the way the grid counts cells). It starts the
//!   program ([`Pseudoconsole::start`]), resizes and reparents, and
//!   dropping it releases and closes it on a short-lived thread, so nothing
//!   waits on a host that blocks in `ClosePseudoConsole`. That thread
//!   first waits for the program to be on the console, which a close that
//!   comes sooner never reaches, and keeps the input open until then.
//!   [`console_host`] says which console host a process is on, for the
//!   caller to end the programs that reached it after the close.
//!
//! Reading, writing and waiting for the program are the caller's: the
//! holder does them on its event loop.

#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows holder's pty starts programs with it")
)]
pub mod cmdline;

#[cfg(windows)]
mod api;
#[cfg(windows)]
mod console;

#[cfg(windows)]
pub use console::{Process, Pseudoconsole, console_host};

/// Whether this build embeds the packaged ConPTY.
#[cfg(all(windows, test))]
pub fn has_embedded() -> bool {
    api::bundle::FILES.is_some()
}
