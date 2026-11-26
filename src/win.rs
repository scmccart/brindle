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

/// Whether this process has no standard input handle: a GUI program started
/// from a console with nothing redirected usually has none (`GetStdHandle`
/// null or invalid). A handle a parent passed, a file's or a pipe's (or its
/// console's, which `IsTerminal` catches), is there.
pub fn stdin_absent() -> bool {
    use windows::Win32::System::Console::STD_INPUT_HANDLE;
    // SAFETY: reads this process's standard handle; nothing is borrowed.
    match unsafe { GetStdHandle(STD_INPUT_HANDLE) } {
        // Null or INVALID_HANDLE_VALUE.
        Ok(handle) => handle.is_invalid(),
        Err(_) => true,
    }
}

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

/// Whether DirectWrite's system font collection (the one GPUI lists) has a
/// family named `family`, in any of its localized names. An `Err` is
/// DirectWrite failing to answer, not a family that isn't there.
pub fn has_font_family(family: &str) -> std::io::Result<bool> {
    use windows::Win32::Graphics::DirectWrite::{DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory};
    let found = || -> windows::core::Result<bool> {
        // SAFETY: plain DirectWrite calls with valid out-pointers.
        unsafe {
            let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut collection = None;
            factory.GetSystemFontCollection(&mut collection, false)?;
            let Some(collection) = collection else { return Ok(false) };
            let (mut index, mut exists) = (0, windows::core::BOOL::default());
            collection.FindFamilyName(&windows::core::HSTRING::from(family), &mut index, &mut exists)?;
            Ok(exists.as_bool())
        }
    };
    found().map_err(|error| std::io::Error::other(format!("DirectWrite: {error}")))
}

/// The window's HWND.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enum: raw_window_handle's non-exhaustive RawWindowHandle, of which only Win32's counts"
)]
pub fn hwnd(window: &gpui::Window) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // GPUI's own `window_handle` is another thing.
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(win32) => Some(windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut std::ffi::c_void)),
        _ => None,
    }
}

/// Whether this process runs elevated (high integrity or above), which
/// decides its default state directory.
pub fn elevated() -> Result<bool, String> {
    crate::remote::protocol::win::is_elevated().map_err(|error| format!("reading this process's token: {error}"))
}

/// What [`protect_state_dir`] makes the directory as; it says why.
const STATE_DIR_CONTENTS: crate::remote::protocol::win::Contents =
    crate::remote::protocol::win::Contents::Executables;

/// Makes or checks an elevated Brindle's state directory `dir` for
/// Administrators and SYSTEM only, as its terminal server's `run-elevated`
/// is ([`win::protected_dir`](crate::remote::protocol::win::protected_dir)),
/// and holds it until the process exits, so an unelevated process can't add,
/// swap or rename what is read from it. `Err` says why, naming `dir`.
///
/// It is made with `Contents::Executables`, not `State`: a directory found
/// there without the elevated descriptor is set aside and made fresh, never
/// re-protected in place, since an unelevated process could have planted a
/// `session.toml` in it that an elevated Brindle would then restore
/// ([`STATE_DIR_CONTENTS`], which a test pins).
pub fn protect_state_dir(dir: &std::path::Path) -> Result<(), String> {
    use crate::remote::protocol::win::{ProtectedDir, protected_dir};

    static HELD: std::sync::OnceLock<ProtectedDir> = std::sync::OnceLock::new();
    let held = protected_dir(dir, STATE_DIR_CONTENTS).map_err(|error| error.to_string())?;
    // Called once, before the session lock; a second hold would be the same
    // directory's.
    HELD.set(held).ok();
    Ok(())
}

/// Shows `text` in a message box (an error icon, titled Brindle) when this
/// process has no console and its stderr goes nowhere, so what it printed
/// was lost: a launch from the Start Menu or Explorer that gives up. Called
/// after [`attach_parent_console`]; one started from a console, or with its
/// stderr redirected, has printed it already and shows nothing.
pub fn show_unprinted(text: &str) {
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW};
    // SAFETY: reads the process's console window; no argument.
    let console = !unsafe { GetConsoleWindow() }.is_invalid();
    // SAFETY: GetStdHandle only reads the process's standard handles.
    let stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) }.is_ok_and(|handle| !handle.is_invalid());
    if console || stderr {
        return;
    }
    // SAFETY: both strings are NUL-terminated and outlive the call; the box
    // has no owner window.
    unsafe {
        MessageBoxW(None, &windows::core::HSTRING::from(text), w!("Brindle"), MB_OK | MB_ICONERROR | MB_SETFOREGROUND)
    };
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

#[cfg(test)]
mod tests {
    use crate::remote::protocol::access::{Found, Step, protected_step};

    /// The elevated state directory keeps nothing it didn't make: whatever
    /// is found in its place without the elevated descriptor (a directory
    /// an unelevated process made, a link, a file) is set aside, never
    /// protected again with its files.
    #[test]
    fn a_state_directory_found_unprotected_is_set_aside() {
        for found in [Found::Other, Found::Link, Found::NotDir] {
            assert_eq!(protected_step(found, super::STATE_DIR_CONTENTS, false), Step::SetAside, "{found:?}");
        }
        assert_eq!(protected_step(Found::Matches, super::STATE_DIR_CONTENTS, false), Step::Use);
    }
}
