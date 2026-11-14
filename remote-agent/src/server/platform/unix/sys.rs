//! The running binary and random numbers.

use std::fs;
use std::io;

/// The bytes of the running binary, from `/proc/self/exe`, which still
/// names it after an install replaces the file.
pub fn read_own_binary() -> io::Result<Vec<u8>> {
    fs::read("/proc/self/exe")
}

/// A random number from the kernel, if it gives one.
pub fn random_u64() -> Option<u64> {
    let mut bytes = [0u8; 8];
    let read = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    (read == bytes.len() as isize).then(|| u64::from_ne_bytes(bytes))
}
