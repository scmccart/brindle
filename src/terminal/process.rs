//! Process info for tab titles, badges and new-tab directories: which
//! process is in the foreground on a tab's PTY, its name, its elevation and
//! its working directory, the path style its directory reports use
//! ([`LOCAL_PATH_STYLE`]), the line end of what is copied from its grid
//! ([`LOCAL_LINE_END`]), where its foreground program comes from
//! ([`LOCAL_FOREGROUND`]) and where its directory comes from
//! ([`LOCAL_DIRECTORY`]).
//!
//! Linux reads `/proc`. Windows has no foreground process group, so a
//! tab's foreground program is its program's most recently started
//! descendant, found in a [`Snapshot`] of all processes that one title tick
//! shares between tabs, off the UI thread ([`detect`]), with that process's
//! [`Elevation`] read from its token. No process's directory can be read
//! there, so a tab's is the one its program reported.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::LineEnd;
use super::context::Elevation;
use super::shell_integration::PathStyle;

/// How a program on this machine spells the directories it reports (OSC 7,
/// 9;9). [`crate::remote::Host::path_style`] picks it for a local session.
#[cfg(windows)]
pub const LOCAL_PATH_STYLE: PathStyle = PathStyle::Windows;

/// How a program on this machine spells the directories it reports (OSC 7,
/// 9;9). [`crate::remote::Host::path_style`] picks it for a local session.
#[cfg(not(windows))]
pub const LOCAL_PATH_STYLE: PathStyle = PathStyle::Posix;

/// The line end of text copied from a grid on this machine.
#[cfg(windows)]
pub const LOCAL_LINE_END: LineEnd = LineEnd::CrLf;

/// The line end of text copied from a grid on this machine.
#[cfg(not(windows))]
pub const LOCAL_LINE_END: LineEnd = LineEnd::Lf;

/// Where the directory of a program on this machine comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalDirectory {
    /// The foreground process's own, read from the system
    /// ([`PtyProcess::working_directory`]).
    #[cfg_attr(windows, allow(dead_code, reason = "Windows can't read another process's directory"))]
    Process,
    /// The one the program last reported (OSC 7, 9;9), if it exists: the
    /// system gives no other. It is a path the program chose, so whether
    /// it exists is checked off the UI thread (`dir_check`).
    #[cfg_attr(not(windows), allow(dead_code, reason = "only Windows has to take the program's word"))]
    Reported,
}

/// Where a local session's directory comes from on this platform.
#[cfg(windows)]
pub const LOCAL_DIRECTORY: LocalDirectory = LocalDirectory::Reported;

/// Where a local session's directory comes from on this platform.
#[cfg(not(windows))]
pub const LOCAL_DIRECTORY: LocalDirectory = LocalDirectory::Process;

/// How a local tab's foreground program is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalForeground {
    /// The PTY's foreground process group, read from the system when a
    /// title asks for it ([`PtyProcess::foreground_name`]).
    #[cfg_attr(windows, allow(dead_code, reason = "Windows has no foreground process group"))]
    Group,
    /// The newest descendant of the tab's program, with its elevation,
    /// found by the title tick off the UI thread ([`detect`]) and kept on
    /// the terminal until the next tick.
    #[cfg_attr(not(windows), allow(dead_code, reason = "only Windows has to look at every process"))]
    Descendant,
}

/// Where a local session's foreground program comes from on this platform.
#[cfg(windows)]
pub const LOCAL_FOREGROUND: LocalForeground = LocalForeground::Descendant;

/// Where a local session's foreground program comes from on this platform.
#[cfg(not(windows))]
pub const LOCAL_FOREGROUND: LocalForeground = LocalForeground::Group;

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
struct Snapshot {
    rows: Vec<ProcessRow>,
}

impl Snapshot {
    #[cfg(windows)]
    fn take() -> Self {
        Self { rows: windows_rows() }
    }

    #[cfg(not(windows))]
    fn take() -> Self {
        Self::default()
    }
}

/// A tab's foreground program as one walk of a [`Snapshot`] found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descendant {
    pub pid: u32,
    /// Its executable's name without `.exe`: what the tab's title shows.
    pub name: String,
}

/// What a title tick found for a local tab's program ([`detect`]): its
/// foreground process's name, for the title, and that same process's
/// elevation, for the badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub name: String,
    pub elevation: Elevation,
}

/// The process for a tab whose program is `root`: its most recently
/// started descendant, or `root` itself when it has none, named without
/// `.exe`. At each level the child with the highest pid wins, as pids
/// roughly follow start order. Console hosts are skipped. `None` when
/// `root` isn't in `rows`.
pub fn newest_descendant(rows: &[ProcessRow], root: u32) -> Option<Descendant> {
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
    Some(Descendant { pid: current.pid, name: name.to_string() })
}

/// The elevations read so far, by pid, so a process's token is read once
/// while it runs: the title tick keeps it between looks.
#[derive(Debug, Default)]
pub struct ElevationCache {
    read: HashMap<u32, Elevation>,
}

impl ElevationCache {
    /// `pid`'s elevation: the one read before, else what `read` reads now.
    /// Only a token actually read is kept; an `Unknown` (a failure) is
    /// read again on the next look.
    fn get(&mut self, pid: u32, read: impl FnOnce(u32) -> Elevation) -> Elevation {
        if let Some(&known) = self.read.get(&pid) {
            return known;
        }
        let elevation = read(pid);
        match elevation {
            Elevation::Standard | Elevation::Elevated => {
                self.read.insert(pid, elevation);
            }
            Elevation::Unknown => {}
        }
        elevation
    }

    /// Forgets every pid not in `rows`: a process that has ended, so that
    /// a later process Windows gives the same pid is read afresh.
    fn prune(&mut self, rows: &[ProcessRow]) {
        let running: HashSet<u32> = rows.iter().map(|row| row.pid).collect();
        self.read.retain(|pid, _| running.contains(pid));
    }
}

/// Each root's foreground program in `rows` ([`newest_descendant`]) with
/// that process's elevation, from `cache` or else from `read` (a probe that
/// reads one token); `None` for a root not in `rows`. The cache is pruned
/// to `rows` first. One entry per root, in order.
fn detect_in(
    rows: &[ProcessRow],
    roots: &[u32],
    cache: &mut ElevationCache,
    mut read: impl FnMut(u32) -> Elevation,
) -> Vec<Option<Detected>> {
    cache.prune(rows);
    roots
        .iter()
        .map(|&root| {
            let Descendant { pid, name } = newest_descendant(rows, root)?;
            let elevation = cache.get(pid, &mut read);
            Some(Detected { name, elevation })
        })
        .collect()
}

/// The foreground program of each local tab whose program is one of
/// `roots` ([`LocalForeground::Descendant`]): one look at all processes,
/// and a token read for each foreground process `cache` doesn't know. It
/// reads the system, so the title tick runs it on the background executor.
pub fn detect(roots: &[u32], cache: &mut ElevationCache) -> Vec<Option<Detected>> {
    detect_in(&Snapshot::take().rows, roots, cache, elevation)
}

/// Whether process `pid` runs elevated, from its token's `TokenElevation`.
/// Only a token actually read decides: when the process or its token can't
/// be opened, a refusal included, or the token can't be read, it is
/// `Unknown`, never taken for elevated.
#[cfg(windows)]
pub fn elevation(pid: u32) -> Elevation {
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: plain arguments; the handle is owned below.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return Elevation::Unknown;
    };
    // SAFETY: a fresh handle nothing else owns; dropping it closes it.
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    let mut token = HANDLE::default();
    // SAFETY: a live process handle; the token handle is owned below.
    if unsafe { OpenProcessToken(HANDLE(process.as_raw_handle()), TOKEN_QUERY, &mut token) }.is_err() {
        return Elevation::Unknown;
    }
    // SAFETY: a fresh handle nothing else owns; dropping it closes it.
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut written = 0u32;
    // SAFETY: `TokenElevation` fills a `TOKEN_ELEVATION`, which the buffer
    // is, with its size; the token is open for query.
    let read = unsafe {
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        )
    };
    match read {
        Ok(()) if elevation.TokenIsElevated != 0 => Elevation::Elevated,
        Ok(()) => Elevation::Standard,
        Err(_) => Elevation::Unknown,
    }
}

/// No token to read here: nothing off Windows asks ([`LOCAL_FOREGROUND`]).
#[cfg(not(windows))]
pub fn elevation(_pid: u32) -> Elevation {
    Elevation::Unknown
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

    /// Name of the foreground process on the PTY (e.g. `vim`), where the
    /// PTY has a foreground process group ([`LocalForeground::Group`]).
    pub fn foreground_name(&self) -> Option<String> {
        process_name(self.foreground_pid()?)
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

    /// The title the walk gives.
    fn name(rows: &[ProcessRow], root: u32) -> Option<String> {
        newest_descendant(rows, root).map(|found| found.name)
    }

    /// The process the walk names, whose elevation the badge shows.
    fn pid(rows: &[ProcessRow], root: u32) -> Option<u32> {
        newest_descendant(rows, root).map(|found| found.pid)
    }

    #[test]
    fn no_children_names_the_program() {
        let rows = rows(&[(4, 1, "System"), (100, 50, "cmd.exe")]);
        assert_eq!(name(&rows, 100).as_deref(), Some("cmd"));
        assert_eq!(pid(&rows, 100), Some(100));
    }

    #[test]
    fn follows_a_chain_down() {
        let rows = rows(&[(100, 50, "pwsh.exe"), (200, 100, "cmd.exe"), (300, 200, "python.exe")]);
        assert_eq!(name(&rows, 100).as_deref(), Some("python"));
        assert_eq!(pid(&rows, 100), Some(300));
        assert_eq!(pid(&rows, 200), Some(300), "from any root on the chain");
    }

    #[test]
    fn the_highest_pid_wins_among_siblings() {
        let rows = rows(&[(100, 50, "cmd.exe"), (300, 100, "ping.exe"), (250, 100, "timeout.exe"), (900, 250, "x.exe")]);
        assert_eq!(name(&rows, 100).as_deref(), Some("ping"));
        assert_eq!(pid(&rows, 100), Some(300));
    }

    #[test]
    fn exe_is_stripped_whatever_its_case() {
        let rows = rows(&[(1, 0, "NOTEPAD.EXE"), (2, 0, "make"), (3, 0, ".exe")]);
        assert_eq!(name(&rows, 1).as_deref(), Some("NOTEPAD"));
        assert_eq!(name(&rows, 2).as_deref(), Some("make"));
        assert_eq!(name(&rows, 3).as_deref(), Some(""));
        assert_eq!([pid(&rows, 1), pid(&rows, 2), pid(&rows, 3)], [Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn console_hosts_are_skipped() {
        let rows = rows(&[(100, 50, "cmd.exe"), (400, 100, "conhost.exe"), (500, 100, "OpenConsole.exe")]);
        assert_eq!(name(&rows, 100).as_deref(), Some("cmd"));
        assert_eq!(pid(&rows, 100), Some(100), "never a console host's pid");
    }

    #[test]
    fn unknown_root_and_cycles() {
        assert_eq!(newest_descendant(&rows(&[(1, 0, "a.exe")]), 7), None);
        // 10 -> 20 -> 10 (a reused pid): stops instead of looping.
        let cycle = rows(&[(10, 20, "a.exe"), (20, 10, "b.exe")]);
        assert!(newest_descendant(&cycle, 10).is_some());
        assert!(pid(&cycle, 10).is_some_and(|pid| pid == 10 || pid == 20));
    }

    /// A probe that answers from `known` and counts what it was asked.
    fn reader<'a>(known: &'a [(u32, Elevation)], asked: &'a mut Vec<u32>) -> impl FnMut(u32) -> Elevation + 'a {
        move |pid| {
            asked.push(pid);
            known.iter().find(|(at, _)| *at == pid).map_or(Elevation::Unknown, |(_, elevation)| *elevation)
        }
    }

    /// `sudo pwsh` in a tab, as Windows sudo starts it inline: the elevated
    /// shell is the newest descendant, and its elevation is the badge's.
    #[test]
    fn detection_names_the_newest_descendant_and_reads_its_elevation() {
        let rows = rows(&[
            (100, 50, "pwsh.exe"),
            (110, 100, "conhost.exe"),
            (200, 100, "sudo.exe"),
            (300, 200, "sudo.exe"),
            (400, 300, "pwsh.exe"),
            (500, 60, "cmd.exe"),
        ]);
        let known = [(400, Elevation::Elevated), (500, Elevation::Standard)];
        let mut asked = Vec::new();
        let found = detect_in(&rows, &[100, 500, 999], &mut ElevationCache::default(), reader(&known, &mut asked));
        assert_eq!(
            found,
            [
                Some(Detected { name: "pwsh".into(), elevation: Elevation::Elevated }),
                Some(Detected { name: "cmd".into(), elevation: Elevation::Standard }),
                None,
            ]
        );
        assert_eq!(asked, [400, 500], "only the foreground processes are read; never a root not running");
    }

    #[test]
    fn a_read_token_is_read_once_while_its_process_runs() {
        let rows = rows(&[(100, 50, "cmd.exe"), (200, 100, "python.exe")]);
        let known = [(200, Elevation::Standard)];
        let mut cache = ElevationCache::default();
        let mut asked = Vec::new();
        for _ in 0..3 {
            let found = detect_in(&rows, &[100], &mut cache, reader(&known, &mut asked));
            assert_eq!(found, [Some(Detected { name: "python".into(), elevation: Elevation::Standard })]);
        }
        assert_eq!(asked, [200]);
    }

    /// A failure isn't kept: a token that couldn't be read is tried on
    /// every look, and the first one read decides from then on.
    #[test]
    fn an_unread_token_is_read_again_on_the_next_look() {
        let rows = rows(&[(100, 50, "pwsh.exe")]);
        let mut cache = ElevationCache::default();
        let mut asked = Vec::new();
        for _ in 0..2 {
            let found = detect_in(&rows, &[100], &mut cache, reader(&[], &mut asked));
            assert_eq!(found, [Some(Detected { name: "pwsh".into(), elevation: Elevation::Unknown })]);
        }
        assert_eq!(asked, [100, 100]);
        let readable = [(100, Elevation::Elevated)];
        let mut asked = Vec::new();
        for _ in 0..2 {
            let found = detect_in(&rows, &[100], &mut cache, reader(&readable, &mut asked));
            assert_eq!(found, [Some(Detected { name: "pwsh".into(), elevation: Elevation::Elevated })]);
        }
        assert_eq!(asked, [100]);
    }

    /// The cache keeps only the pids of the latest look, so a pid that
    /// ended and came back as another process is read afresh.
    #[test]
    fn an_ended_process_is_forgotten_so_its_pid_is_read_afresh() {
        let mut cache = ElevationCache::default();
        let mut asked = Vec::new();
        let elevated = [(200, Elevation::Elevated)];
        let before = rows(&[(100, 50, "cmd.exe"), (200, 100, "pwsh.exe")]);
        detect_in(&before, &[100], &mut cache, reader(&elevated, &mut asked));
        // pwsh ended: the next look has no 200, and forgets it.
        let ended = rows(&[(100, 50, "cmd.exe")]);
        let found = detect_in(&ended, &[100], &mut cache, reader(&[(100, Elevation::Standard)], &mut asked));
        assert_eq!(found, [Some(Detected { name: "cmd".into(), elevation: Elevation::Standard })]);
        // 200 again, now a standard python.
        let reused = rows(&[(100, 50, "cmd.exe"), (200, 100, "python.exe")]);
        let found = detect_in(&reused, &[100], &mut cache, reader(&[(200, Elevation::Standard)], &mut asked));
        assert_eq!(found, [Some(Detected { name: "python".into(), elevation: Elevation::Standard })]);
        assert_eq!(asked, [200, 100, 200]);
    }

    #[test]
    fn no_look_finds_nothing_and_off_windows_no_token_is_read() {
        assert_eq!(detect_in(&[], &[100], &mut ElevationCache::default(), |_| Elevation::Elevated), [None]);
        assert!(detect_in(&rows(&[(1, 0, "a.exe")]), &[], &mut ElevationCache::default(), |_| Elevation::Elevated).is_empty());
        #[cfg(not(windows))]
        assert_eq!(elevation(std::process::id()), Elevation::Unknown);
    }
}

/// The token reads against this machine's processes.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    /// This test process's own token is readable, and reads as what this
    /// process is.
    #[test]
    fn reads_this_processes_elevation() {
        let elevated = brindle_remote_protocol::win::is_elevated().unwrap();
        let expected = if elevated { Elevation::Elevated } else { Elevation::Standard };
        assert_eq!(elevation(std::process::id()), expected);
    }

    /// A pid no process has can't be opened: unknown, never elevated.
    #[test]
    fn a_process_that_isnt_there_is_unknown() {
        assert_eq!(elevation(u32::MAX - 3), Elevation::Unknown);
    }

    /// An unelevated user can't open the System process's token: a
    /// refusal reads as unknown, never as elevated, though it runs as
    /// SYSTEM. An elevated test process (as on CI) may be let in, so it
    /// checks only the refusal it can make.
    #[test]
    fn a_refused_token_is_unknown() {
        if !brindle_remote_protocol::win::is_elevated().unwrap() {
            assert_eq!(elevation(4), Elevation::Unknown);
        }
    }

    /// One look at this machine finds this test process, which is running,
    /// and a root that isn't running finds nothing.
    #[test]
    fn a_look_at_this_machine_finds_this_process() {
        let found = detect(&[std::process::id(), u32::MAX - 3], &mut ElevationCache::default());
        assert_eq!(found.len(), 2);
        assert!(found[0].as_ref().is_some_and(|found| !found.name.is_empty()), "{found:?}");
        assert_eq!(found[1], None);
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
            name = process.foreground_name();
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
