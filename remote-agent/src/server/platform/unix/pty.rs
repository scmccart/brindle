//! A session's PTY and program for the holder, over [`crate::pty`]: the
//! master end's I/O and size, and the program's process group, which gets
//! the signals and is reaped.

use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::Child;

use super::event_loop::Source;
use crate::pty::{self, PtyCommand};

/// The master end of a session's PTY, non-blocking.
pub struct Pty(OwnedFd);

impl Pty {
    /// What an event loop waits on: readable while the program's output is
    /// waiting, writable while the PTY takes input.
    pub fn source(&self) -> Source {
        Source::of(&self.0)
    }

    /// Reads the program's output. `Ok(0)` means there will be no more;
    /// `WouldBlock` that none is waiting.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read =
            unsafe { libc::read(self.0.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if read >= 0 {
            return Ok(read as usize);
        }
        let error = io::Error::last_os_error();
        // Every slave fd is closed.
        if error.raw_os_error() == Some(libc::EIO) {
            return Ok(0);
        }
        Err(error)
    }

    /// Writes input for the program, as much as the PTY takes; `WouldBlock`
    /// when it takes none.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written =
            unsafe { libc::write(self.0.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if written > 0 {
            return Ok(written as usize);
        }
        Err(io::Error::last_os_error())
    }

    pub fn resize(
        &self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> io::Result<()> {
        pty::resize(self.0.as_raw_fd(), cols, rows, cell_width, cell_height)
    }
}

/// A session's program: the child process, which leads a session and a
/// process group of its own.
pub struct Program {
    child: Child,
    pid: libc::pid_t,
}

/// How a program ended.
pub struct Exit {
    /// Its exit code; `None` when a signal killed it.
    pub code: Option<i32>,
    /// The signal that killed it.
    pub signal: Option<i32>,
}

impl Program {
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// The processes to hang up and kill with this program.
    pub fn group(&self) -> Group {
        Group(self.pid)
    }

    /// Reaps the program if it has exited.
    pub fn try_wait(&mut self) -> io::Result<Option<Exit>> {
        Ok(self.child.try_wait()?.map(|status| Exit {
            code: status.code(),
            signal: status.signal(),
        }))
    }

    /// Waits for the program to exit, and reaps it.
    pub fn wait(&mut self) -> io::Result<()> {
        self.child.wait().map(drop)
    }
}

/// The process group a session's program leads. It outlives the program:
/// children that ignored the hang-up are still in it.
#[derive(Debug, Clone, Copy)]
pub struct Group(libc::pid_t);

impl Group {
    /// Sends `SIGHUP`, as closing its terminal does.
    pub fn hang_up(self) {
        pty::signal_process_group(self.0, libc::SIGHUP);
    }

    /// Sends `SIGKILL`.
    pub fn kill(self) {
        pty::signal_process_group(self.0, libc::SIGKILL);
    }

    /// Sends `SIGKILL` if the group still has members. A group id can't be
    /// reused while it has, the reaped leader's children that ignored
    /// `SIGHUP` included.
    pub fn kill_remaining(self) {
        if self.0 > 0 && unsafe { libc::kill(-self.0, 0) } == 0 {
            self.kill();
        }
    }
}

/// The program `request` runs: its command, or the login shell.
pub fn program_name(request: &PtyCommand) -> OsString {
    request.program()
}

/// Opens a PTY and starts `request`'s program on it.
pub fn spawn(request: &PtyCommand) -> io::Result<(Pty, Program)> {
    let spawned = pty::spawn(request)?;
    Ok((
        Pty(spawned.master),
        Program {
            child: spawned.child,
            pid: spawned.pid,
        },
    ))
}
