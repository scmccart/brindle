//! What both server roles need to confine a fault to its unit (`process
//! R3`): the text of a caught panic, and the debug-only switches through
//! which the tests inject one (`process R20`).
//!
//! A switch is an environment variable naming a marker: in a debug build,
//! the code it guards panics when what it handles contains the marker, so a
//! test can fail one session by making its program print it, or by sending
//! it. They are environment variables rather than `cfg(test)` hooks because
//! the server tests run the built binary; release builds never read them.

use std::any::Any;
use std::sync::OnceLock;

/// What a caught panic said: its message, or "no message".
pub fn describe(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".into())
}

/// The marker the fault switch `var` names, read once into `cell`: `None`
/// when it is unset or empty.
#[cfg(debug_assertions)]
pub fn marker(var: &str, cell: &'static OnceLock<Option<Vec<u8>>>) -> Option<&'static [u8]> {
    cell.get_or_init(|| {
        std::env::var_os(var)
            .filter(|value| !value.is_empty())
            .map(|value| value.into_encoded_bytes())
    })
    .as_deref()
}

/// A release build has no fault switches.
#[cfg(not(debug_assertions))]
pub fn marker(_var: &str, _cell: &'static OnceLock<Option<Vec<u8>>>) -> Option<&'static [u8]> {
    None
}

/// Whether `bytes` contain `marker`, which is never empty.
pub fn holds(bytes: &[u8], marker: &[u8]) -> bool {
    bytes.windows(marker.len()).any(|window| window == marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panics_text_is_its_message() {
        let caught = |f: fn()| std::panic::catch_unwind(f).unwrap_err();
        assert_eq!(describe(&*caught(|| panic!("plain"))), "plain");
        assert_eq!(
            describe(&*caught(|| panic!("formatted {}", 1))),
            "formatted 1"
        );
        assert_eq!(
            describe(&*caught(|| std::panic::panic_any(7u8))),
            "no message"
        );
    }

    #[test]
    fn a_marker_is_found_anywhere_in_the_bytes() {
        assert!(holds(b"before-mark-after", b"mark"));
        assert!(holds(b"mark", b"mark"));
        assert!(!holds(b"mar", b"mark"));
        assert!(!holds(b"", b"mark"));
    }
}
