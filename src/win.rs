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

/// Brindle's AppUserModelID. Its windows, its Start Menu shortcuts and its
/// toasts all carry it, so Windows treats them as one app.
pub const APP_ID: &str = "Brindle.Terminal";

/// Where an app with no shortcut registers its toast name and icon.
const APP_ID_KEY: &str = r"Software\Classes\AppUserModelId\Brindle.Terminal";

/// Writes `bytes` to a temporary file beside `path`, then renames it over
/// `path`, so a concurrent Brindle never reads half a file.
pub fn write_replacing(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        std::fs::remove_file(&tmp).ok();
    })
}

/// Gives this process Brindle's AppUserModelID. Called before any window
/// opens.
pub fn set_app_id() {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    // SAFETY: sets a process-wide string; nothing else is borrowed.
    if let Err(err) = unsafe { SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(APP_ID)) } {
        log::warn!("setting the AppUserModelID: {err}");
    }
}

/// Registers [`APP_ID`] for the current user, so toasts work without a
/// Start Menu shortcut: `DisplayName` Brindle, and `IconUri` when given.
pub fn register_app_id(icon: Option<&std::path::Path>) -> windows::core::Result<()> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
    };
    use windows::core::HSTRING;
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under HKCU; closed below.
    unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(APP_ID_KEY), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    }
    .ok()?;
    let set = |name: &str, value: &str| {
        let data: Vec<u8> = value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
        // SAFETY: `key` is open; `data` is a NUL-terminated UTF-16 string.
        unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(&data)) }.ok()
    };
    let result = set("DisplayName", "Brindle").and_then(|()| match icon {
        Some(icon) => set("IconUri", &icon.display().to_string()),
        None => Ok(()),
    });
    // SAFETY: closes the key opened above.
    unsafe { RegCloseKey(key) }.ok().ok();
    result
}

/// Removes what [`register_app_id`] wrote. A missing key is fine.
pub fn unregister_app_id() -> windows::core::Result<()> {
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
    // SAFETY: deletes Brindle's own key under HKCU.
    match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &windows::core::HSTRING::from(APP_ID_KEY)) } {
        ERROR_FILE_NOT_FOUND => Ok(()),
        status => status.ok(),
    }
}

/// The window's HWND.
pub fn hwnd(window: &gpui::Window) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // GPUI's own `window_handle` is another thing.
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(win32) => Some(windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut std::ffi::c_void)),
        _ => None,
    }
}

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
