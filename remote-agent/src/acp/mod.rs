//! The core's ACP client, as pure code (design decisions 4, 5 and 8 of the
//! OpenSpec change `agents-sessions`): the core's shell feeds an agent's
//! stdout through these and writes what they return to its stdin, and never
//! decodes ACP itself. Only this directory names `agent_client_protocol_schema`
//! (`ports R3`, `only_the_acp_modules_name_the_schema_crate`).
//!
//! - [`wire`]: the JSON-RPC envelope ([`wire::Message`], decoded by hand) and
//!   the ACP forms the reducer reads, decoded through the schema crate's types
//!   into Brindle's own closed ones.
//! - [`lines`]: stdout split into bounded lines ([`lines::Lines`]).
//! - [`rpc`]: the request bookkeeping ([`rpc::Rpc`]) and every line the core
//!   writes to the agent.
//!
//! Each is a whole pure module (`fcis` R2, R3; `PURE_MODULES`).

pub mod lines;
pub mod wire;
