//! The core's ACP client, as pure code (design decisions 4, 5 and 8 of the
//! OpenSpec change `agents-sessions`): the core's shell feeds an agent's
//! stdout through these and writes what they return to its stdin, and never
//! decodes ACP itself. Only this directory names `agent_client_protocol_schema`
//! (`ports R3`, `only_the_acp_modules_name_the_schema_crate`), and within it
//! only the version's own module names that version (`ports R14`,
//! `acp_versions_stay_in_their_modules`).
//!
//! - [`envelope`]: the JSON-RPC envelope ([`envelope::Message`], decoded by
//!   hand), with no ACP version in it.
//! - [`forms`]: what the reducer reads and asks to be written, in Brindle's
//!   own closed types, whatever the version.
//! - [`dialect`]: the version a session speaks ([`dialect::Dialect`]), and
//!   the only `match` on it.
//! - [`v1`]: ACP v1 decoded into the forms through the schema crate's types,
//!   and the forms' requests encoded as v1 lines.
//! - [`diff`]: a file change's hunks from its old and new text.
//! - [`limits`]: the bounds on what a session holds and sends, and the
//!   invariants they add up to (every event fits a frame, every
//!   checkpoint fits a holder).
//! - [`lines`]: stdout split into bounded lines ([`lines::Lines`]).
//! - [`plain`]: a command's output as plain text.
//! - [`rpc`]: the request bookkeeping ([`rpc::Rpc`]), which keeps the
//!   session's dialect and takes each line from it.
//! - [`reduce`]: the reducer ([`reduce::step`] over a [`reduce::AgentSession`]),
//!   which turns the agent's stdout, the client's commands and the holder's
//!   events into transcript events, stdin bytes, replies and checkpoints.
//! - [`checkpoint`]: an [`reduce::AgentSession`] encoded for its holder and
//!   decoded by the next core ([`checkpoint::adopt`]), or adopted as a
//!   conversation that could not be restored.
//!
//! Each is a whole pure module (`fcis` R2, R3; `PURE_MODULES`).

pub mod checkpoint;
pub mod dialect;
pub mod diff;
pub mod envelope;
pub mod forms;
pub mod limits;
pub mod lines;
pub mod plain;
pub mod reduce;
pub mod rpc;
pub mod v1;
