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

/// The holder's switch: `inject_fault` in `holder.rs` panics on bytes
/// holding its marker.
pub const HOLDER_PANIC: &str = "BRINDLE_TEST_HOLDER_PANIC";
/// The core's switch: `inject_fault` in `core/screen.rs` panics on output
/// holding its marker.
pub const FEED_PANIC: &str = "BRINDLE_TEST_FEED_PANIC";
/// The core's switch for agent sessions: `inject_fault` in
/// `core/agents.rs` panics, inside the guarded call to the ACP reducer, on
/// agent output or a prompt holding its marker.
pub const ACP_PANIC: &str = "BRINDLE_TEST_ACP_PANIC";
/// The core's switch for a panic part way through an agent session's step
/// (OpenSpec change `agents-process-lifecycle`, decision 6): `inject_late`
/// in `core/agents.rs` panics inside the guarded call, after the reducer
/// applied the step's events, when its input held the marker. The log then
/// has events no client and no checkpoint has.
pub const ACP_PANIC_LATE: &str = "BRINDLE_TEST_ACP_PANIC_LATE";

/// The switches this build honours, as `--list-fault-switches` prints
/// them: all in a debug build, none in a release build, which never reads
/// them (`process R20`).
pub fn honoured() -> &'static [&'static str] {
    if cfg!(debug_assertions) {
        &[HOLDER_PANIC, FEED_PANIC, ACP_PANIC, ACP_PANIC_LATE]
    } else {
        &[]
    }
}

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
    fn the_helper_lists_its_switches_only_in_debug_builds() {
        if cfg!(debug_assertions) {
            assert_eq!(
                honoured(),
                [
                    "BRINDLE_TEST_HOLDER_PANIC",
                    "BRINDLE_TEST_FEED_PANIC",
                    "BRINDLE_TEST_ACP_PANIC",
                    "BRINDLE_TEST_ACP_PANIC_LATE"
                ]
            );
        } else {
            assert!(honoured().is_empty(), "{:?}", honoured());
        }
    }

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
