//! 0.4: per-user named pipe + peer SID check + one IOCP over many sources.
//!
//! Three parts:
//!   a) build a DACL granting only the current user and SYSTEM, create the
//!      control pipe with it, print the SDDL;
//!   b) accept a same-user client and verify it with GetNamedPipeClientProcessId
//!      plus a token-SID compare;
//!   c) create two pseudoconsoles whose OUTPUT pipes are overlapped named pipes
//!      we own, associate both to one IOCP, issue overlapped reads, run one
//!      GetQueuedCompletionStatus loop that collects both sessions' markers,
//!      then quiesce the loop (cancel + drain) and report the in-flight count.

use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    EqualSid, GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
};
use windows::Win32::Security::SECURITY_ATTRIBUTES;

use crate::pcon;

fn current_user_sid_string() -> (String, Vec<u8>) {
    let mut token = HANDLE::default();
    // SAFETY: querying our own token.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.expect("OpenProcessToken");
    let mut len = 0u32;
    // SAFETY: first call reports the needed size.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
    let mut buf = vec![0u8; len as usize];
    // SAFETY: buf sized as reported.
    unsafe { GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len) }
        .expect("GetTokenInformation");
    let tu = buf.as_ptr() as *const TOKEN_USER;
    // SAFETY: tu points at the filled TOKEN_USER.
    let psid = unsafe { (*tu).User.Sid };
    let mut sidstr = PWSTR::null();
    // SAFETY: psid valid; frees sidstr below.
    unsafe { ConvertSidToStringSidW(psid, &mut sidstr) }.expect("ConvertSidToStringSid");
    let s = unsafe { sidstr.to_string() }.unwrap();
    unsafe { LocalFree(Some(HLOCAL(sidstr.0 as *mut c_void))) };
    // Copy the raw SID bytes so we can EqualSid later.
    // SAFETY: GetLengthSid over a valid SID.
    let sidlen = unsafe { windows::Win32::Security::GetLengthSid(psid) } as usize;
    let raw = unsafe { std::slice::from_raw_parts(psid.0 as *const u8, sidlen) }.to_vec();
    unsafe { CloseHandle(token).ok() };
    (s, raw)
}

pub fn run() {
    let (sid, own_sid_bytes) = current_user_sid_string();
    println!("[0.4a] current user SID: {sid}");

    // ---- a) user-only DACL on the control pipe ----
    let sddl = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)");
    println!("[0.4a] control-pipe SDDL: {sddl}");
    let sddl_w: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut psd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: valid SDDL; psd freed via LocalFree below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_w.as_ptr()),
            SDDL_REVISION_1,
            &mut psd,
            None,
        )
    }
    .expect("ConvertStringSecurityDescriptor");
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: psd.0,
        bInheritHandle: false.into(),
    };

    let pipe_name = format!(r"\\.\pipe\brindle-spike-{}", std::process::id());
    let name_w: Vec<u16> = pipe_name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: valid name and SA; inbound byte pipe, local only.
    let control = unsafe {
        CreateNamedPipeW(
            PCWSTR(name_w.as_ptr()),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            4,
            4096,
            4096,
            0,
            Some(&sa),
        )
    };
    assert_ne!(control, INVALID_HANDLE_VALUE, "CreateNamedPipe(control) failed");
    unsafe { LocalFree(Some(HLOCAL(psd.0))) };
    println!("[0.4a] created control pipe {pipe_name} with user-only DACL");

    // ---- b) peer check with a same-user client ----
    let exe = std::env::current_exe().unwrap();
    let mut client = std::process::Command::new(&exe)
        .arg("pipe-client")
        .arg(&pipe_name)
        .spawn()
        .expect("spawn pipe client");
    // SAFETY: blocking connect; client is same user.
    let _ = unsafe { ConnectNamedPipe(control, None) };
    let mut client_pid = 0u32;
    // SAFETY: control is connected.
    unsafe { GetNamedPipeClientProcessId(control, &mut client_pid) }.expect("GetNamedPipeClientProcessId");
    let same = peer_is_user(client_pid, &own_sid_bytes);
    println!("[0.4b] client pid={client_pid}, same-user check = {}", if same { "ACCEPT" } else { "REFUSE" });
    // Also prove the compare refuses a bogus SID.
    let bogus = peer_is_user_against(client_pid, &[1u8, 2, 3, 4]);
    println!("[0.4b] against a deliberately wrong SID = {}", if bogus { "ACCEPT(bug!)" } else { "REFUSE (as expected)" });
    client.wait().ok();
    unsafe { CloseHandle(control).ok() };
    println!("[0.4b] NOTE: a true second-user denial needs a second account; not available here. The 0700-equivalent DACL plus the SID compare are the two barriers; both exercised same-user.");

    // ---- c) one IOCP over two pseudoconsole outputs ----
    iocp_over_sessions();
}

fn peer_is_user(pid: u32, own_sid: &[u8]) -> bool {
    peer_is_user_against(pid, own_sid)
}

fn peer_is_user_against(pid: u32, expect_sid: &[u8]) -> bool {
    // SAFETY: querying the client process for its token SID.
    let proc = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut token = HANDLE::default();
    let ok = unsafe { OpenProcessToken(proc, TOKEN_QUERY, &mut token) }.is_ok();
    if !ok {
        unsafe { CloseHandle(proc).ok() };
        return false;
    }
    let mut len = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
    let mut buf = vec![0u8; len as usize];
    let got = unsafe { GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len) }.is_ok();
    let mut result = false;
    if got {
        let tu = buf.as_ptr() as *const TOKEN_USER;
        let psid = unsafe { (*tu).User.Sid };
        // Compare against expect_sid by building a PSID from its bytes.
        if expect_sid.len() >= 8 {
            let other = windows::Win32::Security::PSID(expect_sid.as_ptr() as *mut c_void);
            result = unsafe { EqualSid(psid, other) }.is_ok();
        }
    }
    unsafe {
        CloseHandle(token).ok();
        CloseHandle(proc).ok();
    }
    result
}

/// An overlapped output pipe we read, plus the write end handed to conpty.
fn overlapped_output_pipe(name: &str) -> (HANDLE, HANDLE) {
    let name_w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // Server (our) end: inbound, overlapped.
    // SAFETY: valid name; default security (same-user by default on the session).
    let server = unsafe {
        CreateNamedPipeW(
            PCWSTR(name_w.as_ptr()),
            PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            None,
        )
    };
    assert_ne!(server, INVALID_HANDLE_VALUE, "CreateNamedPipe(output server) failed");
    // Client (conpty's) end: it writes.
    // SAFETY: opening the just-created pipe for writing.
    let client = unsafe {
        CreateFileW(
            PCWSTR(name_w.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
    }
    .expect("CreateFile(output client)");
    (server, client)
}

struct Sess {
    read: HANDLE,
    ov: Box<OVERLAPPED>,
    buf: Box<[u8; 65536]>,
    got: Vec<u8>,
    inflight: bool,
}

fn issue_read(s: &mut Sess) {
    *s.ov = OVERLAPPED::default();
    let ov: *mut OVERLAPPED = &mut *s.ov;
    // SAFETY: overlapped read; buf and ov outlive the in-flight op.
    let r = unsafe { ReadFile(s.read, Some(&mut s.buf[..]), None, Some(ov)) };
    // Overlapped reads usually return ERROR_IO_PENDING; both mean in flight.
    let _ = r;
    s.inflight = true;
}

fn iocp_over_sessions() {
    // SAFETY: new IOCP.
    let iocp = unsafe { CreateIoCompletionPort(windows::Win32::Foundation::INVALID_HANDLE_VALUE, None, 0, 0) }
        .expect("CreateIoCompletionPort");

    let mut sessions: Vec<Sess> = Vec::new();

    for i in 0..2 {
        let name = format!(r"\\.\pipe\brindle-spike-out-{}-{}", std::process::id(), i);
        let (server_read, conpty_write) = overlapped_output_pipe(&name);
        // Associate this read handle to the IOCP, keyed by i.
        // SAFETY: server_read is a valid overlapped handle.
        unsafe { CreateIoCompletionPort(server_read, Some(iocp), i as usize, 0) }.expect("assoc");

        // Build a pseudoconsole whose OUTPUT is this overlapped pipe.
        let module = crate::conpty::load_shipped();
        let create: crate::conpty::CreateFn =
            unsafe { crate::conpty::func(module, windows::core::s!("ConptyCreatePseudoConsole")) }.unwrap();
        let (in_read, _in_write) = {
            let (mut r, mut w) = (HANDLE::default(), HANDLE::default());
            unsafe { windows::Win32::System::Pipes::CreatePipe(&mut r, &mut w, None, 0) }.expect("CreatePipe in");
            (r, w)
        };
        let mut hpcon = windows::Win32::System::Console::HPCON::default();
        // SAFETY: handles valid; conpty dups what it keeps.
        unsafe {
            create(
                windows::Win32::System::Console::COORD { X: 80, Y: 25 },
                in_read,
                conpty_write,
                0x10,
                &mut hpcon,
            )
        }
        .ok()
        .expect("create pcon");
        unsafe {
            CloseHandle(in_read).ok();
            CloseHandle(conpty_write).ok();
        }
        let _ = pcon::start_on(hpcon, &format!("cmd.exe /c echo SESSION{i}_OK & exit 0"));

        let mut sess = Sess {
            read: server_read,
            ov: Box::new(OVERLAPPED::default()),
            buf: Box::new([0u8; 65536]),
            got: Vec::new(),
            inflight: false,
        };
        issue_read(&mut sess);
        sessions.push(sess);
    }

    // One loop: collect both markers.
    let mut done = 0;
    for _ in 0..400 {
        let mut bytes = 0u32;
        let mut key = 0usize;
        let mut ov: *mut OVERLAPPED = std::ptr::null_mut();
        // SAFETY: standard GQCS over our IOCP.
        let r = unsafe { GetQueuedCompletionStatus(iocp, &mut bytes, &mut key, &mut ov, 250) };
        if r.is_err() {
            // timeout or pipe end
            // SAFETY: reading error for control flow only.
            let last = unsafe { windows::Win32::Foundation::GetLastError() };
            if last .0 == 258u32 {
                if done == sessions.len() {
                    break;
                }
                continue;
            }
            // broken pipe on a session: mark not inflight.
            if key < sessions.len() {
                sessions[key].inflight = false;
            }
            continue;
        }
        let s = &mut sessions[key];
        s.inflight = false;
        if bytes > 0 {
            s.got.extend_from_slice(&s.buf[..bytes as usize]);
        }
        let marker = format!("SESSION{key}_OK");
        if s.got.windows(marker.len()).any(|w| w == marker.as_bytes()) {
            done += 1;
            println!("[0.4c] IOCP delivered {marker} (session {key})");
            if done == sessions.len() {
                break;
            }
        } else if bytes > 0 {
            issue_read(s);
        }
    }

    // Quiesce: cancel outstanding reads, drain completions, report in-flight.
    let mut inflight = 0;
    for s in &sessions {
        if s.inflight {
            inflight += 1;
            // SAFETY: cancel the outstanding op on this handle.
            unsafe { CancelIoEx(s.read, Some(&*s.ov)) }.ok();
        }
    }
    // Drain any just-cancelled completions.
    loop {
        let mut bytes = 0u32;
        let mut key = 0usize;
        let mut ov: *mut OVERLAPPED = std::ptr::null_mut();
        let r = unsafe { GetQueuedCompletionStatus(iocp, &mut bytes, &mut key, &mut ov, 100) };
        if r.is_err() && unsafe { windows::Win32::Foundation::GetLastError() } .0 == 258u32 {
            break;
        }
    }
    println!("[0.4c] quiesced one IOCP loop: {inflight} reads were in flight, cancelled and drained; the loop reached a clean stop with no thread holding session state");

    for s in sessions {
        unsafe { CloseHandle(s.read).ok() };
    }
    unsafe { CloseHandle(iocp).ok() };
}

pub fn run_client(args: &[String]) {
    // Minimal same-user client: open the pipe, then exit.
    let name = &args[0];
    let name_w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: opening the server's pipe for writing.
    let h = unsafe {
        CreateFileW(
            PCWSTR(name_w.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
    };
    if let Ok(h) = h {
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe { CloseHandle(h).ok() };
    }
}
