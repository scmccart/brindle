//! 0.1 follow-ups: what happens to a session's console window when its
//! CURRENT owner window goes away, with the owner in this process or in a
//! third one, and with or without reparenting to null first.
//!
//! `reparent-ext [scenario...]` is the holder: it owns every pseudoconsole.
//! `owner-window` is process B: it owns one top-level window and prints its HWND.
//! `dialog-ext <log> <title>` is the console program: on each `<log>.goN` file
//! it logs GetConsoleWindow()/IsWindow and opens a MessageBox owned by it.

use std::ffi::c_void;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, SetLastError, HANDLE, HWND, LPARAM, WIN32_ERROR, WPARAM};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::Threading::TerminateProcess;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, FindWindowW, GetWindow, IsWindow, MessageBoxW, PostMessageW, GW_OWNER, MB_OK,
    WM_CLOSE,
};

use crate::reparent::{hwndparent, make_window, owner, pump};
use crate::{conpty, pcon, util};

fn sleep(ms: u64) {
    // Keep our windows responsive: destroying an owner sends cross-thread messages.
    let mut left = ms;
    while left > 0 {
        pump();
        let step = left.min(50);
        std::thread::sleep(Duration::from_millis(step));
        left -= step;
    }
    pump();
}

fn is_window(h: HWND) -> bool {
    !h.0.is_null() && unsafe { IsWindow(Some(h)) }.as_bool()
}

/// An owner window, in this process or in a `spike owner-window` process.
struct Owner {
    hwnd: HWND,
    process: Option<Child>,
}

impl Owner {
    fn make(remote: bool) -> Owner {
        if !remote {
            return Owner { hwnd: make_window(w!("BrindleSpikeOwner")), process: None };
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("owner-window")
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn owner-window");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let hwnd = HWND(line.trim().parse::<usize>().expect("hwnd") as *mut c_void);
        Owner { hwnd, process: Some(child) }
    }

    fn remove(&mut self, how: &str) {
        match (how, self.process.as_mut()) {
            ("destroy", None) => {
                let r = unsafe { DestroyWindow(self.hwnd) };
                println!("  DestroyWindow(owner) ok={}", r.is_ok());
            }
            ("close", Some(child)) => {
                // B's wndproc is DefWindowProc: WM_CLOSE makes B destroy its own window, then B exits.
                unsafe { PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.ok();
                let status = child.wait();
                println!("  posted WM_CLOSE to B's window; B exited: {status:?}");
            }
            ("terminate", Some(child)) => {
                child.kill().ok();
                let status = child.wait();
                println!("  TerminateProcess(B); B exited: {status:?}");
            }
            other => panic!("bad removal {:?}", other.0),
        }
    }
}

struct Sess {
    s: pcon::Session,
    log: PathBuf,
    title: String,
    step: u32,
    printed: usize,
    names: Vec<(HWND, String)>,
    console: HWND,
}

static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Sess {
    fn new() -> Sess {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("{}-{n}", std::process::id());
        let log = std::env::temp_dir().join(format!("spike-reparent-{id}.log"));
        let _ = std::fs::remove_file(&log);
        let title = format!("SPIKE-{id}");
        let exe = std::env::current_exe().unwrap();
        let s = pcon::spawn(
            &format!("\"{}\" dialog-ext \"{}\" {title}", exe.display(), log.display()),
            80,
            25,
        );
        // Nobody renders this session; drain its output so ConPTY never blocks.
        let out = s.output_read.0 as isize;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let mut n = 0u32;
                let r = unsafe { ReadFile(HANDLE(out as *mut c_void), Some(&mut buf), Some(&mut n), None) };
                if r.is_err() || n == 0 {
                    break;
                }
            }
        });
        println!("  session: program pid={}", s.pid);
        Sess { s, log, title, step: 0, printed: 0, names: Vec::new(), console: HWND::default() }
    }

    fn name(&self, h: HWND) -> String {
        if h.0.is_null() {
            return "none".into();
        }
        match self.names.iter().rev().find(|(n, _)| *n == h) {
            Some((_, name)) => format!("{:?}={name}", h.0),
            None => format!("{:?}", h.0),
        }
    }

    fn reparent(&mut self, target: HWND, name: &str) {
        if !target.0.is_null() {
            self.names.push((target, name.into()));
        }
        let module = conpty::load_shipped();
        let f: conpty::ReparentFn =
            unsafe { conpty::func(module, windows::core::s!("ConptyReparentPseudoConsole")) }.unwrap();
        let hr = unsafe { f(self.s.hpcon, target) };
        println!("  ConptyReparentPseudoConsole -> {name} ({:?}): hr={:#010x}", target.0, hr.0 as u32);
        sleep(800);
    }

    /// Tells the program to take its next step (log the console window, open a dialog).
    fn go(&mut self) {
        std::fs::write(format!("{}.go{}", self.log.display(), self.step), b"").unwrap();
        self.step += 1;
    }

    fn find_dialog(&self, wait_ms: u64) -> Option<HWND> {
        let title = util::wide(&self.title);
        for _ in 0..wait_ms / 100 {
            if let Ok(h) = unsafe { FindWindowW(w!("#32770"), PCWSTR(title.as_ptr())) } {
                if !h.is_invalid() {
                    return Some(h);
                }
            }
            sleep(100);
        }
        None
    }

    fn chain(&mut self, tag: &str) -> Option<HWND> {
        let dlg = self.find_dialog(5000);
        match dlg {
            None => println!("  [{tag}] NO DIALOG appeared within 5 s"),
            Some(dlg) => {
                let console = owner(dlg);
                let top = owner(console);
                self.console = console;
                println!(
                    "  [{tag}] dialog {:?} -> owner (console window) {:?} -> owner {}",
                    dlg.0,
                    console.0,
                    self.name(top)
                );
            }
        }
        dlg
    }

    fn dismiss(&mut self, dlg: HWND) {
        unsafe { PostMessageW(Some(dlg), WM_CLOSE, WPARAM(0), LPARAM(0)) }.ok();
        sleep(400);
    }

    /// One full step: the program opens a dialog, we read the chain and dismiss it.
    fn dialog_step(&mut self, tag: &str) {
        self.go();
        if let Some(dlg) = self.chain(tag) {
            self.dismiss(dlg);
        }
        self.child_log();
    }

    fn console_state(&self, tag: &str) {
        let c = self.console;
        if is_window(c) {
            println!(
                "  [{tag}] console window {:?}: IsWindow=true owner={} GWLP_HWNDPARENT={:#x}",
                c.0,
                self.name(owner(c)),
                hwndparent(c)
            );
        } else {
            println!("  [{tag}] console window {:?}: IsWindow=false", c.0);
        }
    }

    fn child_log(&mut self) {
        sleep(300);
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        for line in text.lines().skip(self.printed) {
            println!("      program: {line}");
            self.printed += 1;
        }
    }

    fn end(self) {
        unsafe { TerminateProcess(self.s.child, 0) }.ok();
        let hpcon = self.s.hpcon.0 as isize;
        // Close off-thread, as Brindle does: it can wait for the host.
        std::thread::spawn(move || {
            let module = conpty::load_shipped();
            let close: conpty::CloseFn =
                unsafe { conpty::func(module, windows::core::s!("ConptyClosePseudoConsole")) }.unwrap();
            unsafe { close(windows::Win32::System::Console::HPCON(hpcon as _)) };
        });
        sleep(500);
    }
}

/// `remote`: the owner is a window of another process. `how`: destroy | close | terminate.
fn scenario(tag: &str, remote: bool, how: &str, null_first: bool, dialog_open: bool) {
    println!(
        "== {tag}: owner in {} process, removed by {how}, {}, {}",
        if remote { "a THIRD" } else { "the holder's" },
        if null_first { "reparent to null FIRST" } else { "no reparent to null" },
        if dialog_open { "a dialog is OPEN at that moment" } else { "no dialog open" },
    );
    let mut o1 = Owner::make(remote);
    let mut s = Sess::new();
    s.reparent(o1.hwnd, "owner1");
    let open = if dialog_open {
        s.go();
        s.chain("owner is owner1, dialog left open")
    } else {
        s.dialog_step("owner is owner1");
        None
    };
    if null_first {
        s.reparent(HWND::default(), "null");
        s.console_state("after reparent to null");
    }
    o1.remove(how);
    sleep(500);
    println!("  IsWindow(owner1)={}", is_window(o1.hwnd));
    s.console_state("after owner1 is gone");
    if let Some(dlg) = open {
        println!("  open dialog {:?}: IsWindow={}", dlg.0, is_window(dlg));
        if is_window(dlg) {
            s.dismiss(dlg);
        }
    }
    s.child_log();
    s.dialog_step("next dialog, owner1 gone");

    let mut o2 = Owner::make(remote);
    s.reparent(o2.hwnd, "owner2");
    s.console_state("after reparent to owner2");
    s.dialog_step("after reparent to owner2");
    s.end();
    o2.remove(if remote { "terminate" } else { "destroy" });
    println!();
}

pub fn run(args: &[String]) {
    let all: &[(&str, bool, &str, bool, bool)] = &[
        ("a1", false, "destroy", false, false),
        ("a2", false, "destroy", false, true),
        ("b0", true, "none", false, false),
        ("b1", true, "close", false, false),
        ("b2", true, "terminate", false, false),
        ("b3", true, "terminate", false, true),
        ("c1", false, "destroy", true, false),
        ("c2", false, "destroy", true, true),
        ("c3", true, "terminate", true, false),
    ];
    for (tag, remote, how, null_first, dialog_open) in all {
        if !args.is_empty() && !args.iter().any(|a| a == tag) {
            continue;
        }
        if *how == "none" {
            cross_process_only();
        } else {
            scenario(tag, *remote, how, *null_first, *dialog_open);
        }
    }
}

/// (b) on its own: holder A owns the pseudoconsole, B and then B' own the windows.
fn cross_process_only() {
    println!("== b0: owner in a THIRD process, nothing removed; move between two such processes");
    let mut b1 = Owner::make(true);
    let mut b2 = Owner::make(true);
    let mut s = Sess::new();
    s.reparent(b1.hwnd, "B");
    s.dialog_step("owner is B");
    s.reparent(b2.hwnd, "B'");
    s.dialog_step("owner is B'");
    s.end();
    b1.remove("terminate");
    b2.remove("terminate");
    println!();
}

/// Process B: one hidden top-level window; exits when the window is gone.
pub fn run_owner_window() {
    let h = make_window(w!("BrindleSpikeRemoteOwner"));
    println!("{}", h.0 as usize);
    while unsafe { IsWindow(Some(h)) }.as_bool() {
        pump();
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn run_dialog_ext(args: &[String]) {
    use std::io::Write as _;
    let log = &args[0];
    let title = util::wide(&args[1]);
    let append = |line: String| {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{line}");
        }
    };
    for i in 0..10 {
        let go = format!("{log}.go{i}");
        let mut waited = 0;
        while !std::path::Path::new(&go).exists() {
            std::thread::sleep(Duration::from_millis(50));
            waited += 1;
            if waited > 1200 {
                return;
            }
        }
        let _ = std::fs::remove_file(&go);
        let console = unsafe { GetConsoleWindow() };
        let alive = !console.0.is_null() && unsafe { IsWindow(Some(console)) }.as_bool();
        let own = unsafe { GetWindow(console, GW_OWNER) }.unwrap_or_default();
        append(format!(
            "step {i}: GetConsoleWindow()={:?} IsWindow={alive} owner={:?}",
            console.0, own.0
        ));
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let ret = unsafe { MessageBoxW(Some(console), w!("spike dialog"), PCWSTR(title.as_ptr()), MB_OK) };
        let err = unsafe { GetLastError() };
        append(format!("step {i}: MessageBoxW returned {} (last error {})", ret.0, err.0));
    }
}
