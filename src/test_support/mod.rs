//! Helpers shared by this crate's unit tests: waits within a scaled limit
//! ([`wait`]), temporary directories with a cleanup guard ([`TempDir`]) and,
//! on Unix, executables that can be run at once ([`write_executable`]).

mod dir;
#[cfg(unix)]
mod exec;
pub mod wait;

pub use dir::TempDir;
#[cfg(unix)]
pub use exec::write_executable;
