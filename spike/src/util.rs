//! Tiny helpers shared by the experiments.

use windows::core::PCWSTR;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A writable UTF-16 command line for CreateProcessW.
pub fn cmdline(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[allow(dead_code)]
pub fn pcwstr(buf: &[u16]) -> PCWSTR {
    PCWSTR(buf.as_ptr())
}
