//! What a tab's face or a zone's header leads with for its status
//! (OpenSpec change `agents-zone-attention`, design decision 3): a dot in
//! the status's colour, or a spinner while an agent works.

use crate::terminal::status::State;

/// A status mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    Dot(State),
    /// A small spinner, in place of the `working` dot.
    Spinner,
}

/// The mark for `status`, `agent_working` when an agent zone's `working`
/// is what it shows (a zone header: the zone's own; a tab face: one of the
/// tab's working zones is an agent zone). A terminal's `working` record
/// keeps its dot: a build's can last for hours, and a spinner would keep
/// the motion clock running.
pub fn mark(status: Option<State>, agent_working: bool) -> Mark {
    match status {
        None => Mark::None,
        Some(State::Working) if agent_working => Mark::Spinner,
        Some(state @ (State::Working | State::Idle | State::Blocked | State::Done | State::Error)) => Mark::Dot(state),
    }
}

impl Mark {
    pub fn is_spinner(self) -> bool {
        match self {
            Self::Spinner => true,
            Self::None | Self::Dot(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_working_spins_and_everything_else_keeps_its_dot() {
        assert_eq!(mark(None, false), Mark::None);
        assert_eq!(mark(None, true), Mark::None);
        assert_eq!(mark(Some(State::Working), true), Mark::Spinner);
        assert_eq!(mark(Some(State::Working), false), Mark::Dot(State::Working), "a terminal's working record");
        for state in [State::Idle, State::Blocked, State::Done, State::Error] {
            assert_eq!(mark(Some(state), true), Mark::Dot(state), "{state:?}");
            assert_eq!(mark(Some(state), false), Mark::Dot(state), "{state:?}");
        }
        assert!(Mark::Spinner.is_spinner());
        assert!(!Mark::Dot(State::Working).is_spinner() && !Mark::None.is_spinner());
    }
}
