//! The server sockets: a Unix stream socket in the server directory, and
//! the peer check both kinds of server socket make before reading anything:
//! the connecting process must run as the server's own user.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use super::conn::Stream;
use super::event_loop::Source;

/// Who a server runs as, which its peers must match: the user id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity(u32);

impl Identity {
    /// This process's own.
    pub fn own() -> Self {
        Self(super::dir::own_uid())
    }

    /// Someone else's, for a server that runs, as far as its peer check is
    /// concerned, as another user.
    #[cfg(test)]
    pub fn other(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Fails if `path` is too long for a socket address.
pub fn check_path(path: &Path) -> io::Result<()> {
    // sun_path holds 108 bytes with the terminating NUL.
    if path.as_os_str().len() > 107 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the socket path {} is too long for a Unix socket; set a shorter BRINDLE_SERVER_DIR",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Connects to the server listening at `path`, blocking.
pub fn connect(path: &Path) -> io::Result<Stream> {
    UnixStream::connect(path).map(Stream::new)
}

/// A non-blocking listening socket.
pub struct Listener {
    listener: UnixListener,
    identity: Identity,
}

impl Listener {
    /// Listens at `path` for peers that run as `identity`. The caller holds
    /// the lock that goes with `path`, so a socket already there is stale
    /// and is removed first.
    pub fn bind(path: &Path, identity: Identity) -> io::Result<Self> {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("can't listen on {}: {error}", path.display()),
            )
        })?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, identity })
    }

    /// What an event loop waits on: readable when a connection is waiting.
    pub fn source(&self) -> Source {
        Source::of(&self.listener)
    }

    /// Accepts the next connection waiting, as the core and the holders
    /// do. `Ok(None)` means none is waiting. A peer that runs as another
    /// user (or whose user can't be told) is closed before anything is read
    /// from it, and comes back as a `PermissionDenied` error for the caller
    /// to log before it calls again.
    pub fn accept(&self) -> io::Result<Option<Stream>> {
        let stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        };
        match check(stream.as_raw_fd(), self.identity.0) {
            Ok(()) => Ok(Some(Stream::new(stream))),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Err(error),
            Err(error) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("refused a connection whose user can't be told: {error}"),
            )),
        }
    }
}

/// The user id of the process at the other end of a Unix stream socket.
fn peer_uid(fd: RawFd) -> io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(credentials.uid)
}

/// Accepts the peer only when it runs as `own_uid`.
fn check(fd: RawFd, own_uid: u32) -> io::Result<()> {
    let uid = peer_uid(fd)?;
    if uid != own_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refused a connection from uid {uid}; this server runs as uid {own_uid}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Read, Write};
    use std::time::Duration;

    use super::super::dir::own_uid;
    use super::*;

    /// A client's end of a connection to a server under test.
    pub(crate) struct TestPeer(UnixStream);

    impl TestPeer {
        pub(crate) fn connect(path: &Path) -> Self {
            Self(UnixStream::connect(path).unwrap())
        }

        /// Sends a `HELLO`-sized message, for a server to refuse.
        pub(crate) fn send_hello(&mut self) {
            send_hello(&mut self.0);
        }

        /// Whether the server closed the connection without reading what
        /// was sent.
        pub(crate) fn closed_unread(&mut self) -> bool {
            closed_unread(&mut self.0)
        }
    }

    fn send_hello(stream: &mut UnixStream) {
        stream.write_all(&[2, 0, 0, 0, 2, b'{', b'}']).unwrap();
    }

    /// Closing a Unix stream socket with unread data gives the peer
    /// `ECONNRESET`, where a server that read it all gives EOF.
    fn closed_unread(stream: &mut UnixStream) -> bool {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut byte = [0u8; 1];
        matches!(
            stream.read(&mut byte),
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset
        )
    }

    #[test]
    fn closing_unread_is_told_from_closing_after_reading() {
        let (mut client, server) = UnixStream::pair().unwrap();
        send_hello(&mut client);
        drop(server);
        assert!(closed_unread(&mut client));

        let (mut client, mut server) = UnixStream::pair().unwrap();
        send_hello(&mut client);
        let mut message = [0u8; 7];
        server.read_exact(&mut message).unwrap();
        drop(server);
        assert!(!closed_unread(&mut client));
    }

    #[test]
    fn accept_closes_a_foreign_uid_without_reading_it() {
        let temp = crate::server::dir::tests::TempDir::new("peer-accept");
        let socket = temp.0.join("s.sock");
        let listener = Listener::bind(&socket, Identity::own()).unwrap();
        assert!(listener.accept().unwrap().is_none());

        let mut stream = UnixStream::connect(&socket).unwrap();
        send_hello(&mut stream);
        // As if this server ran as someone else.
        let foreign = Listener {
            listener: listener.listener.try_clone().unwrap(),
            identity: Identity::own().other(),
        };
        let error = foreign.accept().err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(closed_unread(&mut stream));
        assert!(listener.accept().unwrap().is_none());

        let mut stream = UnixStream::connect(&socket).unwrap();
        send_hello(&mut stream);
        let mut accepted = listener.accept().unwrap().unwrap();
        let mut message = [0u8; 7];
        accepted.read_exact(&mut message).unwrap();
        assert_eq!(message[0], 2);
    }

    #[test]
    fn the_peer_must_be_the_same_user() {
        let (a, b) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(a.as_raw_fd()).unwrap(), own_uid());
        check(a.as_raw_fd(), own_uid()).unwrap();
        // As if this server ran as someone else.
        let error = check(b.as_raw_fd(), own_uid().wrapping_add(1)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("refused"), "{error}");
    }

    #[test]
    fn a_non_socket_has_no_peer() {
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(peer_uid(file.as_raw_fd()).is_err());
    }
}
