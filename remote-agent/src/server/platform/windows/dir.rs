//! The server directory on Windows: to be `%BRINDLE_SERVER_DIR%`, else
//! `%LOCALAPPDATA%\brindle\run`, with a DACL for the user and SYSTEM.

use std::io;
use std::path::{Path, PathBuf};

use super::unsupported;

/// Where the server directory for this process's environment is, without
/// creating it.
pub fn resolve_from_env() -> io::Result<PathBuf> {
    Err(unsupported())
}

/// Creates `dir` (and missing parents) for this user only, or checks that
/// the existing one is.
pub fn ensure(_dir: &Path) -> io::Result<()> {
    Err(unsupported())
}
