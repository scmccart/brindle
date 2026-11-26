//! Brindle's one door to `brindle-transcript`: the types an agent zone
//! folds and shows, re-exported, and the two mappings from an agent's
//! activity onto program status's states. No other file in `src/` names
//! the crate (`tests/architecture.rs` checks it), so its name and paths
//! change in one place.
//!
//! Pure: values in, values out.

pub use brindle_transcript::{
    Activity, AgentState, Entry, EntryId, Event, HoldReason, NoticeLevel, PermissionKind,
    PermissionOption, PermissionOutcome, Slot, StopReason, ToolKind, ToolStatus, Transcript, TurnState,
};

use crate::remote::protocol::client::AgentActivity;
use crate::terminal::status::State;

/// The status a live agent zone shows for what its transcript says the
/// agent is doing: one to one, in program status's names.
pub fn status_of(activity: Activity) -> State {
    match activity {
        Activity::Idle => State::Idle,
        Activity::Working => State::Working,
        Activity::Blocked => State::Blocked,
        Activity::Done => State::Done,
        Activity::Error => State::Error,
    }
}

/// The status a listed agent session shows (the sessions palette, the
/// sweep) for the activity its listing gives: one to one, as
/// [`status_of`].
pub fn listed_status(activity: AgentActivity) -> State {
    match activity {
        AgentActivity::Idle => State::Idle,
        AgentActivity::Working => State::Working,
        AgentActivity::Blocked => State::Blocked,
        AgentActivity::Done => State::Done,
        AgentActivity::Error => State::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_activity_maps_onto_its_state() {
        let table = [
            (Activity::Idle, State::Idle),
            (Activity::Working, State::Working),
            (Activity::Blocked, State::Blocked),
            (Activity::Done, State::Done),
            (Activity::Error, State::Error),
        ];
        for (activity, state) in table {
            assert_eq!(status_of(activity), state, "{activity:?}");
        }
    }

    #[test]
    fn a_listed_activity_maps_onto_its_state() {
        let table = [
            (AgentActivity::Idle, State::Idle),
            (AgentActivity::Working, State::Working),
            (AgentActivity::Blocked, State::Blocked),
            (AgentActivity::Done, State::Done),
            (AgentActivity::Error, State::Error),
        ];
        for (activity, state) in table {
            assert_eq!(listed_status(activity), state, "{activity:?}");
        }
    }

    /// Both mappings agree: a session shows the same status listed as live.
    #[test]
    fn listed_and_live_agree() {
        let pairs = [
            (Activity::Idle, AgentActivity::Idle),
            (Activity::Working, AgentActivity::Working),
            (Activity::Blocked, AgentActivity::Blocked),
            (Activity::Done, AgentActivity::Done),
            (Activity::Error, AgentActivity::Error),
        ];
        for (live, listed) in pairs {
            assert_eq!(status_of(live), listed_status(listed), "{live:?}");
        }
    }
}
