//! The ready pipe: how a terminal server role (`serve` or `hold`) tells the
//! process that started it that it is listening. Brindle starts the core, a
//! core starts its holders and a holder starts a core when it has none, so
//! the byte and the rule for reading it are here for all of them.
//!
//! The starter gives the role the write end of a pipe (`--ready-fd N` on
//! Linux, `--ready-handle N` on Windows). Once the role listens, it writes
//! [`BYTE`] and closes the pipe at once. A role that fails first writes
//! nothing, so its pipe just ends.
//!
//! On Linux a starter reads the pipe to its end, and the role is ready only
//! if the pipe held exactly that one byte ([`Signal`]). The first byte alone
//! doesn't say so. Rust's `Command` reports a failed exec through a
//! close-on-exec pipe of its own in the child. When that pipe's write end is
//! the fd the ready pipe is passed as (fd 3), the `dup2` that passes the ready
//! pipe replaces it, and the 8 bytes of the exec error arrive in the ready
//! pipe instead.
//!
//! A Windows starter takes the first byte as it comes. There the pipe is
//! passed as a handle that only the role ever writes to: a start that fails
//! is an error from `CreateProcessW`, not bytes in a pipe. And the pipe's
//! end can't be waited for, since a process that another thread starts
//! meanwhile can inherit the handle and keep the pipe open.

/// What a role writes to its ready pipe once it is listening.
pub const BYTE: u8 = b'R';

/// What a starter has read from a role's ready pipe so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signal {
    /// How many bytes the pipe has held.
    len: usize,
    /// They are the ready byte and nothing else.
    exact: bool,
}

impl Signal {
    /// Takes the bytes that one read of the pipe returned.
    pub fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.exact = self.len == 0 && bytes == [BYTE];
        self.len = self.len.saturating_add(bytes.len());
    }

    /// How many bytes the pipe has held, for messages.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the pipe has held nothing.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the role is listening: the pipe held the ready byte and
    /// nothing else. Only the pipe's end says so, since more may follow
    /// until then.
    pub fn is_ready(&self) -> bool {
        self.exact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(chunks: &[&[u8]]) -> Signal {
        let mut signal = Signal::default();
        for chunk in chunks {
            signal.push(chunk);
        }
        signal
    }

    #[test]
    fn only_the_ready_byte_alone_is_ready() {
        assert!(read(&[b"R"]).is_ready());
        // A read that returned nothing changes nothing.
        assert!(read(&[b"", b"R", b""]).is_ready());
        assert_eq!(read(&[b"R"]).len(), 1);

        // A role that failed before it listened wrote nothing.
        assert!(!read(&[]).is_ready());
        assert!(read(&[]).is_empty());
        // Another byte isn't the ready byte.
        assert!(!read(&[b"r"]).is_ready());
        assert!(!read(&[b"\0"]).is_ready());
        // And neither is more than it, however the reads split it.
        assert!(!read(&[b"RR"]).is_ready());
        assert!(!read(&[b"R", b"R"]).is_ready());
        assert!(!read(&[b"R", b"\n"]).is_ready());
        assert!(!read(&[b"x", b"R"]).is_ready());
    }

    #[test]
    fn a_failed_execs_error_is_not_ready() {
        // What `Command` writes when the exec fails: the errno (ENOENT
        // here), big-endian, then "NOEX".
        let error = [0, 0, 0, 2, b'N', b'O', b'E', b'X'];
        let signal = read(&[&error]);
        assert_eq!((signal.is_ready(), signal.len()), (false, 8));
        // Nor is one whose errno ends in the ready byte's value, read a
        // byte at a time.
        let error = [0, 0, 0, BYTE, b'N', b'O', b'E', b'X'];
        let split: Vec<&[u8]> = error.chunks(1).collect();
        assert!(!read(&split).is_ready());
        // Not even with the ready byte first.
        let error = [BYTE, 0, 0, 0, b'N', b'O', b'E', b'X'];
        let split: Vec<&[u8]> = error.chunks(1).collect();
        assert!(!read(&split).is_ready());
    }
}
