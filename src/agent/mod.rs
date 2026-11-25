//! Agent zones: an ACP agent session in the terminal server, shown as a
//! conversation instead of a terminal (`agent-zones`).
//!
//! [`transcript`] is Brindle's one door to `brindle-transcript`: every other
//! file reaches the transcript's types through it.

// The later groups of `agents-profiles` use these; until then only the
// tests use the mappings, and nothing the re-exports.
#[cfg_attr(not(test), allow(dead_code))]
#[allow(unused_imports)]
pub mod transcript;

// What an agent session's start asks for: group 4's `Start::SpawnAgent`
// builds it; until then only the tests do.
#[cfg_attr(not(test), allow(dead_code))]
pub mod spawn;

// Whether a close asks first: group 6's close paths call it.
#[cfg_attr(not(test), allow(dead_code))]
pub mod close;

// The chat model and the protocol gate: groups 4 and 5 use them.
#[cfg_attr(not(test), allow(dead_code))]
pub mod model;
