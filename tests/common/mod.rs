//! Helpers shared by the end-to-end and performance suites, included by
//! each with `#[path]`.

#![allow(dead_code)]

pub type Result<T = ()> = std::result::Result<T, String>;

/// Lines with trailing spaces and trailing empty lines removed.
pub fn trim_screen<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = lines.map(|l| l.trim_end().to_string()).collect();
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Fails with a message unless `cond` holds.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+));
        }
    };
}
