//! The peer check both kinds of server socket make before reading anything:
//! the connecting process must run as the server's own user.

use std::io;
use std::os::fd::RawFd;

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
mod tests {
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::server::dir::own_uid;

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
