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
use std::time::{Duration, Instant};

use brindle_remote_protocol::ready;

use super::TIMEOUT;

/// A connection to a server's endpoint.
pub type Stream = UnixStream;

/// Whether everything a refused peer sent reaches the server: a socket
/// half-closes, so the server reads it to the end before closing.
pub const REFUSED_WRITES_ARRIVE: bool = true;

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

/// Runs `bin <role> --ready-fd 3` for `dir` and waits for it to be ready:
/// for its ready pipe to end, having held the ready byte and nothing else
/// ([`ready`]). Fails, with the server's stderr, if it exits before being
/// ready.
pub fn start(bin: &Path, role: &str, dir: &Path, env: &[(&str, &str)]) -> io::Result<()> {
    start_with(bin, role, dir, env, || Ok(()))
}

/// [`start`], with `in_child` run in the forked child before its exec, once
/// the ready pipe is its fd 3. What it sets up there, such as an ignored
/// signal, the server inherits. It may make only async-signal-safe calls.
pub fn start_with(
    bin: &Path,
    role: &str,
    dir: &Path,
    env: &[(&str, &str)],
    in_child: impl FnMut() -> io::Result<()> + Send + Sync + 'static,
) -> io::Result<()> {
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let mut command = Command::new(bin);
    command
        .args([role, "--ready-fd", "3"])
        .env("BRINDLE_SERVER_DIR", dir)
        .env("BRINDLE_CACHE_DIR", dir.join("cache"))
        .env("BRINDLE_NO_SCOPE", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || pass_fd_as(write_fd, 3));
        command.pre_exec(in_child);
    }
    let child = super::retry_busy_exec(|| command.spawn())?;
    drop(write);
    let signal = read_ready_pipe(&read);
    let output = child.wait_with_output()?;
    if !signal.is_some_and(|signal| signal.is_ready()) {
        let pipe = match signal {
            Some(signal) => format!("its ready pipe held {} bytes", signal.len()),
            None => "its ready pipe didn't end".to_string(),
        };
        return Err(io::Error::other(format!(
            "{role} wasn't ready ({}; {pipe}): {}",
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

/// Reads a ready pipe to its end: what it held, or `None` if it didn't end
/// within [`TIMEOUT`]. A server closes it right after its ready byte, and
/// one that fails before that closes it by exiting.
fn read_ready_pipe(pipe: &OwnedFd) -> Option<ready::Signal> {
    let deadline = Instant::now() + TIMEOUT;
    let mut signal = ready::Signal::default();
    let mut buffer = [0u8; 16];
    loop {
        let mut pollfd = libc::pollfd {
            fd: pipe.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let left = deadline.saturating_duration_since(Instant::now());
        // Rounded up, so the wait doesn't end before the deadline.
        let wait = left.as_nanos().div_ceil(1_000_000) as libc::c_int;
        match unsafe { libc::poll(&mut pollfd, 1, wait) } {
            1 => {}
            0 => return None,
            _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            _ => return None,
        }
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

/// For a forked child, before its exec and once the ready pipe is its fd 3:
/// puts that pipe in the place of every close-on-exec fd the child has.
/// One of those is the pipe through which Rust's `Command` reports a failed
/// exec, so the error goes down the ready pipe instead, as it does when that
/// pipe's write end happens to be fd 3 and passing the ready pipe replaces
/// it. For a program that can't be run: one that runs would keep the fds.
///
/// Makes only async-signal-safe calls, as `pre_exec` needs.
pub fn send_exec_error_down_fd_3() -> io::Result<()> {
    // Far past any fd of a test.
    const FDS: libc::c_int = 4096;
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

/// The name of the program `pid` runs, `None` once the pid is gone. A
/// forked child has its parent's until its exec.
pub fn program_of(pid: u32) -> Option<String> {
    let name = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(name.trim_end().to_string())
}

/// One of the signal sets in `/proc/<pid>/status` (`SigIgn` for the ignored
/// signals, `SigBlk` for the blocked ones), where bit `n - 1` stands for
/// signal `n`. `None` once the pid is gone.
pub fn signal_set(pid: u32, name: &str) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        let set = line.strip_prefix(name)?.strip_prefix(':')?;
        u64::from_str_radix(set.trim(), 16).ok()
    })
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
pub const BLOCKED_FOR: Duration = Duration::from_millis(250);

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
    let mut quiet = super::Quiet::new(BLOCKED_FOR);
    let mut ended = false;
    let blocked = super::eventually(TIMEOUT, || {
        if !alive(writer) {
            ended = true;
            return true;
        }
        quiet.observe(context_switches(writer).filter(|_| asleep(writer) && asleep(reader)))
    });
    blocked && !ended
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
