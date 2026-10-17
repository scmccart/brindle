//! Small Windows-only helpers.

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};
use windows::core::w;

/// For processes Brindle starts outside a tab: Brindle has no console for
/// them to share, so a console program would otherwise get a window.
pub const CREATE_NO_WINDOW: u32 = windows::Win32::System::Threading::CREATE_NO_WINDOW.0;

/// Points stdout and stderr at the console Brindle was started from, for
/// output printed just before exiting.
///
/// Brindle is a GUI program, so it starts with no console. A handle that is
/// already set (output redirected to a file or pipe) is kept. With no parent
/// console (a Start Menu launch), nothing changes and the output is lost.
pub fn attach_parent_console() {
    // SAFETY: GetStdHandle only reads the process's standard handles.
    let missing = |which: STD_HANDLE| !unsafe { GetStdHandle(which) }.is_ok_and(|h| !h.0.is_null());
    let (out, err) = (missing(STD_OUTPUT_HANDLE), missing(STD_ERROR_HANDLE));
    if !out && !err {
        return;
    }
    // SAFETY: attaching has no preconditions; it fails if there is no
    // parent console or this process already has one.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return;
    }
    // SAFETY: CONOUT$ names the attached console's screen buffer; the handle
    // lives until the process exits, as standard handles do.
    let Ok(console) = (unsafe {
        CreateFileW(
            w!("CONOUT$"),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }) else {
        return;
    };
    let set = |which: STD_HANDLE, handle: HANDLE| {
        // SAFETY: `handle` is a valid console handle owned by the process.
        unsafe { SetStdHandle(which, handle) }.ok();
    };
    if out {
        set(STD_OUTPUT_HANDLE, console);
    }
    if err {
        set(STD_ERROR_HANDLE, console);
    }
}
