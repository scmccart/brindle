//! Agent zones: an ACP agent session in the terminal server, shown as a
//! conversation instead of a terminal (`agent-zones`).
//!
//! [`transcript`] is Brindle's one door to `brindle-transcript`: every other
//! file reaches the transcript's types through it.

// The later groups of `agents-profiles` use these; until then only the
// tests do.
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub mod transcript;
