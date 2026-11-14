//! The Unix half of the tests' support: sockets, starting a server process
//! with `--ready-fd`, and process checks through `/proc`.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::TIMEOUT;

/// A connection to a server's endpoint.
pub type Stream = UnixStream;

/// Connects to the server listening at `endpoint`.
pub fn connect(endpoint: &Path) -> io::Result<Stream> {
    UnixStream::connect(endpoint)
}

/// Closes the connection for every clone of `stream`, as its process
/// exiting or being killed would.
pub fn close(stream: &Stream) {
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Makes `dir` a directory only this user can enter.
pub fn private_dir(dir: &Path) {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
}

/// The arguments a holder starts a real core with
/// (`BRINDLE_TEST_HOLDER_CORE_ARGS`).
pub const SERVE_ARGS: &str = "serve --ready-fd {ready_fd}";

/// Kills `pid`.
pub fn kill(pid: u32) {
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// Kills the server process `pid`, if it still runs.
pub fn kill_server(pid: u32) {
    if alive(pid) {
        kill(pid);
    }
}

/// The running `brindle-remote <role>` processes (`serve` or `hold`) whose
/// environment names `dir` as their server directory.
pub fn server_processes(dir: &Path, role: &str) -> Vec<u32> {
    let wanted = format!("BRINDLE_SERVER_DIR={}", dir.display());
    let mut pids: Vec<u32> = fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| {
            let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
                return false;
            };
            let is_role = cmdline.split(|&b| b == 0).nth(1) == Some(role.as_bytes());
            is_role
                && alive(pid)
                && fs::read(format!("/proc/{pid}/environ"))
                    .is_ok_and(|env| env.split(|&b| b == 0).any(|var| var == wanted.as_bytes()))
        })
        .collect();
    pids.sort_unstable();
    pids
}

/// For a forked child, before its exec: makes `fd` the child's fd `target`,
/// open in the program it runs. `dup2` does that when the numbers differ.
/// When they already match it does nothing, so `fd` would stay close-on-exec:
/// the flag is cleared instead.
///
/// Makes only async-signal-safe calls, as `pre_exec` needs.
pub fn pass_fd_as(fd: RawFd, target: RawFd) -> io::Result<()> {
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

/// Runs `bin <role> --ready-fd 3` for `dir` and waits for its ready byte.
/// Fails, with the server's stderr, if it exits before being ready.
pub fn start(bin: &Path, role: &str, dir: &Path, env: &[(&str, &str)]) -> io::Result<()> {
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let mut command = Command::new(bin);
    command
        .args([role, "--ready-fd", "3"])
        .env("BRINDLE_SERVER_DIR", dir)
        .env("BRINDLE_NO_SCOPE", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || pass_fd_as(write_fd, 3));
    }
    // A freshly written binary can be briefly busy: another test thread's
    // fork may still hold the writer's fd until it execs.
    let mut attempts = 0;
    let child = loop {
        match command.spawn() {
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && attempts < 100 => {
                attempts += 1;
                thread::sleep(Duration::from_millis(20));
            }
            result => break result?,
        }
    };
    drop(write);
    let mut pollfd = libc::pollfd {
        fd: read.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ready = unsafe { libc::poll(&mut pollfd, 1, TIMEOUT.as_millis() as libc::c_int) };
    let mut byte = [0u8; 1];
    let got =
        ready == 1 && unsafe { libc::read(read.as_raw_fd(), byte.as_mut_ptr().cast(), 1) } == 1;
    let output = child.wait_with_output()?;
    if !got {
        return Err(io::Error::other(format!(
            "{role} wasn't ready ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "the starter exited with {}",
            output.status
        )));
    }
    Ok(())
}

/// The state letter in `/proc/<pid>/stat`, `None` once the pid is gone.
pub fn state(pid: u32) -> Option<char> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat[stat.rfind(')')? + 1..].trim_start().chars().next()
}

/// Running, and not a zombie.
pub fn alive(pid: u32) -> bool {
    state(pid).is_some_and(|state| state != 'Z' && state != 'X')
}

pub fn parent(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat[stat.rfind(')')? + 1..]
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Children of `pid` named `comm`.
pub fn children_named(pid: u32, comm: &str) -> Vec<u32> {
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&child| {
            parent(child) == Some(pid)
                && fs::read_to_string(format!("/proc/{child}/comm"))
                    .is_ok_and(|name| name.trim_end() == comm)
        })
        .collect()
}

/// How often `pid` has been switched off a CPU, `None` once it is gone. The
/// count stands still while a process stays blocked.
pub fn context_switches(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let count = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name)?.trim().parse::<u64>().ok())
    };
    Some(count("voluntary_ctxt_switches:")? + count("nonvoluntary_ctxt_switches:")?)
}

/// How long [`stays_blocked`] watches a writer do nothing.
const BLOCKED_FOR: Duration = Duration::from_millis(250);

/// Waits until `writer` is blocked for good on output that `reader` no
/// longer takes. Returns `false` if `writer` ends instead, or isn't blocked
/// within [`TIMEOUT`].
///
/// A writer faster than its reader sleeps too, but only until the reader
/// takes some more. So it counts as blocked once, for [`BLOCKED_FOR`], it
/// never ran and both were asleep at every look: nobody was about to move
/// the output on. How long the output took to get that far doesn't matter,
/// as it would with a fixed sleep.
pub fn stays_blocked(writer: u32, reader: u32) -> bool {
    let asleep = |pid| state(pid) == Some('S');
    let deadline = Instant::now() + TIMEOUT;
    // Since when it has been quiet, and the writer's switch count then.
    let mut quiet: Option<(Instant, u64)> = None;
    while alive(writer) && Instant::now() < deadline {
        let switches = context_switches(writer).filter(|_| asleep(writer) && asleep(reader));
        quiet = match (switches, quiet) {
            (Some(now), Some((since, before))) if now == before => {
                if since.elapsed() >= BLOCKED_FOR {
                    return true;
                }
                quiet
            }
            (Some(now), _) => Some((Instant::now(), now)),
            (None, _) => None,
        };
        thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Hands `stream` to a `sleep` process (as its fd 3), so killing that
/// process closes the connection as a client dying does. The caller drops
/// its own side.
pub fn hand_to_process(stream: &Stream) -> Child {
    let fd = stream.as_raw_fd();
    let mut command = Command::new("sleep");
    command.arg("1000").stdin(Stdio::null());
    unsafe {
        command.pre_exec(move || pass_fd_as(fd, 3));
    }
    command.spawn().unwrap()
}
