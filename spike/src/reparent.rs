//! 0.1: does a console program's dialog end up owned by a given HWND when a
//! *separate* process owns the pseudoconsole and calls ConptyReparentPseudoConsole?
//!
//! The harness creates two top-level windows (W1, W2) standing in for two
//! Brindle windows, creates a pseudoconsole with the shipped ConPTY, and runs
//! a child (`spike dialog`) that repeatedly calls GetConsoleWindow() and opens
//! a message box owned by it. Between dialogs the harness reparents the
//! pseudoconsole to W1, to W2, to a destroyed HWND, and to none, and each time
//! logs the owner chain of the dialog.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::HPCON;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetWindow,
    GetWindowLongPtrW, IsWindow, PeekMessageW, PostMessageW, RegisterClassW, TranslateMessage,
    CW_USEDEFAULT, GWLP_HWNDPARENT, GW_OWNER, MSG, PM_REMOVE, WINDOW_EX_STYLE, WM_CLOSE, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};

use crate::conpty;
use crate::pcon;

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, w, l) }
}

pub fn make_window(title: PCWSTR) -> HWND {
    // SAFETY: module handle of the current process.
    let hinst = unsafe { GetModuleHandleW(None) }.unwrap();
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: title,
        ..Default::default()
    };
    // SAFETY: valid class; re-register is harmless (returns 0).
    unsafe { RegisterClassW(&class) };
    // SAFETY: registered class; default position; hidden (not shown).
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            title,
            title,
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            400,
            200,
            None,
            None,
            Some(hinst.into()),
            None,
        )
    }
    .expect("CreateWindowExW")
}

pub fn pump() {
    let mut msg = MSG::default();
    // SAFETY: standard non-blocking message drain for our windows.
    while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn wait_dialog() -> Option<HWND> {
    for _ in 0..60 {
        pump();
        // Message-box class is #32770; our title is SPIKE.
        // SAFETY: looking up a top-level window by class+title.
        if let Ok(h) = unsafe { FindWindowW(w!("#32770"), w!("SPIKE")) } {
            if !h.is_invalid() {
                return Some(h);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

pub fn owner(h: HWND) -> HWND {
    // SAFETY: GetWindow over a valid HWND.
    unsafe { GetWindow(h, GW_OWNER) }.unwrap_or_default()
}

pub fn hwndparent(h: HWND) -> isize {
    // SAFETY: reading the owner/parent slot.
    unsafe { GetWindowLongPtrW(h, GWLP_HWNDPARENT) }
}

fn report(tag: &str, w1: HWND, w2: HWND) {
    match wait_dialog() {
        None => println!("[0.1] {tag}: no dialog appeared"),
        Some(dlg) => {
            let o1 = owner(dlg); // the pseudoconsole window (in OpenConsole.exe)
            let o2 = owner(o1); // its owner, set by reparent
            let which = |h: HWND| {
                if h == w1 {
                    "W1"
                } else if h == w2 {
                    "W2"
                } else if h.0.is_null() {
                    "none"
                } else {
                    "other"
                }
            };
            println!(
                "[0.1] {tag}: dialog={:?} -> owner(console)={:?} -> owner={:?} [{}]  (console GWLP_HWNDPARENT={:#x})",
                dlg.0, o1.0, o2.0, which(o2), hwndparent(o1)
            );
            // Dismiss so the child loops to the next dialog.
            // SAFETY: posting WM_CLOSE to the dialog.
            unsafe { PostMessageW(Some(dlg), WM_CLOSE, WPARAM(0), LPARAM(0)) }.ok();
            std::thread::sleep(std::time::Duration::from_millis(400));
            pump();
        }
    }
}

fn reparent(hpcon: HPCON, target: HWND, tag: &str) {
    let module = conpty::load_shipped();
    // SAFETY: real export of the stated type.
    let f: conpty::ReparentFn =
        unsafe { conpty::func(module, w_to_s()) }.expect("ConptyReparentPseudoConsole");
    // SAFETY: hpcon open; any HWND accepted.
    let hr = unsafe { f(hpcon, target) };
    println!("[0.1] reparent -> {tag}: hr={:#010x}", hr.0 as u32);
    std::thread::sleep(std::time::Duration::from_millis(800));
}

fn w_to_s() -> windows::core::PCSTR {
    windows::core::s!("ConptyReparentPseudoConsole")
}

pub fn run() {
    let w1 = make_window(w!("BrindleSpikeW1"));
    let w2 = make_window(w!("BrindleSpikeW2"));
    println!("[0.1] W1={:?} W2={:?}", w1.0, w2.0);
    pump();

    // Child in a separate process owns nothing; the pseudoconsole is ours.
    let exe = std::env::current_exe().unwrap();
    let session = pcon::spawn(&format!("\"{}\" dialog", exe.display()), 80, 25);
    println!("[0.1] child pid={}", session.pid);

    reparent(session.hpcon, w1, "W1");
    report("after reparent to W1", w1, w2);

    reparent(session.hpcon, w2, "W2");
    report("after reparent to W2", w1, w2);

    // Destroy W1 while the session lives, then make it the target.
    // SAFETY: our window.
    unsafe { DestroyWindow(w1) }.ok();
    // SAFETY: checking validity.
    println!("[0.1] destroyed W1; IsWindow(W1)={}", unsafe { IsWindow(Some(w1)) }.as_bool());
    reparent(session.hpcon, w1, "destroyed W1");
    report("after reparent to destroyed W1", w1, w2);

    // Recover by reparenting to the live W2.
    reparent(session.hpcon, w2, "W2 again");
    report("after reparent back to W2", w1, w2);

    // Reparent to none.
    reparent(session.hpcon, HWND(std::ptr::null_mut()), "none (null HWND)");
    report("after reparent to none", w1, w2);

    // SAFETY: tidy up the surviving window.
    unsafe { DestroyWindow(w2) }.ok();
    println!("[0.1] done");
}

pub fn run_dialog() {
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK};
    for _ in 0..6 {
        // SAFETY: returns this program's console window (the pseudoconsole's).
        let console = unsafe { GetConsoleWindow() };
        // SAFETY: owner is the console window; modal, dismissed by the harness.
        unsafe { MessageBoxW(Some(console), w!("spike dialog"), w!("SPIKE"), MB_OK) };
    }
}
