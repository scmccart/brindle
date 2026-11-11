//! A session's pseudoconsole and program for the holder: to be ConPTY over
//! a pair of overlapped pipes.

use std::ffi::OsString;
use std::io;

use super::event_loop::Source;
use super::unsupported;
use crate::pty::PtyCommand;

/// A session's pseudoconsole: its input and output.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Pty(());

impl Pty {
    /// What an event loop waits on: readable while the program's output is
    /// waiting, writable while the pseudoconsole takes input.
    pub fn source(&self) -> Source {
        unreachable!("a stub, never constructed")
    }

    /// Reads the program's output. `Ok(0)` means there will be no more;
    /// `WouldBlock` that none is waiting.
    pub fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }

    /// Writes input for the program, as much as is taken; `WouldBlock` when
    /// none is.
    pub fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(unsupported())
    }

    pub fn resize(&self, _cols: u16, _rows: u16, _cell_w: u16, _cell_h: u16) -> io::Result<()> {
        Err(unsupported())
    }
}

/// A session's program.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Program(());

/// How a program ended.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Exit {
    /// Its exit code; `None` when a signal killed it.
    pub code: Option<i32>,
    /// The signal that killed it (never on Windows).
    pub signal: Option<i32>,
}

impl Program {
    pub fn pid(&self) -> u32 {
        0
    }

    /// The processes to hang up and kill with this program.
    pub fn group(&self) -> Group {
        Group
    }

    /// Whether the program has exited, and how.
    pub fn try_wait(&mut self) -> io::Result<Option<Exit>> {
        Err(unsupported())
    }

    /// Waits for the program to exit.
    pub fn wait(&mut self) -> io::Result<()> {
        Err(unsupported())
    }
}

/// The processes that end with a session's program. It outlives the
/// program.
#[derive(Debug, Clone, Copy)]
pub struct Group;

impl Group {
    /// Tells the processes their terminal is gone.
    pub fn hang_up(self) {}

    /// Ends the processes.
    pub fn kill(self) {}

    /// Ends the processes that are still running.
    pub fn kill_remaining(self) {}
}

/// The program `request` runs: its command, or the default shell.
pub fn program_name(request: &PtyCommand) -> OsString {
    request.command.map(OsString::from).unwrap_or_default()
}

/// Opens a pseudoconsole and starts `request`'s program on it.
pub fn spawn(_request: &PtyCommand) -> io::Result<(Pty, Program)> {
    Err(unsupported())
}
