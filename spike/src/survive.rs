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
use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, QueryInformationJobObject,
    SetInformationJobObject, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP,
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS, PROCESS_INFORMATION,
    PROCESS_CREATION_FLAGS, STARTUPINFOW,
};

use crate::pcon;

/// Whether this process is in a job, and that job's breakaway limits.
/// (With nested jobs this reads the innermost one.)
pub fn job_info() -> String {
    let mut in_job = windows::core::BOOL(0);
    // SAFETY: querying ourselves against any job.
    let asked = unsafe { IsProcessInJob(GetCurrentProcess(), None, &mut in_job) };
    if asked.is_err() {
        return format!("IsProcessInJob failed: {asked:?}");
    }
    if !in_job.as_bool() {
        return "in_job=false".into();
    }
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: a null job handle means the caller's job.
    let got = unsafe {
        QueryInformationJobObject(
            None,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            None,
        )
    };
    if let Err(e) = got {
        return format!("in_job=true QueryInformationJobObject failed: {e}");
    }
    let f = info.BasicLimitInformation.LimitFlags;
    format!(
        "in_job=true limit_flags={:#x} breakaway_ok={} silent_breakaway_ok={} kill_on_job_close={}",
        f.0,
        f.contains(JOB_OBJECT_LIMIT_BREAKAWAY_OK),
        f.contains(JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK),
        f.contains(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
    )
}

/// This process's ancestors, nearest first, as `name(pid)`; a dead parent shows as `?(pid)`.
fn ancestors() -> String {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let mut procs = std::collections::HashMap::new();
    // SAFETY: a process snapshot, walked with a correctly sized entry.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).expect("snapshot");
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(0);
            procs.insert(e.th32ProcessID, (e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        CloseHandle(snap).ok();
    }
    let mut out = Vec::new();
    let mut pid = std::process::id();
    for _ in 0..5 {
        let Some((parent, _)) = procs.get(&pid) else { break };
        match procs.get(parent) {
            Some((_, name)) => out.push(format!("{name}({parent})")),
            None => {
                out.push(format!("?({parent})"));
                break;
            }
        }
        pid = *parent;
    }
    out.join(" <- ")
}

static LOG: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Prints, and appends to `--log FILE` (launches from Explorer or wt have no stdout to read).
fn say(line: String) {
    use std::io::Write as _;
    println!("{line}");
    if let Some(path) = LOG.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

macro_rules! println {
    ($($t:tt)*) => { say(format!($($t)*)) };
}

/// `survive <mode> [--log FILE] [--stay SECS]`
pub fn run_parent(args: &[String]) {
    let mode = args.first().map(String::as_str).unwrap_or("none");
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if let Some(path) = opt("--log") {
        let _ = LOG.set(path);
    }
    let stay: u64 = opt("--stay").and_then(|s| s.parse().ok()).unwrap_or(0);
    println!("[P] pid={} mode={mode}", std::process::id());
    println!("[P] ancestors: {}", ancestors());
    println!("[P] before any job of our own: {}", job_info());
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
            if breakaway && e.code() == E_ACCESSDENIED {
                // The design's fallback: retry without breakaway.
                let mut wide: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
                // SAFETY: as above.
                let again = unsafe {
                    CreateProcessW(
                        PCWSTR::null(),
                        Some(PWSTR(wide.as_mut_ptr())),
                        None,
                        None,
                        false,
                        flags & !CREATE_BREAKAWAY_FROM_JOB,
                        None,
                        PCWSTR::null(),
                        &startup,
                        &mut info,
                    )
                };
                match again {
                    Ok(()) => println!(
                        "[P] retry WITHOUT breakaway OK: helper_pid={}",
                        info.dwProcessId
                    ),
                    Err(e) => println!("[P] retry WITHOUT breakaway FAILED: {e}"),
                }
                for _ in 0..50 {
                    if let Ok(s) = std::fs::read_to_string(&marker) {
                        if !s.trim().is_empty() {
                            println!("[P] marker: {}", s.trim());
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }
    if stay > 0 {
        println!("[P] staying {stay} s");
        std::thread::sleep(std::time::Duration::from_secs(stay));
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
    let session = pcon::spawn("cmd.exe /c ping -n 90 127.0.0.1 > NUL", 80, 25);
    std::fs::write(
        marker,
        format!(
            "helper_pid={} child_pid={} helper: {}",
            std::process::id(),
            session.pid,
            job_info()
        ),
    )
    .ok();
    // Keep the HPCON alive for a while so the harness can observe survival.
    std::thread::sleep(std::time::Duration::from_secs(80));
}
