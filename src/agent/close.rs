//! Whether closing a zone or a tab asks first (design.md decision 12): it
//! asks only when an agent zone among them is busy, `working` or
//! `blocked` by the status it shows. Every close path (Close Zone, the
//! header's ×, its menu, Close Tab, the tab's ×, a middle click, the tab
//! menu) goes through [`close_plan`]; quitting never does.
//!
//! Pure: the shell gathers each zone's kind, name and shown status.

use crate::terminal::status::State;
use crate::zone_view::ZoneId;

/// What makes an agent busy for closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBusy {
    /// A turn is running (or the agent is starting).
    Working,
    /// A permission request waits for the user.
    Blocked,
}

impl AgentBusy {
    /// Whether an agent zone showing `status` is busy: `working` and
    /// `blocked` are; idle, done, error and no status aren't.
    pub fn of(status: Option<State>) -> Option<Self> {
        match status? {
            State::Working => Some(Self::Working),
            State::Blocked => Some(Self::Blocked),
            State::Idle | State::Done | State::Error => None,
        }
    }

    /// The dump's word (`--- confirm close … busy=ID:working`).
    pub fn word(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Blocked => "blocked",
        }
    }

    /// What the dialog says the agent is doing.
    fn doing(self) -> &'static str {
        match self {
            Self::Working => "is working",
            Self::Blocked => "is waiting for permission",
        }
    }
}

/// One zone a close would end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closing {
    /// A terminal zone: never asks.
    Terminal(ZoneId),
    /// An agent zone, with its title (the agent's name, else its
    /// profile's) and whether it is busy (its model's `close_busy`:
    /// [`AgentBusy::of`] its status, none while it logs in).
    Agent { zone: ZoneId, name: String, busy: Option<AgentBusy> },
}

/// A busy agent the dialog names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusyAgent {
    pub zone: ZoneId,
    pub name: String,
    pub busy: AgentBusy,
}

/// What a close does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosePlan {
    /// Close at once.
    Now,
    /// Ask first, naming these agents in the zones' order.
    Ask(Vec<BusyAgent>),
}

/// Whether closing `zones` (a zone, or every zone of a tab) asks first.
pub fn close_plan(zones: &[Closing]) -> ClosePlan {
    let busy: Vec<BusyAgent> = zones
        .iter()
        .filter_map(|zone| match zone {
            Closing::Terminal(_) => None,
            Closing::Agent { zone, name, busy } => {
                busy.map(|busy| BusyAgent { zone: *zone, name: name.clone(), busy })
            }
        })
        .collect();
    if busy.is_empty() { ClosePlan::Now } else { ClosePlan::Ask(busy) }
}

/// The close confirmation's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseText {
    pub title: String,
    /// One line per busy agent, then what closing does.
    pub lines: Vec<String>,
}

/// What the dialog says for `busy` (never empty: [`ClosePlan::Ask`]).
pub fn close_text(busy: &[BusyAgent]) -> CloseText {
    let title = match busy.len() {
        1 => "Close a working agent?".to_string(),
        n => format!("Close {n} working agents?"),
    };
    let mut lines: Vec<String> = busy.iter().map(|agent| format!("{} {}", agent.name, agent.busy.doing())).collect();
    lines.push("Closing ends it; the conversation can't be got back.".to_string());
    CloseText { title, lines }
}

/// The dump's `busy=` value: `ID:working|ID:blocked`, comma-separated.
pub fn describe_busy(busy: &[BusyAgent]) -> String {
    busy.iter().map(|agent| format!("{}:{}", agent.zone.0, agent.busy.word())).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(zone: u64, status: Option<State>) -> Closing {
        Closing::Agent { zone: ZoneId(zone), name: format!("Agent {zone}"), busy: AgentBusy::of(status) }
    }

    #[test]
    fn only_working_and_blocked_are_busy() {
        let table = [
            (None, None),
            (Some(State::Idle), None),
            (Some(State::Working), Some(AgentBusy::Working)),
            (Some(State::Blocked), Some(AgentBusy::Blocked)),
            (Some(State::Done), None),
            (Some(State::Error), None),
        ];
        for (status, busy) in table {
            assert_eq!(AgentBusy::of(status), busy, "{status:?}");
        }
    }

    #[test]
    fn terminals_and_idle_agents_close_at_once() {
        assert_eq!(close_plan(&[]), ClosePlan::Now);
        assert_eq!(close_plan(&[Closing::Terminal(ZoneId(1))]), ClosePlan::Now);
        assert_eq!(
            close_plan(&[Closing::Terminal(ZoneId(1)), agent(2, Some(State::Done)), agent(3, None)]),
            ClosePlan::Now
        );
        assert_eq!(close_plan(&[agent(1, Some(State::Error)), agent(2, Some(State::Idle))]), ClosePlan::Now);
    }

    /// A tab mixing busy, idle and terminal zones asks, naming only the
    /// busy agents, in the zones' order.
    #[test]
    fn a_busy_agent_among_the_zones_asks_naming_only_the_busy_ones() {
        let zones = [
            agent(1, Some(State::Idle)),
            Closing::Terminal(ZoneId(2)),
            agent(3, Some(State::Blocked)),
            agent(4, Some(State::Done)),
            agent(5, Some(State::Working)),
        ];
        let ClosePlan::Ask(busy) = close_plan(&zones) else { panic!("asks") };
        assert_eq!(
            busy,
            [
                BusyAgent { zone: ZoneId(3), name: "Agent 3".into(), busy: AgentBusy::Blocked },
                BusyAgent { zone: ZoneId(5), name: "Agent 5".into(), busy: AgentBusy::Working },
            ]
        );
        assert_eq!(describe_busy(&busy), "3:blocked,5:working");
        assert_eq!(close_plan(&[agent(7, Some(State::Working))]), close_plan(&[agent(7, Some(State::Working))]));
    }

    #[test]
    fn the_dialog_names_each_busy_agent() {
        let one = [BusyAgent { zone: ZoneId(1), name: "Fake Agent".into(), busy: AgentBusy::Working }];
        assert_eq!(
            close_text(&one),
            CloseText {
                title: "Close a working agent?".into(),
                lines: vec!["Fake Agent is working".into(), "Closing ends it; the conversation can't be got back.".into()],
            }
        );
        let two = [
            BusyAgent { zone: ZoneId(1), name: "Fake Agent".into(), busy: AgentBusy::Working },
            BusyAgent { zone: ZoneId(2), name: "Other".into(), busy: AgentBusy::Blocked },
        ];
        let text = close_text(&two);
        assert_eq!(text.title, "Close 2 working agents?");
        assert_eq!(text.lines[..2], ["Fake Agent is working".to_string(), "Other is waiting for permission".to_string()]);
    }
}
