//! Starting a server process detached: `--ready-fd N`, the double fork,
//! `setsid`, stdio to the log, and the one ready byte.
//!
//! The starter runs `brindle-remote <role> --ready-fd N` with the write end
//! of a pipe as fd `N`, and reads the other end until it ends. The server
//! writes one byte once it is listening and closes the pipe at once, so that
//! byte and nothing else means it is listening ([`ready`] has the rule and
//! why the first byte alone doesn't). A pipe that ends without it means the
//! server failed before it was listening (its log says why).

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use brindle_remote_protocol::{ready, scope};

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

    /// Tells the starter the server is listening, and closes the pipe. The
    /// starter waits for both: the byte counts only at the pipe's end.
    pub fn signal(self) -> io::Result<()> {
        let byte = [ready::BYTE];
        loop {
            let written = unsafe { libc::write(self.0.as_raw_fd(), byte.as_ptr().cast(), 1) };
            if written == 1 {
                // Dropping `self` closes the pipe.
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
pub struct ReadyPipe {
    pipe: OwnedFd,
    /// What the pipe has held so far.
    signal: ready::Signal,
}

impl ReadyPipe {
    fn new(pipe: OwnedFd) -> Self {
        Self {
            pipe,
            signal: ready::Signal::default(),
        }
    }

    /// What an event loop waits on: readable when the role has written to
    /// the pipe, and when the pipe has ended.
    pub fn source(&self) -> Source {
        Source::of(&self.pipe)
    }

    /// Whether the role is listening (`true`) or ended before it was
    /// (`false`); `None` while neither is known, which is until the pipe
    /// ends. Only then is the ready byte known to be all the pipe held: the
    /// first bytes of a failed exec's error aren't a ready byte
    /// ([`ready`]).
    ///
    /// Each call reads once, and the loop calls again while there is more
    /// to read or the end to see.
    pub fn read(&mut self) -> Option<bool> {
        let mut buffer = [0u8; 16];
        let read = unsafe {
            libc::read(
                self.pipe.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        match read {
            0 => Some(self.signal.is_ready()),
            1.. => {
                self.signal.push(&buffer[..read as usize]);
                None
            }
            _ => match io::Error::last_os_error().kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => None,
                // Its end can't be read, so nothing says it is listening.
                _ => Some(false),
            },
        }
    }

    /// [`ReadyPipe::read`]'s answer, waiting up to `limit` for it: for a
    /// starter with no event loop (the relay). `None` if the pipe hasn't
    /// ended by then.
    pub fn wait(&mut self, limit: Duration) -> Option<bool> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(ready) = self.read() {
                return Some(ready);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut pollfd = libc::pollfd {
                fd: self.pipe.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // Rounded up, so the wait doesn't end before the deadline. An
            // interrupted or failed poll just reads again.
            let wait = left.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as libc::c_int;
            unsafe { libc::poll(&mut pollfd, 1, wait) };
        }
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
    let decision = scope::current();
    if let scope::Decision::Plain(reason) = &decision {
        unscoped(reason);
    }
    start_role_as(&decision, role, args, env, |_| {})
}

/// [`start_role`] with the scope decided by the caller, and `configure`
/// given the command before it runs: the relay points the role's stdout
/// away from its own, which carries frames.
pub fn start_role_as(
    decision: &scope::Decision,
    role: &str,
    args: &[String],
    env: &[(&str, &OsStr)],
    configure: impl FnOnce(&mut Command),
) -> io::Result<Started> {
    let args: Vec<OsString> = args
        .iter()
        .map(|arg| arg.replace(READY_PLACEHOLDER, &READY_FD.to_string()).into())
        .collect();
    let program = OsString::from(format!("/proc/{}/exe", std::process::id()));
    let mut command = scope::command(decision, &scope::new_unit_name(role), &program, &args);
    command.envs(env.iter().copied());
    configure(&mut command);
    let (child, ready) = spawn_with_ready_pipe(&mut command, READY_FD)?;
    Ok(Started {
        child,
        ready: ReadyPipe::new(ready),
        program,
    })
}

/// Spawns a server's starter (`command`, such as `brindle-remote hold
/// --ready-fd 3`) with stdin on `/dev/null` and the write end of a new pipe
/// as fd `ready_fd`. Returns the child, which exits once the server has
/// detached, and the pipe's non-blocking read end: the ready byte and then
/// the pipe's end arrive once the server is listening, or just its end if
/// the server failed first.
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

    /// A new ready pipe: the end a starter reads, and the role's end.
    fn ready_pipe() -> (ReadyPipe, OwnedFd) {
        let (read, write) = super::super::event_loop::pipe().unwrap();
        (ReadyPipe::new(read), write)
    }

    fn write(pipe: &OwnedFd, bytes: &[u8]) {
        let written = unsafe { libc::write(pipe.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize);
    }

    /// Reads as a loop does, again while there is more to read: whether the
    /// role is listening, or `None` while the pipe hasn't ended.
    fn outcome(ready: &mut ReadyPipe) -> Option<bool> {
        // More reads than any of these pipes takes.
        (0..8).find_map(|_| ready.read())
    }

    #[test]
    fn the_ready_byte_reaches_the_pipe() {
        use std::os::fd::IntoRawFd;

        let (mut read, role) = ready_pipe();
        assert_eq!(read.read(), None);
        let ready = Ready::from_fd(role.into_raw_fd()).unwrap();
        ready.signal().unwrap();
        // The byte and the pipe's end: the write end was closed with the
        // `Ready`.
        assert_eq!(outcome(&mut read), Some(true));
        assert!(Ready::from_fd(-1).is_err());
    }

    #[test]
    fn the_ready_byte_counts_only_at_the_pipes_end() {
        let (mut read, role) = ready_pipe();
        write(&role, &[ready::BYTE]);
        // More may follow it: a failed exec's error starts with a byte too.
        assert_eq!(outcome(&mut read), None);
        drop(role);
        assert_eq!(outcome(&mut read), Some(true));
    }

    #[test]
    fn anything_but_the_one_ready_byte_is_a_role_that_isnt_listening() {
        let cases: [&[u8]; 4] = [
            // The role ended before it was listening.
            b"",
            // A failed exec's error, as `Command` writes it: the errno
            // (ENOENT) and "NOEX".
            &[0, 0, 0, 2, b'N', b'O', b'E', b'X'],
            b"RR",
            b"x",
        ];
        for bytes in cases {
            let (mut read, role) = ready_pipe();
            write(&role, bytes);
            drop(role);
            assert_eq!(outcome(&mut read), Some(false), "{bytes:?}");
        }
        // Whatever follows the ready byte, and whenever.
        let (mut read, role) = ready_pipe();
        write(&role, &[ready::BYTE]);
        assert_eq!(outcome(&mut read), None);
        write(&role, &[ready::BYTE]);
        drop(role);
        assert_eq!(outcome(&mut read), Some(false));
    }

    #[test]
    fn a_starter_without_a_loop_waits_for_the_pipes_end() {
        // Nothing yet: the wait ends with the limit.
        let (mut read, role) = ready_pipe();
        let begin = Instant::now();
        assert_eq!(read.wait(Duration::from_millis(50)), None);
        assert!(begin.elapsed() >= Duration::from_millis(50));
        // The byte and the end, the end coming later.
        write(&role, &[ready::BYTE]);
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(role);
        });
        assert_eq!(read.wait(Duration::from_secs(30)), Some(true));
        closer.join().unwrap();
        // A role that ended first.
        let (mut read, role) = ready_pipe();
        drop(role);
        assert_eq!(read.wait(Duration::from_secs(30)), Some(false));
    }

    #[test]
    fn a_loop_waits_for_the_pipes_end_without_spinning() {
        use std::time::Duration;

        use super::super::event_loop::PollLoop;
        use crate::server::event_loop::{EventLoop, Interest};

        let (mut read, role) = ready_pipe();
        let mut ev = PollLoop::new().unwrap();
        ev.register(&read.source(), 1, Interest::READABLE).unwrap();
        let mut events = Vec::new();
        write(&role, &[ready::BYTE]);
        ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(read.read(), None);
        // The byte is taken, so until the pipe ends there is nothing to
        // wake up for.
        ev.wait(&mut events, Some(Duration::from_millis(20)))
            .unwrap();
        assert!(events.is_empty(), "{events:?}");
        drop(role);
        ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(read.read(), Some(true));
    }
}
