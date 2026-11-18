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

/// Whether `pid` is still running.
pub fn alive(pid: u32) -> bool {
    use windows::Win32::Foundation::STILL_ACTIVE;
    use windows::Win32::System::Threading::GetExitCodeProcess;
    let Some(process) = Opened::new(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return false };
    let mut code = 0;
    // SAFETY: an open handle with the right to ask.
    unsafe { GetExitCodeProcess(process.0, &mut code) }.is_ok() && code == STILL_ACTIVE.0 as u32
}

/// The pid that started `pid`, as the process list has it.
pub fn parent(pid: u32) -> Option<u32> {
    all().into_iter().find(|p| p.pid == pid).map(|p| p.parent)
}

/// A window handle, as a number (0 for none).
pub type Window = isize;

/// The visible top-level window of `pid`, if it has one.
pub fn window_of(pid: u32) -> Option<Window> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};
    use windows::core::BOOL;

    struct Found {
        pid: u32,
        window: Option<Window>,
    }
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the `Found` passed to EnumWindows below.
        let found = unsafe { &mut *(found.0 as *mut Found) };
        let mut pid = 0;
        // SAFETY: plain queries about a window that exists.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == found.pid && IsWindowVisible(hwnd).as_bool() {
                found.window = Some(hwnd.0 as isize);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let mut found = Found { pid, window: None };
    // SAFETY: `found` outlives the enumeration.
    unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut Found as isize)) }.ok();
    found.window
}

fn hwnd(window: Window) -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(window as *mut c_void)
}

/// The top-level window with the title `title`, if there is one.
pub fn window_titled(title: &str) -> Option<Window> {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::{HSTRING, PCWSTR};
    let title = HSTRING::from(title);
    // SAFETY: a plain query with a NUL-terminated title.
    unsafe { FindWindowW(PCWSTR::null(), &title) }.ok().map(|w| w.0 as isize)
}

/// `window`'s owner (`GW_OWNER`), if it has one.
pub fn owner(window: Window) -> Option<Window> {
    use windows::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow};
    // SAFETY: any window handle may be asked about.
    unsafe { GetWindow(hwnd(window), GW_OWNER) }.ok().map(|w| w.0 as isize)
}

/// Posts `WM_CLOSE` to `window`: a message, not input.
pub fn post_close(window: Window) -> crate::Result {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    // SAFETY: a message with no pointers in it.
    unsafe { PostMessageW(Some(hwnd(window)), WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .map_err(|e| format!("posting WM_CLOSE to {window:#x}: {e}"))
}

/// Whether this process runs elevated (its token is the full one).
pub fn elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    // SAFETY: this process's own token, closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut used = 0;
    // SAFETY: `elevation` is the size passed.
    let asked = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut used,
        )
    };
    // SAFETY: the token is ours and open.
    unsafe { CloseHandle(token) }.ok();
    asked.is_ok() && elevation.TokenIsElevated != 0
}

/// Whether `token` is elevated (`TokenElevation`); an unreadable one counts
/// as elevated.
fn token_elevated(token: HANDLE) -> bool {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TokenElevation};
    let mut elevation = TOKEN_ELEVATION::default();
    let mut used = 0;
    // SAFETY: `elevation` is the size passed; the token is open.
    let asked = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut used,
        )
    };
    asked.is_err() || elevation.TokenIsElevated != 0
}

/// A primary token duplicated from the desktop shell's (Explorer's), which
/// runs unelevated beside an elevated process when UAC is on. The filtered
/// token linked to an elevated one can't be used instead: without
/// `SeTcbPrivilege` it comes back at `SecurityIdentification`, which
/// `DuplicateTokenEx` won't make a primary token (error 1346).
fn shell_token() -> crate::Result<HANDLE> {
    use windows::Win32::Security::{
        DuplicateTokenEx, SecurityImpersonation, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
        TOKEN_DUPLICATE, TOKEN_QUERY, TokenPrimary,
    };
    use windows::Win32::System::Threading::OpenProcessToken;
    use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};

    // SAFETY: no arguments.
    let shell = unsafe { GetShellWindow() };
    if shell.is_invalid() {
        return Err("no desktop shell window".into());
    }
    let mut pid = 0;
    // SAFETY: a window handle and an out pointer to a local.
    unsafe { GetWindowThreadProcessId(shell, Some(&mut pid)) };
    // Another user's shell would put the unelevated Brindle's pipes under
    // another SID, apart from the elevated server's for the wrong reason.
    use brindle_remote_protocol::win;
    let shell_user = win::principal_of(pid).map_err(|e| format!("the shell's user (pid {pid}): {e}"))?.sid;
    let own = win::own_sid().map_err(|e| format!("this process's user: {e}"))?;
    if shell_user != own {
        return Err(format!("the shell (pid {pid}) runs as {shell_user}, not as this process's {own}"));
    }
    let process =
        Opened::new(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok_or_else(|| format!("can't open the shell, pid {pid}"))?;
    let mut token = HANDLE::default();
    // SAFETY: the process handle is open; the token is closed below.
    unsafe { OpenProcessToken(process.0, TOKEN_DUPLICATE | TOKEN_QUERY, &mut token) }
        .map_err(|e| format!("opening the shell's token (pid {pid}): {e}"))?;
    let mut primary = HANDLE::default();
    let rights = TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID;
    // SAFETY: the token is open; the duplicate is the caller's to close.
    let duplicated = unsafe { DuplicateTokenEx(token, rights, None, SecurityImpersonation, TokenPrimary, &mut primary) };
    // SAFETY: ours and open.
    unsafe { CloseHandle(token) }.ok();
    duplicated.map_err(|e| format!("duplicating the shell's token (pid {pid}): {e}"))?;
    if token_elevated(primary) {
        // SAFETY: ours and open.
        unsafe { CloseHandle(primary) }.ok();
        return Err(format!("the shell (pid {pid}) runs elevated too; is UAC off?"));
    }
    Ok(primary)
}

/// Starts `command_line` unelevated, from an elevated process: with the
/// desktop shell's token ([`shell_token`]), the same user's unelevated one,
/// through `CreateProcessWithTokenW`, so it needs the Secondary Logon
/// service. Returns the process's pid; it runs with its user's own
/// environment, not this process's.
pub fn start_unelevated(command_line: &str) -> crate::Result<u32> {
    use windows::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CreateProcessWithTokenW, LOGON_WITH_PROFILE, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::PWSTR;

    let primary = shell_token()?;
    let startup = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut info = PROCESS_INFORMATION::default();
    let mut line: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the command line is writable and NUL-terminated, and outlives
    // the call; the handles returned are closed below.
    let started = unsafe {
        CreateProcessWithTokenW(
            primary,
            LOGON_WITH_PROFILE,
            None,
            Some(PWSTR(line.as_mut_ptr())),
            CREATE_NEW_CONSOLE,
            None,
            None,
            &startup,
            &mut info,
        )
    };
    // SAFETY: ours and open.
    unsafe { CloseHandle(primary) }.ok();
    started.map_err(|e| format!("starting {command_line:?} unelevated: {e}"))?;
    // SAFETY: the process's handles are ours.
    unsafe {
        CloseHandle(info.hThread).ok();
        CloseHandle(info.hProcess).ok();
    }
    Ok(info.dwProcessId)
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
    ensure!(alive(me.pid), "this process isn't alive");
    ensure!(parent(me.pid) == Some(me.parent), "parent of this process: {:?}", parent(me.pid));
    let mut ended = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "exit"])
        .spawn()
        .map_err(|e| format!("starting cmd: {e}"))?;
    let pid = ended.id();
    ended.wait().map_err(|e| e.to_string())?;
    // Its handle is still open here, so the pid isn't reused yet.
    ensure!(!alive(pid), "cmd {pid} is alive after exiting");
    Ok(())
}
