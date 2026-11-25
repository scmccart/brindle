//! A session's program on plain pipes for the holder (H1 version 3's
//! `SPAWN_PIPES`): its standard input, output and error, each an event-loop
//! [`Source`] read and written through its owner (`process R8`), and the
//! [`Program`] and its group, shared with the PTY sessions, for its exit and
//! hang-up.
//!
//! Declared with the same items on every platform so the holder names none
//! of their types (`ports R5`). Here the pipes are three `pipe2(O_CLOEXEC)`
//! pairs, the holder's ends non-blocking. The program gets the other ends
//! as its fds 0, 1 and 2, becomes a session and process group leader of its
//! own (`setsid`, so it has no controlling terminal), and starts with
//! default signal handling, as a PTY's program does. Its directory is
//! chosen as a PTY session's is ([`crate::pty::start_directory`]), and its
//! environment is exactly the request's.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use super::super::piped::agent_dir;
use super::event_loop::Source;
use super::pty::Program;
use crate::pty::{default_signals, home_dir, set_nonblocking, start_directory};

/// What to run on pipes. `env` is the program's whole environment.
pub struct PipeCommand<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
}

/// The absolute directory an agent session asking for `cwd`, with the
/// environment `env`, works in (`piped::agent_dir`): `~` is the
/// environment's home ([`crate::pty::home_dir`], as for a PTY session), and
/// with neither a usable directory nor a home it is the core's own
/// directory.
pub fn agent_directory(cwd: Option<&str>, env: &BTreeMap<String, String>) -> String {
    let home = home_dir(env);
    agent_dir(cwd, home.as_deref(), Path::is_absolute, Path::is_dir)
        .or_else(|| std::env::current_dir().ok())
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_owned())
}

/// The holder's ends of a program's three pipes, non-blocking.
pub struct Piped {
    /// The write end of the program's stdin; `None` once closed.
    input: Option<OwnedFd>,
    /// The read end of its stdout.
    output: OwnedFd,
    /// The read end of its stderr.
    error: OwnedFd,
}

impl Piped {
    /// Writable while the program's stdin takes input; `None` once it was
    /// closed ([`Piped::close_input`]).
    pub fn input_source(&self) -> Option<Source> {
        self.input.as_ref().map(Source::of)
    }

    /// Readable while the program's stdout has output waiting.
    pub fn output_source(&self) -> Source {
        Source::of(&self.output)
    }

    /// Readable while the program's stderr has bytes waiting.
    pub fn error_source(&self) -> Source {
        Source::of(&self.error)
    }

    /// Writes to the program's stdin, as much as the pipe takes;
    /// `WouldBlock` when it takes none, `BrokenPipe` once it is closed.
    pub fn write_input(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(input) = &self.input else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the program's stdin is closed",
            ));
        };
        // SAFETY: the fd is open while `self` holds it, and `bytes` outlives
        // the call.
        let written = unsafe { libc::write(input.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if written >= 0 {
            return Ok(written as usize);
        }
        Err(io::Error::last_os_error())
    }

    /// Closes the program's stdin, its first sign to end.
    pub fn close_input(&mut self) {
        self.input = None;
    }

    /// Reads the program's stdout. `Ok(0)` means there will be no more;
    /// `WouldBlock` that none is waiting.
    pub fn read_output(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        read(&self.output, buffer)
    }

    /// Reads the program's stderr, as [`Piped::read_output`] reads stdout.
    pub fn read_error(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        read(&self.error, buffer)
    }
}

fn read(fd: &OwnedFd, buffer: &mut [u8]) -> io::Result<usize> {
    // SAFETY: the fd is open, and `buffer` is writable for its length.
    let read = unsafe { libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
    if read >= 0 {
        return Ok(read as usize);
    }
    Err(io::Error::last_os_error())
}

/// A pipe's ends, both close-on-exec: `(read, write)`.
fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: `fds` has room for the two fds `pipe2` writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds are new and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Starts `request`'s program with its standard streams on pipes, in a
/// session and process group of its own.
pub fn spawn_piped(request: &PipeCommand) -> io::Result<(Piped, Program)> {
    let (child_input, input) = pipe()?;
    let (output, child_output) = pipe()?;
    let (error, child_error) = pipe()?;
    for fd in [&input, &output, &error] {
        set_nonblocking(fd.as_raw_fd())?;
    }

    let mut command = Command::new(request.command);
    command
        .env_clear()
        .envs(request.env)
        .args(request.args)
        .stdin(Stdio::from(File::from(child_input)))
        .stdout(Stdio::from(File::from(child_output)))
        .stderr(Stdio::from(File::from(child_error)));
    if let Some(dir) = request.cwd.and_then(|cwd| start_directory(cwd, request.env)) {
        command.current_dir(dir);
    }
    // Asked here, since the child may make only async-signal-safe calls.
    let last_signal = libc::SIGRTMAX();
    // SAFETY: the closure makes only async-signal-safe calls, as `pre_exec`
    // needs.
    unsafe {
        command.pre_exec(move || {
            default_signals(last_signal)?;
            // Its own session, so no controlling terminal, and its own
            // process group for the hang-up.
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // The child's ends close here, in `command`'s drop, so the holder's
    // read ends see the end once the program and what it started close
    // theirs.
    let child = command.spawn()?;
    Ok((
        Piped {
            input: Some(input),
            output,
            error,
        },
        Program::of(child),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        command: &'a str,
        args: &'a [String],
        env: &'a BTreeMap<String, String>,
    ) -> PipeCommand<'a> {
        PipeCommand {
            command,
            args,
            cwd: None,
            env,
        }
    }

    /// Reads `read` to its end, which has come: every writer has exited.
    fn read_to_end(mut read: impl FnMut(&mut [u8]) -> io::Result<usize>) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            match read(&mut buffer) {
                Ok(0) => return bytes,
                Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                Err(error) => panic!("after {bytes:?}: {error}"),
            }
        }
    }

    #[test]
    fn a_missing_program_fails_to_start() {
        let env = BTreeMap::new();
        let result = spawn_piped(&request("/nonexistent/brindle-agent", &[], &env));
        let Err(error) = result else {
            panic!("a missing program started");
        };
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn the_three_streams_are_the_programs_own() {
        let env = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        let args = ["-c".to_string(), "cat; echo err >&2".to_string()];
        let (mut piped, mut program) = spawn_piped(&request("/bin/sh", &args, &env)).unwrap();
        assert_eq!(piped.write_input(b"a\tb\r\n").unwrap(), 5);
        piped.close_input();
        assert!(piped.input_source().is_none());
        assert_eq!(
            piped.write_input(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        // It ends at its input's end; then every byte is in the pipes.
        program.wait().unwrap();
        assert_eq!(read_to_end(|buffer| piped.read_output(buffer)), b"a\tb\r\n");
        assert_eq!(read_to_end(|buffer| piped.read_error(buffer)), b"err\n");
    }
}
