//! Create a pseudoconsole with the shipped ConPTY and start a program on it,
//! mirroring `src/terminal/conpty.rs::spawn` closely enough for the spike.

use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::WriteFile;
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    UpdateProcThreadAttribute, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::conpty;

const WCSWIDTH: u32 = 0x10;

pub struct Session {
    pub hpcon: HPCON,
    pub input_write: HANDLE,
    pub output_read: HANDLE,
    pub child: HANDLE,
    pub pid: u32,
}

/// Creates a pseudoconsole and starts `cmdline` on it.
pub fn spawn(cmdline: &str, cols: i16, rows: i16) -> Session {
    let module = conpty::load_shipped();
    // SAFETY: real export of the stated type.
    let create: conpty::CreateFn =
        unsafe { conpty::func(module, windows::core::s!("ConptyCreatePseudoConsole")) }.unwrap();

    let (input_read, input_write) = pipe();
    let (output_read, output_write) = pipe();
    let mut hpcon = HPCON::default();
    // SAFETY: pipe handles valid; conpty dups what it keeps.
    unsafe {
        create(COORD { X: cols, Y: rows }, input_read, output_write, WCSWIDTH, &mut hpcon)
    }
    .ok()
    .expect("ConptyCreatePseudoConsole");
    // SAFETY: our ends of the pipes conpty duplicated; close ours.
    unsafe {
        CloseHandle(input_read).ok();
        CloseHandle(output_write).ok();
    }

    let (pid, child) = start_on(hpcon, cmdline);
    Session { hpcon, input_write, output_read, child, pid }
}

fn pipe() -> (HANDLE, HANDLE) {
    let (mut r, mut w) = (HANDLE::default(), HANDLE::default());
    // SAFETY: fills both handles; not inheritable.
    unsafe { CreatePipe(&mut r, &mut w, None, 0) }.expect("CreatePipe");
    (r, w)
}

pub fn start_on(hpcon: HPCON, cmdline: &str) -> (u32, HANDLE) {
    let mut size = 0usize;
    // SAFETY: first call only reports the needed size.
    unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) }.ok();
    let mut buf = vec![0u8; size];
    let list = LPPROC_THREAD_ATTRIBUTE_LIST(buf.as_mut_ptr().cast());
    // SAFETY: buf sized as reported, outlives the list.
    unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }.expect("init attr list");
    // SAFETY: the attribute value is the HPCON itself.
    unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            Some(hpcon.0 as *const c_void),
            std::mem::size_of::<HPCON>(),
            None,
            None,
        )
    }
    .expect("UpdateProcThreadAttribute");

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = list;

    let mut line: Vec<u16> = cmdline.encode_utf16().chain(Some(0)).collect();
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: pointers valid for the call; the line buffer is writable.
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &startup.StartupInfo,
            &mut info,
        )
    }
    .expect("CreateProcessW");
    // SAFETY: initialized above, delete once; thread handle unused.
    unsafe {
        DeleteProcThreadAttributeList(list);
        CloseHandle(info.hThread).ok();
    }
    (info.dwProcessId, info.hProcess)
}

/// Writes bytes to the pseudoconsole input (its stdin).
#[allow(dead_code)]
pub fn write_input(h: HANDLE, bytes: &[u8]) {
    let mut done = 0u32;
    // SAFETY: handle open; buffer outlives the call.
    unsafe { WriteFile(h, Some(bytes), Some(&mut done), None) }.ok();
}
