//! 0.3 follow-ups: the control pipe's access model across integrity levels.
//!
//!   access run <server-il> <label> <client-il>:<mode>...
//!       starts `access-server` at <server-il> (self | low | medium), which
//!       creates the pipe with `D:P(A;;GA;;;<user>)(A;;GA;;;SY)` plus the
//!       mandatory label <label> (hi | me | lw | none), then runs each client
//!       (mode rw | r | w) at its integrity level and prints what each side saw.
//!   access first <child-il>
//!       this process creates a pipe; a second process then calls
//!       CreateNamedPipeW on the same name with and without
//!       FILE_FLAG_FIRST_PIPE_INSTANCE.
//!
//! Lower-integrity processes are made from a duplicate of our own token with
//! TokenIntegrityLevel lowered, started with CreateProcessAsUserW.

use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, SetHandleInformation, GENERIC_READ, GENERIC_WRITE, HANDLE,
    HANDLE_FLAGS, HANDLE_FLAG_INHERIT, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows::Win32::Security::{
    DuplicateTokenEx, EqualSid, GetLengthSid, GetSidSubAuthority, GetSidSubAuthorityCount,
    GetTokenInformation, RevertToSelf, SecurityImpersonation, SetTokenInformation,
    TokenImpersonationLevel, TokenIntegrityLevel, TokenPrimary, TokenUser,
    DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, SID_AND_ATTRIBUTES, TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_MODE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, CreatePipe, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    ImpersonateNamedPipeClient, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, GetCurrentProcess, GetCurrentThread, OpenProcess,
    OpenProcessToken, OpenThreadToken, TerminateProcess, WaitForSingleObject, DETACHED_PROCESS,
    INFINITE, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOW,
};

use crate::util::wide;

fn win32(e: &windows::core::Error) -> u32 {
    (e.code().0 as u32) & 0xffff
}

fn last() -> u32 {
    unsafe { GetLastError() }.0
}

// ---- tokens ----

fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<u8>, u32> {
    let mut len = 0u32;
    let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut len) };
    let mut buf = vec![0u8; len.max(8) as usize];
    unsafe { GetTokenInformation(token, class, Some(buf.as_mut_ptr().cast()), buf.len() as u32, &mut len) }
        .map_err(|e| win32(&e))?;
    Ok(buf)
}

fn il_name(rid: u32) -> String {
    match rid {
        0x1000 => "low".into(),
        0x2000 => "medium".into(),
        0x2100 => "medium-plus".into(),
        0x3000 => "high".into(),
        0x4000 => "system".into(),
        other => format!("{other:#x}"),
    }
}

/// `user=<SID> il=<level>` for a token, or the failing step's error code.
fn describe(token: HANDLE) -> String {
    let user = token_info(token, TokenUser).map(|buf| {
        let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
        sid_string(sid)
    });
    let il = token_info(token, TokenIntegrityLevel).map(|buf| {
        let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid };
        let n = unsafe { *GetSidSubAuthorityCount(sid) } as u32;
        il_name(unsafe { *GetSidSubAuthority(sid, n - 1) })
    });
    let same = token_info(token, TokenUser)
        .map(|buf| {
            let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
            let (_, own) = own_user();
            unsafe { EqualSid(sid, PSID(own.as_ptr() as *mut c_void)) }.is_ok()
        })
        .unwrap_or(false);
    format!(
        "TokenUser={} (same as ours: {same}) TokenIntegrityLevel={}",
        user.unwrap_or_else(|e| format!("error {e}")),
        il.unwrap_or_else(|e| format!("error {e}"))
    )
}

fn sid_string(sid: PSID) -> String {
    let mut s = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut s) }.expect("ConvertSidToStringSid");
    let out = unsafe { s.to_string() }.unwrap();
    unsafe { LocalFree(Some(HLOCAL(s.0 as *mut c_void))) };
    out
}

fn own_token() -> HANDLE {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.expect("OpenProcessToken");
    token
}

/// Our user SID, read once: while impersonating, our own process token may not open.
fn own_user() -> (String, Vec<u8>) {
    static OWN: std::sync::OnceLock<(String, Vec<u8>)> = std::sync::OnceLock::new();
    OWN.get_or_init(read_own_user).clone()
}

fn read_own_user() -> (String, Vec<u8>) {
    let token = own_token();
    let buf = token_info(token, TokenUser).expect("TokenUser");
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let len = unsafe { GetLengthSid(sid) } as usize;
    let raw = unsafe { std::slice::from_raw_parts(sid.0 as *const u8, len) }.to_vec();
    let s = sid_string(sid);
    unsafe { CloseHandle(token).ok() };
    (s, raw)
}

fn own_il() -> String {
    let token = own_token();
    let d = describe(token);
    unsafe { CloseHandle(token).ok() };
    d.rsplit('=').next().unwrap().to_string()
}

// ---- starting a process at an integrity level ----

struct Proc {
    process: HANDLE,
    output: std::thread::JoinHandle<String>,
}

impl Proc {
    fn wait(self) -> String {
        unsafe { WaitForSingleObject(self.process, INFINITE) };
        self.output.join().unwrap()
    }
    fn kill(self) -> String {
        unsafe { TerminateProcess(self.process, 0) }.ok();
        self.wait()
    }
}

/// Runs `spike <args>` with its output captured; `il` is self, low or medium.
fn start(args: &[&str], il: &str) -> Proc {
    let exe = std::env::current_exe().unwrap();
    let mut line: Vec<u16> = wide(&format!("\"{}\" {}", exe.display(), args.join(" ")));

    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut read, &mut write, Some(&sa), 0) }.expect("CreatePipe");
    unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }.expect("no inherit");

    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdOutput: write,
        hStdError: write,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    if il == "self" {
        unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                true,
                DETACHED_PROCESS,
                None,
                PCWSTR::null(),
                &startup,
                &mut info,
            )
        }
        .expect("CreateProcessW");
    } else {
        let rid = match il {
            "low" => "S-1-16-4096",
            "medium" => "S-1-16-8192",
            other => panic!("unknown integrity level {other}"),
        };
        let mut own = HANDLE::default();
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT | TOKEN_ASSIGN_PRIMARY,
                &mut own,
            )
        }
        .expect("OpenProcessToken");
        let mut token = HANDLE::default();
        unsafe {
            DuplicateTokenEx(own, TOKEN_ACCESS_MASK(0x0200_0000), None, SecurityImpersonation, TokenPrimary, &mut token)
        }
        .expect("DuplicateTokenEx");
        let mut sid = PSID::default();
        unsafe { ConvertStringSidToSidW(PCWSTR(wide(rid).as_ptr()), &mut sid) }.expect("label sid");
        let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: sid, Attributes: 0x20 } };
        unsafe {
            SetTokenInformation(
                token,
                TokenIntegrityLevel,
                &label as *const _ as *const c_void,
                std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid),
            )
        }
        .expect("SetTokenInformation(TokenIntegrityLevel)");
        unsafe {
            CreateProcessAsUserW(
                Some(token),
                PCWSTR::null(),
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                true,
                DETACHED_PROCESS,
                None,
                PCWSTR::null(),
                &startup,
                &mut info,
            )
        }
        .expect("CreateProcessAsUserW");
        unsafe {
            CloseHandle(token).ok();
            CloseHandle(own).ok();
        }
    }
    unsafe {
        CloseHandle(write).ok();
        CloseHandle(info.hThread).ok();
    }
    let read = read.0 as isize;
    let output = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let mut n = 0u32;
            let r = unsafe { ReadFile(HANDLE(read as *mut c_void), Some(&mut buf), Some(&mut n), None) };
            if r.is_err() || n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        String::from_utf8_lossy(&out).into_owned()
    });
    Proc { process: info.hProcess, output }
}

fn show(who: &str, text: &str) {
    for line in text.lines() {
        println!("  {who}: {line}");
    }
}

// ---- the pipe ----

fn sddl(label: &str) -> String {
    let (sid, _) = own_user();
    let sacl = match label {
        "hi" => "S:(ML;;NW;;;HI)",
        "me" => "S:(ML;;NW;;;ME)",
        "lw" => "S:(ML;;NW;;;LW)",
        _ => "",
    };
    format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY){sacl}")
}

/// CreateNamedPipeW with the given SDDL; Err is GetLastError.
fn create_pipe(name: &str, sddl: &str, first: bool, max: u32) -> Result<HANDLE, u32> {
    let mut psd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide(sddl).as_ptr()),
            SDDL_REVISION_1,
            &mut psd,
            None,
        )
    }
    .map_err(|e| win32(&e))?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: psd.0,
        bInheritHandle: false.into(),
    };
    let mut open = PIPE_ACCESS_DUPLEX;
    if first {
        open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let h = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide(name).as_ptr()),
            open,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            max,
            4096,
            4096,
            0,
            Some(&sa),
        )
    };
    let err = last();
    unsafe { LocalFree(Some(HLOCAL(psd.0))) };
    if h == INVALID_HANDLE_VALUE {
        Err(err)
    } else {
        Ok(h)
    }
}

/// The pipe's DACL and mandatory label as the kernel holds them.
fn actual_sddl(pipe: HANDLE) -> String {
    let mut psd = PSECURITY_DESCRIPTOR::default();
    let info = DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    let r = unsafe { GetSecurityInfo(pipe, SE_KERNEL_OBJECT, info, None, None, None, None, Some(&mut psd)) };
    if r.0 != 0 {
        return format!("GetSecurityInfo error {}", r.0);
    }
    let mut s = PWSTR::null();
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(psd, SDDL_REVISION_1, info, &mut s, None)
    };
    let out = match ok {
        Ok(()) => unsafe { s.to_string() }.unwrap(),
        Err(e) => format!("convert error {}", win32(&e)),
    };
    unsafe {
        LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
        LocalFree(Some(HLOCAL(psd.0)));
    }
    out
}

fn impersonate(pipe: HANDLE, when: &str) {
    match unsafe { ImpersonateNamedPipeClient(pipe) } {
        Err(e) => println!("ImpersonateNamedPipeClient {when}: FAILED error {}", win32(&e)),
        Ok(()) => {
            let mut token = HANDLE::default();
            let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) };
            let what = match opened {
                Err(e) => format!("OpenThreadToken error {}", win32(&e)),
                Ok(()) => {
                    let level = token_info(token, TokenImpersonationLevel)
                        .map(|b| u32::from_ne_bytes(b[..4].try_into().unwrap()))
                        .map(|l| match l {
                            0 => "Anonymous".to_string(),
                            1 => "Identification".to_string(),
                            2 => "Impersonation".to_string(),
                            3 => "Delegation".to_string(),
                            n => n.to_string(),
                        })
                        .unwrap_or_else(|e| format!("error {e}"));
                    let d = describe(token);
                    unsafe { CloseHandle(token).ok() };
                    format!("level={level} {d}")
                }
            };
            let reverted = unsafe { RevertToSelf() }.is_ok();
            println!("ImpersonateNamedPipeClient {when}: OK; thread token {what}; RevertToSelf ok={reverted}");
        }
    }
}

fn peer_check(pipe: HANDLE) {
    let mut pid = 0u32;
    if let Err(e) = unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) } {
        println!("peer check: GetNamedPipeClientProcessId FAILED error {}", win32(&e));
        return;
    }
    let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(p) => p,
        Err(e) => {
            println!("peer check: pid={pid}; OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) FAILED error {}", win32(&e));
            return;
        }
    };
    let mut token = HANDLE::default();
    match unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } {
        Err(e) => println!("peer check: pid={pid}; OpenProcess OK; OpenProcessToken(TOKEN_QUERY) FAILED error {}", win32(&e)),
        Ok(()) => {
            println!("peer check: pid={pid}; OpenProcess OK; OpenProcessToken OK; {}", describe(token));
            unsafe { CloseHandle(token).ok() };
        }
    }
    unsafe { CloseHandle(process).ok() };
}

pub fn run_server(args: &[String]) {
    let (name, label) = (&args[0], &args[1]);
    println!("pid={} integrity={}", std::process::id(), own_il());
    own_user();
    let sddl = sddl(label);
    let pipe = match create_pipe(name, &sddl, true, 4) {
        Ok(h) => h,
        Err(e) => {
            println!("CreateNamedPipeW with {sddl}: FAILED error {e}");
            return;
        }
    };
    println!("CreateNamedPipeW with {sddl}: OK");
    println!("pipe security as stored: {}", actual_sddl(pipe));
    loop {
        if let Err(e) = unsafe { ConnectNamedPipe(pipe, None) } {
            if win32(&e) != 535 {
                println!("ConnectNamedPipe error {}", win32(&e));
                return;
            }
        }
        println!("-- client connected; no data read yet");
        impersonate(pipe, "before any read");
        peer_check(pipe);
        let mut byte = [0u8; 1];
        let mut n = 0u32;
        match unsafe { ReadFile(pipe, Some(&mut byte), Some(&mut n), None) } {
            Ok(()) => println!("ReadFile: {n} byte(s)"),
            Err(e) => println!("ReadFile: error {} (client sent nothing)", win32(&e)),
        }
        impersonate(pipe, "after the read");
        unsafe { DisconnectNamedPipe(pipe) }.ok();
    }
}

pub fn run_client(args: &[String]) {
    let (name, mode) = (&args[0], args[1].as_str());
    println!("pid={} integrity={} opening for {mode}", std::process::id(), own_il());
    let access = match mode {
        "rw" => GENERIC_READ.0 | GENERIC_WRITE.0,
        "r" => GENERIC_READ.0,
        _ => GENERIC_WRITE.0,
    };
    let mut tries = 0;
    let pipe = loop {
        let r = unsafe {
            CreateFileW(
                PCWSTR(wide(name).as_ptr()),
                access,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        };
        match r {
            Ok(h) => break h,
            Err(e) if win32(&e) == 231 && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                println!("CreateFileW: FAILED error {}", win32(&e));
                return;
            }
        }
    };
    println!("CreateFileW: OK");
    // Give the server time to run its checks before any data exists.
    std::thread::sleep(std::time::Duration::from_millis(1000));
    if mode != "r" {
        let mut n = 0u32;
        match unsafe { WriteFile(pipe, Some(b"x"), Some(&mut n), None) } {
            Ok(()) => println!("WriteFile: {n} byte(s)"),
            Err(e) => println!("WriteFile: error {}", win32(&e)),
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    unsafe { CloseHandle(pipe).ok() };
}

pub fn run_first_child(args: &[String]) {
    let name = &args[0];
    println!("pid={} integrity={}", std::process::id(), own_il());
    let sddl = sddl("none");
    for (first, what) in [(true, "WITH FILE_FLAG_FIRST_PIPE_INSTANCE"), (false, "without the flag")] {
        match create_pipe(name, &sddl, first, 4) {
            Ok(h) => {
                println!("CreateNamedPipeW {what}: OK (a second instance of the other process's pipe)");
                unsafe { CloseHandle(h).ok() };
            }
            Err(e) => println!("CreateNamedPipeW {what}: FAILED error {e}"),
        }
    }
}

pub fn run(args: &[String]) {
    println!("orchestrator pid={} integrity={}", std::process::id(), own_il());
    match args.first().map(String::as_str) {
        Some("run") => {
            let (server_il, label) = (args[1].as_str(), args[2].as_str());
            let name = format!(r"\\.\pipe\brindle-spike-access-{}", std::process::id());
            println!("== server at {server_il}, label {label}");
            let server = start(&["access-server", &name, label], server_il);
            std::thread::sleep(std::time::Duration::from_millis(800));
            for spec in &args[3..] {
                let (il, mode) = spec.split_once(':').expect("client spec il:mode");
                let out = start(&["access-client", &name, mode], il).wait();
                show(&format!("client {spec}"), &out);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            show("server", &server.kill());
        }
        Some("first") => {
            let child_il = args.get(1).map(String::as_str).unwrap_or("self");
            for max in [4u32, 1] {
                let name = format!(r"\\.\pipe\brindle-spike-first-{}-{max}", std::process::id());
                let held = create_pipe(&name, &sddl("none"), true, max).expect("first instance");
                println!("== this process holds the first instance (nMaxInstances={max}); a {child_il} process tries the same name");
                show("other process", &start(&["access-first", &name], child_il).wait());
                unsafe { CloseHandle(held).ok() };
            }
        }
        _ => eprintln!("usage: spike access run <server-il> <label> <il:mode>... | access first [il]"),
    }
}
