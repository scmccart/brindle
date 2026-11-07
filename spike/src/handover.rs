//! 0.2: can a second process take a live session over, exports only?
//!
//! The origin ("A") creates a pseudoconsole with the shipped ConPTY and runs a
//! program that prints a marker and exits 7. It then launches the taker ("B"),
//! passing A's pid and the raw handle values. B opens A with PROCESS_DUP_HANDLE
//! and `DuplicateHandle`s the input-write, output-read and child-process
//! handles into itself — nothing but documented calls, no struct layout.
//!
//! B then shows what survives the move: reading output and the exit code. It
//! also records that it has no `hSignal`/`hRef`/`hServerProcess`, so there is
//! no way to rebuild an HPCON (resize/reparent/close) via exports, since
//! `ConptyCreatePseudoConsole` never surfaced those three handles.

use std::io::Write as _;

use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, WaitForSingleObject, PROCESS_DUP_HANDLE,
};
use windows::Win32::Foundation::DuplicateHandle;
use windows::Win32::System::Threading::GetCurrentProcess;

use crate::pcon;

pub fn run_origin() {
    // A program that emits a marker and exits with a known code.
    let session = pcon::spawn("cmd.exe /c echo SPIKE_MARKER_OK & exit 7", 80, 25);
    println!("[A] pid={} child_pid={}", std::process::id(), session.pid);
    println!(
        "[A] handles input_write={} output_read={} child={}",
        session.input_write.0 as isize, session.output_read.0 as isize, session.child.0 as isize
    );

    // Launch the taker with our pid and the raw handle values.
    let exe = std::env::current_exe().unwrap();
    let mut child_b = std::process::Command::new(exe)
        .arg("handover-taker")
        .arg(std::process::id().to_string())
        .arg((session.input_write.0 as isize).to_string())
        .arg((session.output_read.0 as isize).to_string())
        .arg((session.child.0 as isize).to_string())
        .spawn()
        .expect("spawn taker");
    // Stay alive (keeping the source handles valid) until B is done.
    let status = child_b.wait().expect("wait taker");
    println!("[A] taker exited: {status}");
}

pub fn run_taker(args: &[String]) {
    let pid_a: u32 = args[0].parse().unwrap();
    let h_in: isize = args[1].parse().unwrap();
    let h_out: isize = args[2].parse().unwrap();
    let h_child: isize = args[3].parse().unwrap();

    // SAFETY: opening A for handle duplication.
    let a = unsafe { OpenProcess(PROCESS_DUP_HANDLE, false, pid_a) }.expect("OpenProcess(A)");
    let me = unsafe { GetCurrentProcess() };

    let dup = |raw: isize| -> HANDLE {
        let mut out = HANDLE::default();
        // SAFETY: a and me are open; raw is a valid handle in A for the life of A.
        unsafe {
            DuplicateHandle(a, HANDLE(raw as *mut _), me, &mut out, 0, false, DUPLICATE_SAME_ACCESS)
        }
        .expect("DuplicateHandle");
        out
    };
    let out_read = dup(h_out);
    let _in_write = dup(h_in);
    let child = dup(h_child);
    println!("[B] duplicated input_write, output_read and child handles out of A");

    // Data survives: read until the marker appears (or EOF).
    let mut seen = false;
    let mut buf = [0u8; 4096];
    let mut collected: Vec<u8> = Vec::new();
    for _ in 0..200 {
        let mut n = 0u32;
        // SAFETY: out_read is our duplicate; buf outlives the call.
        let r = unsafe { ReadFile(out_read, Some(&mut buf), Some(&mut n), None) };
        if r.is_err() || n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n as usize]);
        if collected.windows(b"SPIKE_MARKER_OK".len()).any(|w| w == b"SPIKE_MARKER_OK") {
            seen = true;
            break;
        }
    }
    println!("[B] output marker seen: {seen}");

    // Exit code survives via the duplicated process handle.
    // SAFETY: child is our duplicate of A's child process handle.
    let waited = unsafe { WaitForSingleObject(child, 5000) };
    let mut code = 0u32;
    // SAFETY: child handle open.
    let got = unsafe { GetExitCodeProcess(child, &mut code) }.is_ok();
    println!(
        "[B] child wait={} GetExitCodeProcess ok={} code={}",
        if waited == WAIT_OBJECT_0 { "signaled" } else { "timeout" },
        got,
        code
    );

    println!(
        "[B] HPCON NOT reconstructable via exports: have only I/O pipes + child; \
         ConptyPackPseudoConsole needs hServerProcess/hRef/hSignal, which \
         ConptyCreatePseudoConsole never surfaces and no export extracts."
    );
    std::io::stdout().flush().ok();
}
