//! A session's program on plain pipes for the holder (H1 version 3's
//! `SPAWN_PIPES`): its standard input, output and error, each an event-loop
//! [`Source`] read and written through its owner (`process R8`), and the
//! [`Program`] and its group, shared with the PTY sessions, for its exit and
//! hang-up.
//!
//! Declared with the same items on every platform so the holder names none
//! of their types (`ports R5`). For now [`spawn_piped`] refuses: the pipes
//! and the spawn come with `agents-sessions` section 5, so no [`Piped`]
//! exists yet and its methods can't be reached.

use std::collections::BTreeMap;
use std::io;

use super::event_loop::Source;
use super::pty::Program;

/// What to run on pipes. `env` is the program's whole environment.
pub struct PipeCommand<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
}

/// No value: a [`Piped`] can't be made until [`spawn_piped`] can start one.
enum Unstarted {}

/// The holder's ends of a program's three pipes, non-blocking.
pub struct Piped(Unstarted);

impl Piped {
    /// Writable while the program's stdin takes input.
    pub fn input_source(&self) -> Source {
        match self.0 {}
    }

    /// Readable while the program's stdout has output waiting.
    pub fn output_source(&self) -> Source {
        match self.0 {}
    }

    /// Readable while the program's stderr has bytes waiting.
    pub fn error_source(&self) -> Source {
        match self.0 {}
    }

    /// Writes to the program's stdin.
    pub fn write_input(&self, _bytes: &[u8]) -> io::Result<usize> {
        match self.0 {}
    }

    /// Closes the program's stdin, its first sign to end.
    pub fn close_input(&mut self) {
        match self.0 {}
    }

    /// Reads the program's stdout. `Ok(0)` means there will be no more.
    pub fn read_output(&self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.0 {}
    }

    /// Reads the program's stderr. `Ok(0)` means there will be no more.
    pub fn read_error(&self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.0 {}
    }
}

/// Starts `request`'s program with its standard streams on pipes, in a
/// group (a job on Windows) of its own.
pub fn spawn_piped(request: &PipeCommand) -> io::Result<(Piped, Program)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "can't run {} on pipes: this holder doesn't start pipe sessions yet",
            request.command
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_spawn_is_refused_as_unsupported_for_now() {
        let env = BTreeMap::new();
        let request = PipeCommand {
            command: "agent",
            args: &[],
            cwd: None,
            env: &env,
        };
        // No `Piped` can be made, so an `Ok` can't be either.
        let Err(error) = spawn_piped(&request);
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("agent"), "{error}");
    }
}
