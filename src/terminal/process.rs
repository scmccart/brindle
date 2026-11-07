//! Process info for tab titles and new-tab directories: which process is in
//! the foreground on a tab's PTY, its name, and its working directory.
//!
//! Linux reads `/proc`. Windows has no foreground process group, so a
//! tab's process name is its program's most recently started descendant,
//! found in a [`Snapshot`] of all processes that one title tick shares
//! between tabs. Neither platform reads another process's directory there.

use std::path::PathBuf;

/// A process in a [`Snapshot`].
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent: u32,
    pub exe: String,
}

/// One look at the running processes, shared by every tab in a title tick.
/// Empty except on Windows, where it is the only way to name a tab's
/// foreground program.
#[derive(Default)]
pub struct Snapshot {
    #[cfg_attr(not(windows), allow(dead_code))]
    rows: Vec<ProcessRow>,
}

impl Snapshot {
    #[cfg(windows)]
    pub fn take() -> Self {
        Self { rows: windows_rows() }
    }

    #[cfg(not(windows))]
    pub fn take() -> Self {
        Self::default()
    }
}

/// The name for a tab whose program is `root`: its most recently started
/// descendant, or `root` itself when it has none, without `.exe`. At each
/// level the child with the highest pid wins, as pids roughly follow start
/// order. Console hosts are skipped. `None` when `root` isn't in `rows`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn newest_descendant(rows: &[ProcessRow], root: u32) -> Option<String> {
    let is_host = |row: &ProcessRow| {
        row.exe.eq_ignore_ascii_case("conhost.exe") || row.exe.eq_ignore_ascii_case("OpenConsole.exe")
    };
    let mut current = rows.iter().find(|row| row.pid == root)?;
    // A pid reused by a descendant could make a cycle; the depth bounds it.
    for _ in 0..rows.len() {
        let newest = rows
            .iter()
            .filter(|row| row.parent == current.pid && row.pid != current.pid && !is_host(row))
            .max_by_key(|row| row.pid);
        match newest {
            Some(child) => current = child,
            None => break,
        }
    }
    let exe = current.exe.as_str();
    let name = match exe.len().checked_sub(4) {
        Some(stem) if exe.is_char_boundary(stem) && exe[stem..].eq_ignore_ascii_case(".exe") => &exe[..stem],
        _ => exe,
    };
    Some(name.to_string())
}

#[cfg(windows)]
fn windows_rows() -> Vec<ProcessRow> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut rows = Vec::new();
    // SAFETY: a process snapshot, walked with a correctly sized entry and
    // closed afterwards.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return rows };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            rows.push(ProcessRow {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        CloseHandle(snapshot).ok();
    }
    rows
}

/// What a local tab knows about the program on its PTY.
#[derive(Clone, Copy, Debug)]
pub struct PtyProcess {
    child_pid: u32,
}

impl PtyProcess {
    pub fn new(child_pid: u32) -> Self {
        Self { child_pid }
    }

    /// The PTY's foreground process group, which is its leader's pid. The
    /// PTY belongs to the terminal server, so it is read from `/proc`, as
    /// the controlling terminal of the tab's program.
    #[cfg(target_os = "linux")]
    fn foreground_pid(&self) -> Option<u32> {
        terminal_foreground(self.child_pid)
    }

    #[cfg(not(target_os = "linux"))]
    fn foreground_pid(&self) -> Option<u32> {
        None
    }

    /// Name of the foreground process on the PTY (e.g. `vim`).
    #[cfg(not(windows))]
    pub fn foreground_name(&self, _: &Snapshot) -> Option<String> {
        process_name(self.foreground_pid()?)
    }

    /// The newest descendant of the tab's program in `snapshot`.
    #[cfg(windows)]
    pub fn foreground_name(&self, snapshot: &Snapshot) -> Option<String> {
        newest_descendant(&snapshot.rows, self.child_pid)
    }

    /// Current working directory of the foreground process (falls back to
    /// the program the tab started).
    #[cfg_attr(windows, allow(dead_code))]
    pub fn working_directory(&self) -> Option<PathBuf> {
        let pid = self.foreground_pid().unwrap_or(self.child_pid);
        process_cwd(pid).or_else(|| process_cwd(self.child_pid))
    }
}

/// The foreground process group of `pid`'s controlling terminal: the
/// `tpgid` field of `/proc/<pid>/stat`. `None` without a terminal.
#[cfg(target_os = "linux")]
pub fn terminal_foreground(pid: u32) -> Option<u32> {
    parse_tpgid(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// `tpgid` from a `/proc/<pid>/stat` line. The command name is in
/// parentheses and may hold spaces and parentheses itself, so the fields
/// are counted from the last `)`: state, ppid, pgrp, session, tty_nr, tpgid.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_tpgid(stat: &str) -> Option<u32> {
    let fields = &stat[stat.rfind(')')? + 1..];
    let tpgid: i64 = fields.split_whitespace().nth(5)?.parse().ok()?;
    u32::try_from(tpgid).ok().filter(|&pgid| pgid > 0)
}

#[cfg(target_os = "linux")]
pub fn process_name(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim().to_string())
}

#[cfg(target_os = "linux")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(target_os = "linux"))]
#[cfg_attr(windows, allow(dead_code))]
pub fn process_name(_pid: u32) -> Option<String> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    fn rows(list: &[(u32, u32, &str)]) -> Vec<ProcessRow> {
        list.iter().map(|&(pid, parent, exe)| ProcessRow { pid, parent, exe: exe.into() }).collect()
    }

    #[test]
    fn no_children_names_the_program() {
        let rows = rows(&[(4, 1, "System"), (100, 50, "cmd.exe")]);
        assert_eq!(newest_descendant(&rows, 100).as_deref(), Some("cmd"));
    }

    #[test]
    fn follows_a_chain_down() {
        let rows = rows(&[(100, 50, "pwsh.exe"), (200, 100, "cmd.exe"), (300, 200, "python.exe")]);
        assert_eq!(newest_descendant(&rows, 100).as_deref(), Some("python"));
    }

    #[test]
    fn the_highest_pid_wins_among_siblings() {
        let rows = rows(&[(100, 50, "cmd.exe"), (300, 100, "ping.exe"), (250, 100, "timeout.exe"), (900, 250, "x.exe")]);
        assert_eq!(newest_descendant(&rows, 100).as_deref(), Some("ping"));
    }

    #[test]
    fn exe_is_stripped_whatever_its_case() {
        let rows = rows(&[(1, 0, "NOTEPAD.EXE"), (2, 0, "make"), (3, 0, ".exe")]);
        assert_eq!(newest_descendant(&rows, 1).as_deref(), Some("NOTEPAD"));
        assert_eq!(newest_descendant(&rows, 2).as_deref(), Some("make"));
        assert_eq!(newest_descendant(&rows, 3).as_deref(), Some(""));
    }

    #[test]
    fn console_hosts_are_skipped() {
        let rows = rows(&[(100, 50, "cmd.exe"), (400, 100, "conhost.exe"), (500, 100, "OpenConsole.exe")]);
        assert_eq!(newest_descendant(&rows, 100).as_deref(), Some("cmd"));
    }

    #[test]
    fn unknown_root_and_cycles() {
        assert_eq!(newest_descendant(&rows(&[(1, 0, "a.exe")]), 7), None);
        // 10 -> 20 -> 10 (a reused pid): stops instead of looping.
        assert!(newest_descendant(&rows(&[(10, 20, "a.exe"), (20, 10, "b.exe")]), 10).is_some());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn reads_this_process() {
        let pid = std::process::id();
        assert!(process_name(pid).is_some_and(|name| !name.is_empty()));
        assert_eq!(process_cwd(pid), std::env::current_dir().ok());
    }

    #[test]
    fn tpgid_is_counted_from_the_last_parenthesis() {
        assert_eq!(parse_tpgid("42 (bash) S 1 42 42 34816 4321 4194560 0 0"), Some(4321));
        assert_eq!(parse_tpgid("42 (a) b (c)) S 1 42 42 34816 77 0"), Some(77));
        assert_eq!(parse_tpgid("42 (sleep) S 1 42 42 0 -1 0"), None);
        assert_eq!(parse_tpgid("42 (sleep) S 1"), None);
        assert_eq!(parse_tpgid(""), None);
    }

    /// A child on a PTY of its own, as a holder starts a session's program:
    /// a shell with job control running `sleep` in the foreground, in /tmp.
    #[test]
    fn the_foreground_job_of_a_spawned_child() {
        use std::os::fd::{FromRawFd as _, OwnedFd};
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let (mut master, mut slave) = (-1, -1);
        // SAFETY: openpty fills both fds on success; they are owned below.
        let opened = unsafe {
            libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null())
        };
        assert_eq!(opened, 0);
        // SAFETY: freshly opened, owned here.
        let (_master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let mut command = Command::new("sh");
        command
            .args(["-c", "set -m; sleep 30; :"])
            .current_dir("/tmp")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        let process = PtyProcess::new(child.id());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut name = None;
        while Instant::now() < deadline {
            name = process.foreground_name(&Snapshot::default());
            if name.as_deref() == Some("sleep") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let foreground = terminal_foreground(child.id());
        let cwd = process.working_directory();
        child.kill().ok();
        child.wait().ok();
        if let Some(sleep) = foreground {
            unsafe { libc::kill(sleep as i32, libc::SIGKILL) };
        }
        assert_eq!(name.as_deref(), Some("sleep"));
        assert_ne!(foreground, Some(child.id()), "the job has its own group");
        assert_eq!(cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    }
}
