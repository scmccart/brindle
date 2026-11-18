//! What the client needs from Linux: the core's Unix socket, the starter
//! (`serve --ready-fd 3`, in a systemd scope of its own where there is one),
//! and a process's image and end.

use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

use super::{Endpoint, Start};
use crate::remote::protocol::{ready, scope, server_dir};

/// The connection to the core.
pub type Stream = UnixStream;

/// Where the core of the server directory `dir` listens, for messages.
pub fn address(dir: &Path) -> String {
    dir.join(server_dir::CORE_SOCKET).display().to_string()
}

/// Connects to the core of the server directory `dir`. `NotFound` or
/// `ConnectionRefused` when none listens. A socket's backlog takes the
/// connection while the core is busy, so `_busy` (how long Windows waits for
/// a free pipe instance) isn't needed.
pub fn connect(dir: &Path, _busy: Duration) -> io::Result<Stream> {
    UnixStream::connect(dir.join(server_dir::CORE_SOCKET))
}

/// Runs `<helper> serve --ready-fd 3`, with the endpoint's role variables
/// over this process's environment, and waits up to `limit` for it to be
/// ready: for its ready pipe to end, having held the ready byte and nothing
/// else ([`ready`]). The core detaches itself (double fork, `setsid`), so it
/// is never Brindle's child and outlives it.
pub fn start(endpoint: &Endpoint, limit: Duration) -> Result<Start> {
    let mut fds = [-1; 2];
    // SAFETY: `pipe2` fills both fds on success; they are owned below.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error()).context("starting the terminal server: pipe");
    }
    // SAFETY: freshly created and not owned elsewhere.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let decision = match endpoint.scope {
        true => scope::current(),
        false => scope::Decision::Plain("the endpoint uses no scope"),
    };
    if let scope::Decision::Plain(reason) = &decision {
        log::info!("starting the terminal server without a scope of its own: {reason}");
    }
    let args = ["serve".into(), "--ready-fd".into(), "3".into()];
    let mut command = scope::command(
        &decision,
        &scope::new_unit_name("serve"),
        endpoint.helper.as_os_str(),
        &args,
    );
    command
        .envs(endpoint.role_env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || pass_fd_as(write_fd, 3));
    }
    let child = command.spawn().with_context(|| {
        format!("terminal server didn't start: can't run {}", endpoint.helper.display())
    })?;
    drop(write);
    // `None` when the pipe didn't end within the limit.
    let signal = read_ready_pipe(&read, limit);
    // The starter exits once the core has detached (or failed first).
    let output = child.wait_with_output().context("waiting for the terminal server to start")?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok(match signal {
        Some(signal) if signal.is_ready() => Start::Ready,
        Some(_) => Start::Exited { status: output.status.to_string(), stderr },
        None => Start::TimedOut { stderr },
    })
}

/// For a forked child, before its exec: makes `fd` the child's fd `target`,
/// open in the program it runs. `dup2` does that when the numbers differ.
/// When they already match it does nothing, so `fd` would stay close-on-exec:
/// the flag is cleared instead.
///
/// Makes only async-signal-safe calls, as `pre_exec` needs.
fn pass_fd_as(fd: RawFd, target: RawFd) -> io::Result<()> {
    // SAFETY: syscalls on fd numbers; a bad one is an error, not undefined.
    let result = unsafe {
        if fd == target {
            match libc::fcntl(fd, libc::F_GETFD) {
                -1 => -1,
                flags => libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC),
            }
        } else {
            libc::dup2(fd, target)
        }
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Reads a ready pipe to its end: what it held, or `None` if it didn't end
/// within `limit`.
///
/// A core closes the pipe right after its ready byte, and one that fails
/// before that closes it by exiting, so the end comes with either. Only then
/// is a ready byte known to be all the pipe held: a failed exec's error can
/// arrive here too, and it starts with a byte like any other ([`ready`]).
fn read_ready_pipe(pipe: &OwnedFd, limit: Duration) -> Option<ready::Signal> {
    let deadline = Instant::now() + limit;
    let mut signal = ready::Signal::default();
    let mut buffer = [0u8; 16];
    loop {
        if !wait_readable(pipe, deadline) {
            return None;
        }
        // SAFETY: reads into a buffer of that length.
        let read =
            unsafe { libc::read(pipe.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        match read {
            0 => return Some(signal),
            1.. => signal.push(&buffer[..read as usize]),
            _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => {}
            _ => return None,
        }
    }
}

/// Whether `fd` has data (or EOF) by `deadline`.
fn wait_readable(fd: &OwnedFd, deadline: Instant) -> bool {
    loop {
        let mut pollfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // Rounded up, so the wait doesn't end before the deadline.
        let left = deadline.saturating_duration_since(Instant::now());
        let wait = left.as_nanos().div_ceil(1_000_000) as libc::c_int;
        // SAFETY: one valid pollfd.
        match unsafe { libc::poll(&mut pollfd, 1, wait) } {
            1 => return true,
            0 => return false,
            _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => {}
            _ => return false,
        }
    }
}

/// The running binary of the process `pid`: `/proc/<pid>/exe` still reaches
/// the file after an install replaced it.
pub fn image_of(pid: u32) -> io::Result<PathBuf> {
    Ok(PathBuf::from(format!("/proc/{pid}/exe")))
}

/// Ends the process `pid`, a helper command this process started.
pub fn kill(pid: u32) {
    // SAFETY: a plain kill of a child of this process.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
}

/// A stand-in for the core in tests: listens where the core of `dir` would.
#[cfg(test)]
pub struct Listener(std::os::unix::net::UnixListener);

#[cfg(test)]
impl Listener {
    pub fn bind(dir: &Path) -> Self {
        Self(std::os::unix::net::UnixListener::bind(dir.join(server_dir::CORE_SOCKET)).unwrap())
    }

    /// The next client's connection.
    pub fn accept(&self) -> Stream {
        self.0.accept().unwrap().0
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::process::Command;

    use super::*;

    /// Long enough for a loaded machine; a pipe's end comes much sooner.
    const LIMIT: Duration = Duration::from_secs(30);
    /// What `Command` writes for an exec that failed: the errno (`ENOENT`),
    /// big-endian, then "NOEX".
    const EXEC_ERROR: [u8; 8] = [0, 0, 0, 2, b'N', b'O', b'E', b'X'];

    /// A pipe as `start` makes one: its read end and its write end.
    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    fn write(pipe: &OwnedFd, bytes: &[u8]) {
        let written = unsafe { libc::write(pipe.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize);
    }

    #[test]
    fn only_the_one_ready_byte_at_the_pipes_end_is_ready() {
        let cases: [(&[u8], bool); 5] = [
            (&[ready::BYTE], true),
            // A core that failed before it listened.
            (b"", false),
            (&EXEC_ERROR, false),
            (b"RR", false),
            (b"x", false),
        ];
        for (bytes, ready) in cases {
            let (read, core) = pipe();
            write(&core, bytes);
            drop(core);
            let signal = read_ready_pipe(&read, LIMIT).expect("the pipe ended");
            assert_eq!((signal.is_ready(), signal.len()), (ready, bytes.len()), "{bytes:?}");
        }
    }

    #[test]
    fn a_ready_byte_isnt_ready_until_the_pipe_ends() {
        // More may follow it, so within the limit nothing is known.
        let (read, core) = pipe();
        write(&core, &[ready::BYTE]);
        let begin = Instant::now();
        let limit = Duration::from_millis(100);
        assert_eq!(read_ready_pipe(&read, limit), None);
        assert!(begin.elapsed() >= limit);

        // The end may come after the byte.
        let (read, core) = pipe();
        write(&core, &[ready::BYTE]);
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(core);
        });
        let signal = read_ready_pipe(&read, LIMIT).expect("the pipe ended");
        assert!(signal.is_ready());
        closer.join().unwrap();
    }

    /// For a forked child, before its exec and once the ready pipe is its
    /// fd 3: puts that pipe in the place of every close-on-exec fd the child
    /// has. One of those is the pipe through which `Command` reports a failed
    /// exec, so the error goes down the ready pipe instead, as it does when
    /// that pipe's write end happens to be fd 3 and passing the ready pipe
    /// replaces it. For a program that can't be run: one that runs would
    /// keep the fds.
    ///
    /// Makes only async-signal-safe calls, as `pre_exec` needs.
    fn send_exec_error_down_fd_3() -> io::Result<()> {
        // Far past any fd of a test.
        const FDS: RawFd = 4096;
        for fd in 4..FDS {
            // SAFETY: syscalls on fd numbers; a closed one is an error.
            let replaced = unsafe {
                match libc::fcntl(fd, libc::F_GETFD) {
                    flags if flags != -1 && flags & libc::FD_CLOEXEC != 0 => libc::dup2(3, fd),
                    _ => 0,
                }
            };
            if replaced == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[test]
    fn a_failed_execs_error_in_the_ready_pipe_is_not_ready() {
        let (read, write) = pipe();
        let write_fd = write.as_raw_fd();
        let mut command = Command::new("/nonexistent/brindle-remote");
        command.args(["serve", "--ready-fd", "3"]).stdin(Stdio::null());
        unsafe {
            command.pre_exec(move || pass_fd_as(write_fd, 3));
            command.pre_exec(send_exec_error_down_fd_3);
        }
        // `Command` never hears of the failure, so a start goes on to read
        // the pipe, where the error's first byte once read as "ready".
        let mut child =
            command.spawn().expect("the exec error went to `Command`, not down the pipe");
        drop(write);
        let signal = read_ready_pipe(&read, LIMIT).expect("the pipe ended");
        assert_eq!((signal.is_ready(), signal.len()), (false, EXEC_ERROR.len()));
        assert!(!child.wait().unwrap().success());
    }

    /// Runs [`start`] with a shell script standing in for the helper: it is
    /// run as `<script> serve --ready-fd 3`.
    fn start_script(tag: &str, script: &str) -> Start {
        let dir =
            std::env::temp_dir().join(format!("brindle-local-start-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("brindle-remote");
        crate::test_support::write_executable(&helper, &format!("#!/bin/sh\n{script}\n"));
        let started = start(&Endpoint::at(helper, dir.join("server")), LIMIT);
        std::fs::remove_dir_all(&dir).ok();
        started.unwrap()
    }

    #[test]
    fn a_start_is_ready_only_on_the_ready_byte_and_the_pipes_end() {
        let started = start_script("ready", "printf R >&3");
        assert_eq!(started, Start::Ready);

        // The bytes a failed exec leaves in the ready pipe when the pipe
        // took the place of `Command`'s own for the error.
        let started = start_script("exec-error", r"printf '\000\000\000\002NOEX' >&3; exit 1");
        let Start::Exited { status, .. } = started else { panic!("{started:?}") };
        assert_eq!(status, "exit status: 1");

        // A core that says why it can't start, and writes nothing.
        let started = start_script("refused", "echo 'serve: the directory is refused' >&2; exit 1");
        let Start::Exited { stderr, .. } = started else { panic!("{started:?}") };
        assert!(stderr.contains("the directory is refused"), "{stderr}");
    }

    /// Runs `printf R >&3` with a new pipe's write end passed as fd 3, and
    /// checks that the byte arrives. The pipe is first put on fd 3 itself
    /// (`on_target`), close-on-exec as `pipe2` can return it, or kept off
    /// it. That is done in the forked child, whose fds are its own copy, so
    /// no other test's fd 3 is touched.
    fn assert_passed_as_fd_3(on_target: bool) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let write_fd = write.as_raw_fd();
        let mut command = Command::new("sh");
        command.args(["-c", "printf R >&3"]).stdin(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                let fd = match (on_target, write_fd == 3) {
                    (true, false) => libc::dup3(write_fd, 3, libc::O_CLOEXEC),
                    (false, true) => libc::fcntl(write_fd, libc::F_DUPFD_CLOEXEC, 4),
                    _ => write_fd,
                };
                if fd == -1 {
                    return Err(io::Error::last_os_error());
                }
                pass_fd_as(fd, 3)
            });
        }
        let output = command.output().unwrap();
        drop(write);
        let mut got = String::new();
        std::fs::File::from(read).read_to_string(&mut got).unwrap();
        assert_eq!(got, "R", "on_target {on_target}: {}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn a_pipe_is_passed_as_fd_3_whether_or_not_it_is_there_already() {
        // `dup2(3, 3)` does nothing, so the child's exec would close it.
        assert_passed_as_fd_3(true);
        assert_passed_as_fd_3(false);
    }
}
