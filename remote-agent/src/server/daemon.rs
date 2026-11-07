//! Starting a server process detached: `--ready-fd N`, the double fork,
//! `setsid`, stdio to the log, and the one ready byte.
//!
//! The starter runs `brindle-remote <role> --ready-fd N` with the write end
//! of a pipe as fd `N`, and waits for one byte on the read end. EOF instead
//! means the server failed before it was listening (its log says why).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The pipe to write the ready byte to.
pub struct Ready(OwnedFd);

impl Ready {
    /// Takes ownership of an inherited fd, checking it is open and making it
    /// close-on-exec so session programs never hold it.
    pub fn from_fd(fd: RawFd) -> io::Result<Self> {
        if fd < 0 || unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("--ready-fd {fd} is not an open file descriptor"),
            ));
        }
        crate::pty::set_close_on_exec(fd)?;
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// Tells the starter the server is listening, and closes the pipe.
    pub fn signal(self) -> io::Result<()> {
        loop {
            let written = unsafe { libc::write(self.0.as_raw_fd(), b"R".as_ptr().cast(), 1) };
            if written == 1 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

/// Parses `--ready-fd N` from a role's arguments; nothing else is accepted.
pub fn parse_ready_fd(role: &str, args: &[String]) -> Result<Option<RawFd>, String> {
    match args {
        [] => Ok(None),
        [flag, fd] if flag == "--ready-fd" => fd
            .parse::<RawFd>()
            .ok()
            .filter(|fd| *fd >= 0)
            .map(Some)
            .ok_or_else(|| {
                format!("{role}: --ready-fd needs a file descriptor number, not {fd:?}")
            }),
        _ => Err(format!("usage: brindle-remote {role} [--ready-fd N]")),
    }
}

/// Opens (appending) a server log, user-only.
pub fn open_log(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// Detaches this process: forks twice so the server is in a new session
/// with no controlling terminal and isn't the starter's child, moves to
/// `/`, and points stdin at `/dev/null` and stdout and stderr at `log`.
/// Returns only in the detached grandchild; the original process and the
/// intermediate child exit. Call it while the process has one thread.
pub fn daemonize(log: &File) -> io::Result<()> {
    let null = File::open("/dev/null")?;
    match unsafe { libc::fork() } {
        -1 => return Err(io::Error::last_os_error()),
        0 => {}
        child => {
            // The starter waits on the ready pipe, not on this process, but
            // reap the intermediate child so it doesn't linger as a zombie.
            let mut status = 0;
            unsafe {
                libc::waitpid(child, &mut status, 0);
                libc::_exit(0);
            }
        }
    }
    if unsafe { libc::setsid() } == -1 {
        unsafe { libc::_exit(1) };
    }
    match unsafe { libc::fork() } {
        -1 => unsafe { libc::_exit(1) },
        0 => {}
        _ => unsafe { libc::_exit(0) },
    }
    unsafe {
        libc::chdir(c"/".as_ptr());
        libc::dup2(null.as_raw_fd(), libc::STDIN_FILENO);
        libc::dup2(log.as_raw_fd(), libc::STDOUT_FILENO);
        libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO);
    }
    Ok(())
}

/// Seconds since the epoch, for log lines.
pub fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn ready_fd_arguments() {
        assert_eq!(parse_ready_fd("hold", &[]), Ok(None));
        assert_eq!(
            parse_ready_fd("hold", &args(&["--ready-fd", "3"])),
            Ok(Some(3))
        );
        assert!(parse_ready_fd("hold", &args(&["--ready-fd", "-1"])).is_err());
        assert!(parse_ready_fd("hold", &args(&["--ready-fd"])).is_err());
        assert!(parse_ready_fd("hold", &args(&["--other", "3"])).is_err());
    }

    #[test]
    fn the_ready_byte_reaches_the_pipe() {
        use std::os::fd::IntoRawFd;

        let (read, write) = crate::server::event_loop::pipe().unwrap();
        let ready = Ready::from_fd(write.into_raw_fd()).unwrap();
        ready.signal().unwrap();
        let mut byte = [0u8; 2];
        let n = unsafe { libc::read(read.as_raw_fd(), byte.as_mut_ptr().cast(), 2) };
        assert_eq!(n, 1);
        // The write end was closed with the Ready.
        let n = unsafe { libc::read(read.as_raw_fd(), byte.as_mut_ptr().cast(), 2) };
        assert_eq!(n, 0);
        assert!(Ready::from_fd(-1).is_err());
    }
}
