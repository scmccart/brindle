//! 0.3: does a detached, broken-away helper outlive its parent, including
//! when the parent lives inside a job that forbids breakaway?
//!
//! `survive <mode>` plays Brindle ("P"). It spawns the helper with
//! DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB, optionally
//! after putting itself in a job. It prints whether the spawn succeeded and
//! the helper/child pids, then exits. The harness then checks the pids.
//!
//! modes: none | job | job-breakaway-ok | job-killonclose

use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP,
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS, PROCESS_INFORMATION,
    PROCESS_CREATION_FLAGS, STARTUPINFOW,
};

use crate::pcon;

pub fn run_parent(args: &[String]) {
    let mode = args.first().map(String::as_str).unwrap_or("none");
    let breakaway = mode != "job-killonclose"; // killonclose test spawns WITHOUT breakaway

    // Optionally place P in a job.
    let mut job: Option<HANDLE> = None;
    if mode.starts_with("job") {
        // SAFETY: creates an unnamed job object.
        let j = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.expect("CreateJobObject");
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        if mode == "job-breakaway-ok" {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        }
        if mode == "job-killonclose" {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        }
        // SAFETY: info outlives the call; size matches.
        let set = unsafe {
            SetInformationJobObject(
                j,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        println!("[P] SetInformationJobObject ok={}", set.is_ok());
        // SAFETY: assigning ourselves to the job.
        let assigned = unsafe { AssignProcessToJobObject(j, GetCurrentProcess()) };
        println!("[P] AssignProcessToJobObject(self) ok={} (mode={mode})", assigned.is_ok());
        if assigned.is_err() {
            println!("[P] NOTE: could not nest into a new job; result below may reflect the outer job");
        }
        job = Some(j);
    }

    let marker = std::env::temp_dir().join(format!("spike-survive-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let exe = std::env::current_exe().unwrap();
    let line = format!("\"{}\" survive-helper \"{}\"", exe.display(), marker.display());

    let mut flags: PROCESS_CREATION_FLAGS =
        DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT;
    if breakaway {
        flags |= CREATE_BREAKAWAY_FROM_JOB;
    }

    let mut wide: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
    let mut info = PROCESS_INFORMATION::default();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: writable command line; no inherited handles.
    let created = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(wide.as_mut_ptr())),
            None,
            None,
            false,
            flags,
            None,
            PCWSTR::null(),
            &startup,
            &mut info,
        )
    };
    match created {
        Ok(()) => {
            println!(
                "[P] CreateProcess(helper) OK: breakaway={breakaway} helper_pid={}",
                info.dwProcessId
            );
            // SAFETY: our handles, unused.
            unsafe {
                CloseHandle(info.hThread).ok();
                CloseHandle(info.hProcess).ok();
            }
            // Wait briefly for the helper to write its marker (pids).
            for _ in 0..50 {
                if let Ok(s) = std::fs::read_to_string(&marker) {
                    if !s.trim().is_empty() {
                        println!("[P] marker: {}", s.trim());
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            println!("[P] marker_path={}", marker.display());
        }
        Err(e) => {
            println!("[P] CreateProcess(helper) FAILED: breakaway={breakaway} err={e}");
        }
    }
    if let Some(j) = job {
        // For job-killonclose, closing the job handle here (as P exits) should
        // kill everything still in the job. The harness checks after.
        // SAFETY: our job handle.
        unsafe { CloseHandle(j).ok() };
    }
    println!("[P] exiting now");
}

pub fn run_helper(args: &[String]) {
    let marker = &args[0];
    // A long-lived program on a real pseudoconsole.
    let session = pcon::spawn("cmd.exe /c ping -n 30 127.0.0.1 > NUL", 80, 25);
    std::fs::write(marker, format!("helper_pid={} child_pid={}", std::process::id(), session.pid)).ok();
    // Keep the HPCON alive for a while so the harness can observe survival.
    std::thread::sleep(std::time::Duration::from_secs(25));
}
