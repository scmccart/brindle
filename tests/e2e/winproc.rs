//! Windows process queries for the harness, where Linux reads `/proc`: the
//! process list, a process's image and arguments, and ending one.

use std::ffi::c_void;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
};

pub struct Process {
    pub pid: u32,
    pub parent: u32,
    /// The executable's file name, such as `OpenConsole.exe`.
    pub name: String,
}

/// Every running process.
pub fn all() -> Vec<Process> {
    let mut found = Vec::new();
    // SAFETY: a process snapshot, walked with a correctly sized entry and
    // closed afterwards.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            found.push(Process {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        CloseHandle(snapshot).ok();
    }
    found
}

/// The image path of `pid`, if it is running and ours to ask about.
pub fn image(pid: u32) -> Option<PathBuf> {
    brindle_remote_protocol::win::image_path_of(pid).ok()
}

struct Opened(HANDLE);

impl Opened {
    fn new(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Self> {
        // SAFETY: plain arguments; the handle is closed on drop.
        unsafe { OpenProcess(access, false, pid) }.ok().map(Self)
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: this handle is ours and open.
        unsafe { CloseHandle(self.0) }.ok();
    }
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(process: HANDLE, class: u32, info: *mut c_void, len: u32, used: *mut u32) -> i32;
}

/// `ProcessCommandLineInformation`: a `UNICODE_STRING`, then its text.
const COMMAND_LINE: u32 = 60;

/// The command line `pid` was started with.
pub fn command_line(pid: u32) -> Option<String> {
    #[repr(C)]
    struct UnicodeString {
        /// In bytes.
        length: u16,
        maximum: u16,
        buffer: *const u16,
    }
    let process = Opened::new(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    // A command line has at most 32767 units; u64s keep the header aligned.
    let mut info = vec![0u64; (std::mem::size_of::<UnicodeString>() + 2 * 32 * 1024) / 8];
    let mut used = 0;
    // SAFETY: the buffer is `len` bytes, aligned for the header the call
    // writes at its start, whose `buffer` points into the same allocation.
    unsafe {
        let len = (info.len() * 8) as u32;
        if NtQueryInformationProcess(process.0, COMMAND_LINE, info.as_mut_ptr().cast(), len, &mut used) < 0 {
            return None;
        }
        let header = &*info.as_ptr().cast::<UnicodeString>();
        if header.buffer.is_null() {
            return None;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(header.buffer, usize::from(header.length) / 2)))
    }
}

/// The first argument in a command line, after the program (quoted when it
/// has spaces).
pub fn first_argument(command_line: &str) -> Option<&str> {
    let line = command_line.trim_start();
    let rest = match line.strip_prefix('"') {
        Some(quoted) => quoted.split_once('"')?.1,
        None => line.split_once(char::is_whitespace)?.1,
    };
    rest.split_whitespace().next()
}

/// Ends `pid` at once.
pub fn kill(pid: u32) {
    if let Some(process) = Opened::new(pid, PROCESS_TERMINATE) {
        // SAFETY: an open handle with the right to end the process.
        unsafe { TerminateProcess(process.0, 1) }.ok();
    }
}

pub fn self_test() -> crate::Result {
    use crate::ensure;
    for (line, want) in [
        (r#""C:\a b\brindle-remote.exe" hold --ready-handle 5"#, Some("hold")),
        (r"C:\a\brindle-remote.exe serve", Some("serve")),
        (r#""C:\a b\brindle-remote.exe""#, None),
        (r"brindle-remote.exe", None),
    ] {
        ensure!(first_argument(line) == want, "{line:?}: {:?}", first_argument(line));
    }
    let own = command_line(std::process::id()).ok_or("no command line for this process")?;
    ensure!(own.contains("--self-test"), "own command line: {own:?}");
    let me = all().into_iter().find(|p| p.pid == std::process::id()).ok_or("this process isn't listed")?;
    ensure!(image(me.pid).is_some_and(|path| path.file_name().is_some_and(|n| *n == *me.name)), "image of {}", me.name);
    Ok(())
}
