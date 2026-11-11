//! Starting a server process detached: `--ready-fd N`, the double fork,
//! `setsid`, stdio to the log, and the one ready byte.
//!
//! The starter runs `brindle-remote <role> --ready-fd N` with the write end
//! of a pipe as fd `N`, and waits for one byte on the read end. EOF instead
//! means the server failed before it was listening (its log says why).

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use brindle_remote_protocol::scope;

use super::event_loop::Source;

/// The fd a started role's ready pipe gets.
const READY_FD: libc::c_int = 3;
/// Stands for [`READY_FD`] in a role's arguments.
const READY_PLACEHOLDER: &str = "{ready_fd}";

/// The pipe to write the ready byte to.
pub struct Ready(OwnedFd);

impl Ready {
    /// Takes ownership of an inherited fd, checking it is open and making it
    /// close-on-exec so session programs never hold it.
    fn from_fd(fd: RawFd) -> io::Result<Self> {
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

/// The ready pipe a role was started with, from its arguments: `--ready-fd
/// N`, or nothing for a role that stays in the foreground. Nothing else is
/// accepted.
pub fn ready_from_args(role: &str, args: &[String]) -> Result<Option<Ready>, String> {
    parse_ready_fd(role, args)?
        .map(Ready::from_fd)
        .transpose()
        .map_err(|error| error.to_string())
}

fn parse_ready_fd(role: &str, args: &[String]) -> Result<Option<RawFd>, String> {
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

/// The arguments that start `role` detached with [`start_role`]:
/// `<role> --ready-fd {ready_fd}`.
pub fn role_args(role: &str) -> Vec<String> {
    vec![role.into(), "--ready-fd".into(), READY_PLACEHOLDER.into()]
}

/// The read end of a started role's ready pipe.
pub struct ReadyPipe(OwnedFd);

impl ReadyPipe {
    /// What an event loop waits on: readable once the role is listening or
    /// has failed.
    pub fn source(&self) -> Source {
        Source::of(&self.0)
    }

    /// Whether the role is listening (`true`) or ended before it was
    /// (`false`); `None` while neither is known.
    pub fn read(&self) -> Option<bool> {
        let mut buffer = [0u8; 16];
        let read =
            unsafe { libc::read(self.0.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if read < 0 && io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
            return None;
        }
        Some(read > 0)
    }
}

/// A role started by [`start_role`].
pub struct Started {
    /// The starter, which exits once the role has detached.
    pub child: Child,
    pub ready: ReadyPipe,
    /// What was run, for the log.
    pub program: OsString,
}

/// Starts a role of this process's own binary, detached: `args` (see
/// [`role_args`]; `{ready_fd}` in them stands for the ready pipe's fd
/// number) with `env` added to this process's environment.
///
/// The binary is run as `/proc/<pid>/exe`, which names it even after an
/// install replaced the file, in a systemd scope of its own when there is a
/// user manager. Otherwise `unscoped` is told why not, before the start.
pub fn start_role(
    role: &str,
    args: &[String],
    env: &[(&str, &OsStr)],
    unscoped: impl FnOnce(&str),
) -> io::Result<Started> {
    let args: Vec<OsString> = args
        .iter()
        .map(|arg| arg.replace(READY_PLACEHOLDER, &READY_FD.to_string()).into())
        .collect();
    let program = OsString::from(format!("/proc/{}/exe", std::process::id()));
    let decision = scope::current();
    if let scope::Decision::Plain(reason) = &decision {
        unscoped(reason);
    }
    let mut command = scope::command(&decision, role, &program, &args);
    command.envs(env.iter().copied());
    let (child, ready) = spawn_with_ready_pipe(&mut command, READY_FD)?;
    Ok(Started {
        child,
        ready: ReadyPipe(ready),
        program,
    })
}

/// Spawns a server's starter (`command`, such as `brindle-remote hold
/// --ready-fd 3`) with stdin on `/dev/null` and the write end of a new pipe
/// as fd `ready_fd`. Returns the child, which exits once the server has
/// detached, and the pipe's non-blocking read end: one byte arrives once the
/// server is listening, or EOF if it failed first.
fn spawn_with_ready_pipe(
    command: &mut Command,
    ready_fd: libc::c_int,
) -> io::Result<(Child, OwnedFd)> {
    let (ready_read, ready_write) = super::event_loop::pipe()?;
    let write_fd = ready_write.as_raw_fd();
    command.stdin(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if write_fd == ready_fd {
                let flags = libc::fcntl(write_fd, libc::F_GETFD);
                if flags == -1
                    || libc::fcntl(write_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1
                {
                    return Err(io::Error::last_os_error());
                }
            } else if libc::dup2(write_fd, ready_fd) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    drop(ready_write);
    Ok((child, ready_read))
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
        assert_eq!(
            role_args("hold"),
            args(&["hold", "--ready-fd", "{ready_fd}"])
        );
    }

    #[test]
    fn the_ready_byte_reaches_the_pipe() {
        use std::os::fd::IntoRawFd;

        let (read, write) = super::super::event_loop::pipe().unwrap();
        let read = ReadyPipe(read);
        assert_eq!(read.read(), None);
        let ready = Ready::from_fd(write.into_raw_fd()).unwrap();
        ready.signal().unwrap();
        assert_eq!(read.read(), Some(true));
        // The write end was closed with the Ready.
        assert_eq!(read.read(), Some(false));
        assert!(Ready::from_fd(-1).is_err());
    }
}
