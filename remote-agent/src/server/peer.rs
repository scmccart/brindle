//! The peer check both kinds of server socket make before reading anything:
//! the connecting process must run as the server's own user.

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};

/// Accepts the next connection waiting on a non-blocking `listener`, as
/// the core and the holders do. `Ok(None)` means none is waiting. A peer
/// that runs as another user (or whose user can't be told) is closed
/// before anything is read from it, and comes back as a `PermissionDenied`
/// error for the caller to log before it calls again.
pub fn accept(listener: &UnixListener, own_uid: u32) -> io::Result<Option<UnixStream>> {
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    };
    match check(stream.as_raw_fd(), own_uid) {
        Ok(()) => Ok(Some(stream)),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Err(error),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refused a connection whose user can't be told: {error}"),
        )),
    }
}

/// The user id of the process at the other end of a Unix stream socket.
pub fn peer_uid(fd: RawFd) -> io::Result<u32> {
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
pub fn check(fd: RawFd, own_uid: u32) -> io::Result<()> {
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
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use super::*;
    use crate::server::dir::own_uid;

    /// Sends a `HELLO`-sized message on `stream`, for a server to refuse.
    pub(crate) fn send_hello(stream: &mut UnixStream) {
        stream.write_all(&[2, 0, 0, 0, 2, b'{', b'}']).unwrap();
    }

    /// Whether the server closed `stream` without reading what was sent:
    /// closing a Unix stream socket with unread data gives the peer
    /// `ECONNRESET`, where a server that read it all gives EOF.
    pub(crate) fn closed_unread(stream: &mut UnixStream) -> bool {
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
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(accept(&listener, own_uid()).unwrap().is_none());

        let mut stream = UnixStream::connect(&socket).unwrap();
        send_hello(&mut stream);
        // As if this server ran as someone else.
        let error = accept(&listener, own_uid().wrapping_add(1)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(closed_unread(&mut stream));
        assert!(accept(&listener, own_uid()).unwrap().is_none());

        let mut stream = UnixStream::connect(&socket).unwrap();
        send_hello(&mut stream);
        let mut accepted = accept(&listener, own_uid()).unwrap().unwrap();
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
