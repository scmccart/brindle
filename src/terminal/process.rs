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
    /// The PTY master, for `tcgetpgrp`. It stays open for as long as the
    /// event loop owns the PTY.
    #[cfg(unix)]
    master_fd: i32,
}

impl PtyProcess {
    #[cfg(unix)]
    pub fn new(child_pid: u32, master_fd: i32) -> Self {
        Self { child_pid, master_fd }
    }

    #[cfg(not(unix))]
    pub fn new(child_pid: u32) -> Self {
        Self { child_pid }
    }

    /// The PTY's foreground process group, which is its leader's pid.
    #[cfg(unix)]
    fn foreground_pid(&self) -> Option<u32> {
        // SAFETY: tcgetpgrp has no memory-safety preconditions; a stale fd
        // only makes it fail.
        let pgrp = unsafe { libc::tcgetpgrp(self.master_fd) };
        (pgrp > 0).then_some(pgrp as u32)
    }

    #[cfg(not(unix))]
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
    pub fn working_directory(&self) -> Option<PathBuf> {
        let pid = self.foreground_pid().unwrap_or(self.child_pid);
        process_cwd(pid).or_else(|| process_cwd(self.child_pid))
    }
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
}
