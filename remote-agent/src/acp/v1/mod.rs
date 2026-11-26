//! ACP protocol version 1: [`decode`] reads what a v1 agent writes into the
//! forms, and [`encode`] writes the requests (`ports R14`).

pub mod decode;
pub mod encode;
