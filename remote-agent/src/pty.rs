//! PTY sessions shared by the stdio agent (`--server`) and the terminal
//! server's holder: opening the PTY, spawning the program with its
//! environment and working directory, resizing, signalling, `/proc` queries
//! and the shutdown timings.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// How much PTY output is read at once.
pub const OUTPUT_CHUNK_SIZE: usize = 32 * 1024;
/// How long a hung-up session's process group has to exit after `SIGHUP`
/// before it gets `SIGKILL`.
pub const GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(1);
/// How long to wait for a session to go after `SIGKILL`.
pub const FORCED_SHUTDOWN: Duration = Duration::from_secs(1);
/// After the program exits, output is still read until it has been quiet
/// for this long...
pub const OUTPUT_DRAIN_IDLE: Duration = Duration::from_millis(250);
/// ...or for at most this long.
pub const OUTPUT_DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// What to run on a new PTY.
pub struct PtyCommand<'a> {
    /// `None` runs the user's login shell.
    pub command: Option<&'a str>,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    /// `env` is the program's whole environment. Otherwise it is added to
    /// this process's own, with a default `TERM`.
    pub exact_env: bool,
}

pub struct SpawnedPty {
    pub pid: libc::pid_t,
    pub child: Child,
    /// The PTY master, non-blocking: session input goes in, output and the
    /// process queries come from it.
    pub master: OwnedFd,
}

impl PtyCommand<'_> {
    /// The program that runs: the command, or the login shell.
    pub fn program(&self) -> OsString {
        self.command
            .map(OsString::from)
            .unwrap_or_else(|| default_shell(self.env))
    }
}

pub fn spawn(request: &PtyCommand) -> io::Result<SpawnedPty> {
    let winsize = make_winsize(
        request.cols,
        request.rows,
        request.cell_width,
        request.cell_height,
    );
    let (master, slave) = open_pty(&winsize)?;
    set_nonblocking(master.as_raw_fd())?;

    let slave_stdin = duplicate_fd(slave.as_raw_fd())?;
    let slave_stdout = duplicate_fd(slave.as_raw_fd())?;
    let program = request.program();
    let mut command = command_for(request, program.clone());
    if request.command.is_none() {
        // The default session is a login shell, as a terminal emulator or
        // sshd would start it.
        if let Some(name) = Path::new(&program).file_name() {
            let mut login = OsString::from("-");
            login.push(name);
            command.arg0(login);
        }
    }
    if !request.exact_env && !request.env.contains_key("TERM") {
        command.env("TERM", "xterm-256color");
    }
    command
        .stdin(Stdio::from(File::from(slave_stdin)))
        .stdout(Stdio::from(File::from(slave_stdout)))
        .stderr(Stdio::from(File::from(slave)));

    // The child must become a session leader and acquire the slave as its
    // controlling terminal for shell job control and tcgetpgrp to work.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = command.spawn()?;
    let pid = child.id() as libc::pid_t;
    Ok(SpawnedPty { pid, child, master })
}

fn command_for(request: &PtyCommand, program: OsString) -> Command {
    let mut command = Command::new(program);
    if request.exact_env {
        command.env_clear();
    }
    command.args(request.args).envs(request.env);
    if let Some(cwd) = request.cwd {
        // A directory that does not exist here (a path inherited from a
        // different host, or since deleted) starts the session in $HOME.
        let wanted = expand_remote_cwd(cwd, request.env);
        if wanted.is_dir() {
            command.current_dir(wanted);
        } else if let Some(home) = home_dir(request.env).filter(|home| home.is_dir()) {
            command.current_dir(home);
        }
    }
    command
}

pub fn home_dir(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    env.get("HOME")
        .filter(|home| !home.is_empty())
        .map(OsStr::new)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| passwd_entry().map(|(_, home)| home))
}

/// The login shell and home directory of the current user from the passwd
/// database. `docker exec` sets no `SHELL`.
pub fn passwd_entry() -> Option<(OsString, PathBuf)> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    let mut buffer = vec![0u8; 4096];
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::zeroed();
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    let entry = unsafe { entry.assume_init() };
    if entry.pw_shell.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    let shell = unsafe { CStr::from_ptr(entry.pw_shell) }
        .to_bytes()
        .to_vec();
    let dir = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes().to_vec();
    Some((
        OsString::from_vec(shell),
        PathBuf::from(OsString::from_vec(dir)),
    ))
}

pub fn expand_remote_cwd(cwd: &str, env: &BTreeMap<String, String>) -> PathBuf {
    let home = || home_dir(env);
    if cwd == "~" {
        return home().unwrap_or_else(|| PathBuf::from(cwd));
    }
    if let Some(rest) = cwd.strip_prefix("~/")
        && let Some(home) = home()
    {
        return home.join(rest);
    }
    PathBuf::from(cwd)
}

fn default_shell(env: &BTreeMap<String, String>) -> OsString {
    env.get("SHELL")
        .filter(|shell| !shell.is_empty())
        .map(|shell| OsString::from(shell.as_str()))
        .or_else(|| std::env::var_os("SHELL").filter(|shell| !shell.is_empty()))
        .or_else(|| {
            passwd_entry()
                .map(|(shell, _)| shell)
                .filter(|shell| !shell.is_empty() && PathBuf::from(shell).exists())
        })
        .unwrap_or_else(|| OsString::from("/bin/sh"))
}

fn open_pty(winsize: &libc::winsize) -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            winsize,
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        if master >= 0 {
            unsafe { libc::close(master) };
        }
        if slave >= 0 {
            unsafe { libc::close(slave) };
        }
        return Err(error);
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    set_close_on_exec(master.as_raw_fd())?;
    set_close_on_exec(slave.as_raw_fd())?;
    set_utf8(master.as_raw_fd());
    Ok((master, slave))
}

/// Sets `IUTF8`, as alacritty's PTY did, so the line discipline erases a
/// whole multibyte character. Failing to is not fatal.
fn set_utf8(fd: RawFd) {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `tcgetattr` fills the struct on success, and only then is it read.
    unsafe {
        if libc::tcgetattr(fd, termios.as_mut_ptr()) == 0 {
            let mut termios = termios.assume_init();
            termios.c_iflag |= libc::IUTF8;
            libc::tcsetattr(fd, libc::TCSANOW, &termios);
        }
    }
}

pub fn set_close_on_exec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn duplicate_fd(fd: RawFd) -> io::Result<OwnedFd> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

pub fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn make_winsize(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: cols.saturating_mul(cell_width),
        ws_ypixel: rows.saturating_mul(cell_height),
    }
}

pub fn resize(
    fd: RawFd,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
) -> io::Result<()> {
    let winsize = make_winsize(cols, rows, cell_width, cell_height);
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &winsize) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The foreground process of the PTY (else the session's own program), its
/// name and its working directory.
pub fn inspect_process(
    pty_fd: RawFd,
    child_pid: libc::pid_t,
) -> brindle_remote_protocol::ProcessInfo {
    let foreground_group = Some(unsafe { libc::tcgetpgrp(pty_fd) }).filter(|pid| *pid > 0);
    let pid = foreground_group
        .and_then(find_process_in_group)
        .unwrap_or(child_pid);
    let proc_dir = format!("/proc/{pid}");
    let foreground_name = fs::read_to_string(format!("{proc_dir}/comm"))
        .ok()
        .map(|name| {
            name.trim_end_matches(|character| character == '\r' || character == '\n')
                .to_owned()
        })
        .filter(|name| !name.is_empty());
    let cwd = fs::read_link(format!("{proc_dir}/cwd"))
        .or_else(|_| fs::read_link(format!("/proc/{child_pid}/cwd")))
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    brindle_remote_protocol::ProcessInfo {
        pid: pid.max(0) as u32,
        foreground_name,
        cwd,
    }
}

fn find_process_in_group(group: libc::pid_t) -> Option<libc::pid_t> {
    if process_group(group) == Some(group) {
        return Some(group);
    }
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if process_group(pid) == Some(group) {
            return Some(pid);
        }
    }
    None
}

fn process_group(pid: libc::pid_t) -> Option<libc::pid_t> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat.get(stat.rfind(')')? + 1..)?;
    // Fields after the command are: state (3), ppid (4), pgrp (5).
    fields.split_whitespace().nth(2)?.parse().ok()
}

/// Sends `signal` to the process group led by `pid` (a session's program,
/// which `spawn` made a session and group leader).
pub fn signal_process_group(pid: libc::pid_t, signal: libc::c_int) {
    if pid > 0 {
        unsafe {
            libc::kill(-pid, signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_tilde_uses_remote_environment_home() {
        let env = [("HOME".to_string(), "/home/remote".to_string())].into();
        assert_eq!(expand_remote_cwd("~", &env), PathBuf::from("/home/remote"));
        assert_eq!(
            expand_remote_cwd("~/src tree", &env),
            PathBuf::from("/home/remote/src tree")
        );
        assert_eq!(
            expand_remote_cwd("/srv/app", &env),
            PathBuf::from("/srv/app")
        );
    }
}
