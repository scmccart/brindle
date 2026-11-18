//! The Linux PTY: opening it, spawning the program with its environment and
//! working directory, resizing, signalling and the `/proc` queries.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::program::{
    default_term, expand_home, home_from_env, passwd_shell, shell_from_env, start_dir,
};
use super::stat::{STAT_PGRP, STAT_TPGID, parse_stat_field};
use super::{Foreground, PtyCommand};

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
    if let Some(term) = default_term(request.exact_env, request.env) {
        command.env("TERM", term);
    }
    command
        .stdin(Stdio::from(File::from(slave_stdin)))
        .stdout(Stdio::from(File::from(slave_stdout)))
        .stderr(Stdio::from(File::from(slave)));

    // Asked here, since the child may make only async-signal-safe calls.
    let last_signal = libc::SIGRTMAX();
    // The child starts with default signal handling, whatever this process
    // has. It must also become a session leader and acquire the slave as
    // its controlling terminal for shell job control and tcgetpgrp to work.
    unsafe {
        command.pre_exec(move || {
            default_signals(last_signal)?;
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

/// For a forked child, before its exec: gives every signal from 1 to
/// `last` its default action and unblocks them all, as
/// `POSIX_SPAWN_SETSIGDEF` and `POSIX_SPAWN_SETSIGMASK` would.
///
/// An exec keeps ignored signals ignored and blocked ones blocked. So the
/// program would otherwise get whatever this process was started with: under
/// `nohup`, a `SIGHUP` it never sees, and a hang-up that only the `SIGKILL`
/// after it ends.
///
/// Makes only async-signal-safe calls, as `pre_exec` needs.
fn default_signals(last: libc::c_int) -> io::Result<()> {
    for signal in 1..=last {
        if signal == libc::SIGKILL || signal == libc::SIGSTOP {
            continue;
        }
        // SAFETY: the default action runs no code of this process. The C
        // library refuses the few signals it keeps for itself, and that
        // failure is ignored.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
    let mut none = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: `sigemptyset` fills the set, and only then is it read.
    unsafe {
        if libc::sigemptyset(none.as_mut_ptr()) == -1
            || libc::sigprocmask(libc::SIG_SETMASK, none.as_ptr(), std::ptr::null_mut()) == -1
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
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
        let home = home_dir(request.env);
        let wanted = expand_home(cwd, home.as_deref());
        if let Some(dir) = start_dir(&wanted, home.as_deref(), |path| path.is_dir()) {
            command.current_dir(dir);
        }
    }
    command
}

/// The home directory: the environment's ([`home_from_env`]), else the
/// passwd entry's, which is read only then.
pub fn home_dir(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    home_from_env(
        env.get("HOME").map(String::as_str),
        std::env::var_os("HOME").as_deref(),
    )
    .or_else(|| passwd_entry().map(|(_, home)| home))
}

/// The login shell and home directory of the current user from the passwd
/// database. `docker exec` sets no `SHELL`.
pub fn passwd_entry() -> Option<(OsString, PathBuf)> {
    with_passwd(|entry| {
        let shell = passwd_field(entry.pw_shell)?;
        let dir = passwd_field(entry.pw_dir)?;
        Some((shell, PathBuf::from(dir)))
    })
}

/// The current user's name from the passwd database.
pub fn user_name() -> Option<OsString> {
    with_passwd(|entry| passwd_field(entry.pw_name)).filter(|name| !name.is_empty())
}

/// Calls `read` with the current user's passwd entry, whose strings live
/// only for the call.
fn with_passwd<T>(read: impl FnOnce(&libc::passwd) -> Option<T>) -> Option<T> {
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
    // SAFETY: filled in, since `found` points at it.
    let entry = unsafe { entry.assume_init() };
    read(&entry)
}

/// One string of a passwd entry; `None` for a null pointer.
fn passwd_field(field: *const libc::c_char) -> Option<OsString> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    if field.is_null() {
        return None;
    }
    // SAFETY: a passwd entry's strings are NUL-terminated, and the buffer
    // they point into outlives this call (`with_passwd`).
    let bytes = unsafe { CStr::from_ptr(field) }.to_bytes().to_vec();
    Some(OsString::from_vec(bytes))
}

/// The login shell: the environment's ([`shell_from_env`]), else the passwd
/// entry's, which is read only then ([`passwd_shell`]).
fn default_shell(env: &BTreeMap<String, String>) -> OsString {
    shell_from_env(
        env.get("SHELL").map(String::as_str),
        std::env::var_os("SHELL").as_deref(),
    )
    .unwrap_or_else(|| {
        passwd_shell(passwd_entry().map(|(shell, _)| shell), |path| {
            path.exists()
        })
    })
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

/// The foreground process of the terminal that `pid` (a session's
/// program) controls, else `pid` itself: its name and working directory.
/// For the terminal server's core, which knows a session's program but not
/// its PTY, so it reads the terminal's foreground group (`tpgid`) from
/// `/proc/<pid>/stat` instead of asking the PTY.
pub fn foreground_process(pid: libc::pid_t) -> Foreground {
    let foreground = stat_field(pid, STAT_TPGID)
        .filter(|group| *group > 0)
        .and_then(find_process_in_group)
        .unwrap_or(pid);
    describe(foreground, pid)
}

/// The name and working directory of `pid`; the directory of `fallback`
/// (the session's program) when `pid`'s can't be read.
fn describe(pid: libc::pid_t, fallback: libc::pid_t) -> Foreground {
    let proc_dir = format!("/proc/{pid}");
    let name = fs::read_to_string(format!("{proc_dir}/comm"))
        .ok()
        .map(|name| {
            name.trim_end_matches(['\r', '\n'])
                .to_owned()
        })
        .filter(|name| !name.is_empty());
    let cwd = fs::read_link(format!("{proc_dir}/cwd"))
        .or_else(|_| fs::read_link(format!("/proc/{fallback}/cwd")))
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    Foreground { name, cwd }
}

/// A numeric field of `/proc/<pid>/stat` ([`parse_stat_field`]).
fn stat_field(pid: libc::pid_t, field: usize) -> Option<libc::pid_t> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat_field(&stat, field)
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
    stat_field(pid, STAT_PGRP)
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
    fn a_process_without_a_terminal_is_its_own_foreground() {
        let pid = std::process::id() as libc::pid_t;
        let own = foreground_process(pid);
        // On a terminal or not, the answer is a process that runs.
        assert!(own.name.is_some_and(|name| !name.is_empty()));
        assert!(own.cwd.is_some());
        // Without one, it is this process.
        if stat_field(pid, STAT_TPGID).is_none_or(|group| group <= 0) {
            let cwd = std::env::current_dir().unwrap();
            assert_eq!(foreground_process(pid).cwd.map(PathBuf::from), Some(cwd));
        }
    }
}
