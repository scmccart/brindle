//! The GPUI-free terminal state that Brindle and its server share: the
//! shell-integration parser with its OSC 133 marks, and the OSC 7501
//! program status records.

pub mod notice;
pub mod replay;
pub mod shell_integration;
pub mod status;

pub use notice::{Notice, Source};
