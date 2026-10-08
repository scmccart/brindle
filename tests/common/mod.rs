//! Helpers shared by the end-to-end and performance suites, included by
//! each with `#[path]`.

#![allow(dead_code)]

pub mod tmux;

pub type Result<T = ()> = std::result::Result<T, String>;

/// Fails with a message unless `cond` holds.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}
