//! What the client needs from Linux: the core's Unix socket, the starter
//! (`serve --ready-fd 3`, in a systemd scope of its own where there is one),
//! and a process's image and end.

use std::io::{self, Read as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

use super::{Endpoint, Start};
use crate::remote::protocol::{scope, server_dir};

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

/// Runs `<helper> serve --ready-fd 3` and waits up to `limit` for its ready
/// byte. The core detaches itself (double fork, `setsid`), so it is never
/// Brindle's child and outlives it.
pub fn start(endpoint: &Endpoint, limit: Duration) -> Result<Start> {
    let mut fds = [-1; 2];
    // SAFETY: `pipe2` fills both fds on success; they are owned below.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error()).context("starting the terminal server: pipe");
    }
    // SAFETY: freshly created and not owned elsewhere.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let decision = scope::current();
    if let scope::Decision::Plain(reason) = &decision {
        log::info!("starting the terminal server without a scope of its own: {reason}");
    }
    let args = ["serve".into(), "--ready-fd".into(), "3".into()];
    let mut command = scope::command(&decision, "serve", endpoint.helper.as_os_str(), &args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(write_fd, 3) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().with_context(|| {
        format!("terminal server didn't start: can't run {}", endpoint.helper.display())
    })?;
    drop(write);
    let signalled = wait_readable(&read, limit);
    let mut byte = [0u8; 1];
    let ready = signalled && std::fs::File::from(read).read(&mut byte).ok() == Some(1);
    // The starter exits once the core has detached (or failed first).
    let output = child.wait_with_output().context("waiting for the terminal server to start")?;
    Ok(Start {
        ready,
        signalled,
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Whether `fd` has data (or EOF) within `limit`.
fn wait_readable(fd: &OwnedFd, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let mut pollfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let wait = deadline.saturating_duration_since(Instant::now()).as_millis() as libc::c_int;
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
