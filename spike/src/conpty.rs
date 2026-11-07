//! Loads the shipped `conpty.dll` by absolute path and resolves its
//! pseudoconsole exports, mirroring `src/terminal/conpty.rs`'s loader.

use std::path::{Path, PathBuf};

use windows::core::{s, PCSTR, PCWSTR, HRESULT};
use windows::Win32::Foundation::{HANDLE, HMODULE, HWND};
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};

pub const VERSION: &str = "1.25.260930003";

pub type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
pub type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
pub type CloseFn = unsafe extern "system" fn(HPCON);
pub type ReleaseFn = unsafe extern "system" fn(HPCON) -> HRESULT;
pub type ReparentFn = unsafe extern "system" fn(HPCON, HWND) -> HRESULT;
pub type PackFn = unsafe extern "system" fn(HANDLE, HANDLE, HANDLE, *mut HPCON) -> HRESULT;

/// Absolute path to the shipped conpty.dll in this checkout.
pub fn shipped_dll() -> PathBuf {
    // spike/ sits beside .conpty/ at the repo root.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap()
        .join(".conpty")
        .join(VERSION)
        .join("conpty.dll")
}

pub fn load_shipped() -> HMODULE {
    let path = shipped_dll();
    assert!(path.exists(), "shipped conpty.dll missing at {}", path.display());
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    use std::os::windows::ffi::OsStrExt as _;
    // SAFETY: absolute NUL-terminated path; flags restrict the DLL's own deps.
    unsafe {
        LoadLibraryExW(
            PCWSTR(wide.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    }
    .expect("LoadLibraryExW(conpty.dll)")
}

pub fn kernel32() -> HMODULE {
    use windows::core::w;
    // SAFETY: kernel32 is always loaded.
    unsafe { GetModuleHandleW(w!("kernel32.dll")) }.expect("kernel32")
}

/// Returns the address of `name` in `module`, or None.
pub fn addr(module: HMODULE, name: PCSTR) -> Option<*const ()> {
    // SAFETY: GetProcAddress only reads the export table.
    unsafe { GetProcAddress(module, name) }.map(|f| f as *const ())
}

/// # Safety
/// `F` must be the export's real function type.
pub unsafe fn func<F: Copy>(module: HMODULE, name: PCSTR) -> Option<F> {
    let a = unsafe { GetProcAddress(module, name) }?;
    Some(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, F>(&a) })
}

/// The pseudoconsole export names, shipped vs inbox differ by the `Conpty` prefix.
pub const SHIPPED_NAMES: &[(&str, PCSTR)] = &[
    ("Create", s!("ConptyCreatePseudoConsole")),
    ("CreateAsUser", s!("ConptyCreatePseudoConsoleAsUser")),
    ("Resize", s!("ConptyResizePseudoConsole")),
    ("Clear", s!("ConptyClearPseudoConsole")),
    ("ShowHide", s!("ConptyShowHidePseudoConsole")),
    ("Reparent", s!("ConptyReparentPseudoConsole")),
    ("Release", s!("ConptyReleasePseudoConsole")),
    ("Close", s!("ConptyClosePseudoConsole")),
    ("Pack", s!("ConptyPackPseudoConsole")),
];

pub const INBOX_NAMES: &[(&str, PCSTR)] = &[
    ("Create", s!("CreatePseudoConsole")),
    ("Resize", s!("ResizePseudoConsole")),
    ("Clear", s!("ClearPseudoConsole")),
    ("Close", s!("ClosePseudoConsole")),
    ("Release", s!("ReleasePseudoConsole")),
    // these never existed inbox; probed to prove their absence:
    ("Reparent", s!("ReparentPseudoConsole")),
    ("Pack", s!("PackPseudoConsole")),
];
