//! What a notification looks like once a scanner has found it. The status
//! records ([`crate::status`]) produce these for the states that need the
//! user.

/// Where a notification came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Bell,
    Osc9,
    Osc777,
    /// A program status record (OSC 7501) that needs the user.
    Osc7501,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Bell => "bell",
            Source::Osc9 => "osc9",
            Source::Osc777 => "osc777",
            Source::Osc7501 => "osc7501",
        }
    }
}

/// A notification a program asked for, already cleaned up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub source: Source,
    /// OSC 777's title; empty for OSC 9.
    pub title: String,
    pub body: String,
}
