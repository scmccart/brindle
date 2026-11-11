//! Which ConPTY: the packaged one this binary embeds, or the one built
//! into Windows.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

use windows::Win32::Foundation::{HANDLE, HMODULE, HWND};
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{HRESULT, PCSTR, PCWSTR, s, w};

/// The packaged ConPTY embedded by `build.rs`: `VERSION` and `FILES`.
pub(super) mod bundle {
    include!(concat!(env!("OUT_DIR"), "/conpty.rs"));
}

const DLL: &str = "conpty.dll";
const HOST: &str = "OpenConsole.exe";

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HPCON);
type ReleaseFn = unsafe extern "system" fn(HPCON) -> HRESULT;
type ReparentFn = unsafe extern "system" fn(HPCON, HWND) -> HRESULT;

/// The pseudoconsole functions of one ConPTY.
pub(super) struct Api {
    pub create: CreateFn,
    pub resize: ResizeFn,
    pub close: CloseFn,
    /// Lets the host exit once the client is gone. Not on every build.
    pub release: Option<ReleaseFn>,
    /// Only the packaged host has it.
    pub reparent: Option<ReparentFn>,
    /// Whether this is the packaged ConPTY.
    pub packaged: bool,
}

/// The ConPTY for a new pseudoconsole: the one built into Windows when
/// `inbox`, or when the packaged one can't be used. Each is resolved the
/// first time it is asked for, and the packaged one's failure is logged
/// then, once.
pub(super) fn api(inbox: bool) -> &'static Api {
    static PACKAGED: OnceLock<Option<Api>> = OnceLock::new();
    static INBOX: OnceLock<Api> = OnceLock::new();
    static ASKED: Once = Once::new();
    if inbox {
        ASKED.call_once(|| {
            eprintln!(
                "conpty: BRINDLE_CONPTY=inbox: such sessions use the ConPTY built into Windows"
            );
        });
    } else if let Some(api) = PACKAGED.get_or_init(packaged) {
        return api;
    }
    INBOX.get_or_init(inbox_api)
}

/// The packaged ConPTY, or `None` after saying why it isn't usable.
fn packaged() -> Option<Api> {
    let version = bundle::VERSION;
    let Some(files) = bundle::FILES else {
        eprintln!(
            "conpty: this build embeds no ConPTY (run scripts/fetch-conpty.ps1 before building): \
             sessions use the ConPTY built into Windows, which repaints output and drops mouse modes"
        );
        return None;
    };
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let extract_dir = std::env::var_os("LOCALAPPDATA")
        .filter(|dir| !dir.is_empty())
        .map(|dir| {
            PathBuf::from(dir)
                .join("brindle")
                .join("conpty")
                .join(version)
        });
    let loaded =
        shipped_dir(exe_dir.as_deref(), extract_dir.as_deref(), files).and_then(|dir| {
            load(&dir)
                .map(|api| (api, dir))
                .map_err(|err| format!("loading it: {err}"))
        });
    match loaded {
        Ok((api, dir)) => {
            eprintln!(
                "conpty: sessions use ConPTY {version} from {}",
                dir.display()
            );
            Some(api)
        }
        Err(why) => {
            eprintln!(
                "conpty: the packaged ConPTY {version} isn't usable ({why}): sessions use the \
                 ConPTY built into Windows"
            );
            None
        }
    }
}

/// Where to load the packaged ConPTY from: `exe_dir` when both files there
/// are the embedded ones (an installed helper, or one in the build
/// directory), else `extract_dir`, after writing the embedded files there if
/// they differ. The current directory and `PATH` are never candidates.
fn shipped_dir(
    exe_dir: Option<&Path>,
    extract_dir: Option<&Path>,
    (dll, host): (&[u8], &[u8]),
) -> Result<PathBuf, String> {
    let matches = |dir: &Path| same(&dir.join(DLL), dll) && same(&dir.join(HOST), host);
    if let Some(dir) = exe_dir.filter(|dir| matches(dir)) {
        return Ok(dir.to_path_buf());
    }
    let dir = extract_dir.ok_or("no local app data directory")?;
    if !matches(dir) {
        std::fs::create_dir_all(dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
        for (name, bytes) in [(DLL, dll), (HOST, host)] {
            write_replacing(&dir.join(name), bytes)
                .map_err(|err| format!("writing {}: {err}", dir.join(name).display()))?;
        }
    }
    Ok(dir.to_path_buf())
}

fn same(path: &Path, bytes: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|on_disk| on_disk == bytes)
}

/// Writes `bytes` to a temporary file beside `path`, then renames it over
/// `path`, so a concurrent process never reads half a file.
fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        std::fs::remove_file(&tmp).ok();
    })
}

/// Loads `dir\conpty.dll` by absolute path. It finds `OpenConsole.exe`
/// beside itself.
fn load(dir: &Path) -> io::Result<Api> {
    let path = wide(dir.join(DLL).as_os_str());
    // SAFETY: an absolute, NUL-terminated path; the flags restrict the DLL's
    // own dependencies to its directory and System32.
    let module = unsafe {
        LoadLibraryExW(
            PCWSTR(path.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    }?;
    // SAFETY: the signatures are those `conpty.h` declares for these names.
    unsafe {
        Ok(Api {
            create: function(module, s!("ConptyCreatePseudoConsole")).ok_or_else(missing)?,
            resize: function(module, s!("ConptyResizePseudoConsole")).ok_or_else(missing)?,
            close: function(module, s!("ConptyClosePseudoConsole")).ok_or_else(missing)?,
            release: function(module, s!("ConptyReleasePseudoConsole")),
            reparent: function(module, s!("ConptyReparentPseudoConsole")),
            packaged: true,
        })
    }
}

fn inbox_api() -> Api {
    // SAFETY: kernel32 is always loaded, and these are its documented
    // pseudoconsole functions (Windows 10 1809 and later).
    unsafe {
        let module = GetModuleHandleW(w!("kernel32.dll")).expect("kernel32 is loaded");
        Api {
            create: function(module, s!("CreatePseudoConsole")).expect("CreatePseudoConsole"),
            resize: function(module, s!("ResizePseudoConsole")).expect("ResizePseudoConsole"),
            close: function(module, s!("ClosePseudoConsole")).expect("ClosePseudoConsole"),
            release: function(module, s!("ReleasePseudoConsole")),
            reparent: None,
            packaged: false,
        }
    }
}

fn missing() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "conpty.dll lacks a pseudoconsole function",
    )
}

/// # Safety
/// `F` must be the export's real function type.
unsafe fn function<F: Copy>(module: HMODULE, name: PCSTR) -> Option<F> {
    // SAFETY: GetProcAddress only reads the module's export table.
    let address = unsafe { GetProcAddress(module, name) }?;
    // SAFETY: the caller names the export's type; both are fn pointers.
    Some(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, F>(&address) })
}

pub(super) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "brindle-remote-conpty-{}-{name}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const FILES: (&[u8], &[u8]) = (b"dll bytes", b"host bytes");

    #[test]
    fn matching_files_beside_the_exe_are_used() {
        let exe = scratch("exe-match");
        std::fs::write(exe.join(DLL), FILES.0).unwrap();
        std::fs::write(exe.join(HOST), FILES.1).unwrap();
        let extract = scratch("extract-unused");
        assert_eq!(
            shipped_dir(Some(&exe), Some(&extract), FILES),
            Ok(exe.clone())
        );
        assert!(!extract.join(DLL).exists(), "nothing extracted");
    }

    #[test]
    fn a_different_dll_beside_the_exe_is_ignored() {
        let exe = scratch("exe-planted");
        std::fs::write(exe.join(DLL), b"someone else's").unwrap();
        std::fs::write(exe.join(HOST), FILES.1).unwrap();
        let extract = scratch("extract-planted");
        assert_eq!(
            shipped_dir(Some(&exe), Some(&extract), FILES),
            Ok(extract.clone())
        );
        assert_eq!(std::fs::read(extract.join(DLL)).unwrap(), FILES.0);
        assert_eq!(std::fs::read(extract.join(HOST)).unwrap(), FILES.1);
    }

    #[test]
    fn a_stale_extraction_is_rewritten() {
        let extract = scratch("extract-stale");
        std::fs::write(extract.join(DLL), b"old").unwrap();
        std::fs::write(extract.join(HOST), FILES.1).unwrap();
        assert_eq!(
            shipped_dir(None, Some(&extract), FILES),
            Ok(extract.clone())
        );
        assert_eq!(std::fs::read(extract.join(DLL)).unwrap(), FILES.0);
    }

    #[test]
    fn no_candidates_is_an_error() {
        assert!(shipped_dir(None, None, FILES).is_err());
    }

    #[test]
    fn each_api_is_resolved_once() {
        assert!(std::ptr::eq(api(true), api(true)));
        assert!(std::ptr::eq(api(false), api(false)));
        assert!(!api(true).packaged);
        // The packaged one has what the built-in one lacks.
        if api(false).packaged {
            assert!(api(false).reparent.is_some());
            assert!(!std::ptr::eq(api(false), api(true)));
        }
    }
}
