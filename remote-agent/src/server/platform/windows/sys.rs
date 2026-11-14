//! The running binary and random numbers.

use std::fs;
use std::io;

/// The bytes of the running binary.
pub fn read_own_binary() -> io::Result<Vec<u8>> {
    fs::read(std::env::current_exe()?)
}

/// A random number from the system, if it gives one.
pub fn random_u64() -> Option<u64> {
    None
}
