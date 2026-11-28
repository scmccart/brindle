//! Which agent zone takes a terminal's output (OpenSpec change
//! `agents-prompt-context`, design decision 8): none open, the only one,
//! or a list to choose from, the most recently focused first.

use crate::agent::conversations::ZoneAt;

/// An agent zone of the window, as the list names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTarget {
    pub at: ZoneAt,
    /// Its tab's title and its own (the conversation's, or its profile's
    /// name).
    pub tab_title: String,
    pub zone_title: String,
    /// How recently it had focus: larger is more recent.
    pub recency: u64,
}

impl AgentTarget {
    /// The list's row: the zone's title, then the tab's when it differs.
    pub fn label(&self) -> String {
        if self.tab_title == self.zone_title || self.tab_title.is_empty() {
            self.zone_title.clone()
        } else {
            format!("{} — {}", self.zone_title, self.tab_title)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// No agent zone is open: the notice line says so.
    None,
    One(ZoneAt),
    /// Ask, the most recently focused first.
    Ask(Vec<AgentTarget>),
}

/// The notice line's words when no agent zone is open.
pub const NO_AGENT_ZONE: &str = "no agent zone is open";

/// Where an attachment from a terminal goes, given the window's agent zones
/// in any order.
pub fn target_plan(mut zones: Vec<AgentTarget>) -> Target {
    match zones.len() {
        0 => Target::None,
        1 => Target::One(zones[0].at),
        _ => {
            // Ties keep the window's order.
            zones.sort_by(|a, b| b.recency.cmp(&a.recency));
            Target::Ask(zones)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::tabs::TabId;
    use crate::zone_tree::ZoneId;

    fn zone(tab: u64, zone: u64, recency: u64) -> AgentTarget {
        AgentTarget {
            at: ZoneAt { tab: TabId(tab), zone: ZoneId(zone) },
            tab_title: format!("tab {tab}"),
            zone_title: format!("Claude {zone}"),
            recency,
        }
    }

    #[test]
    fn none_one_or_a_list_most_recently_focused_first_across_tabs() {
        assert_eq!(target_plan(Vec::new()), Target::None);
        assert_eq!(target_plan(vec![zone(1, 2, 0)]), Target::One(zone(1, 2, 0).at));
        let Target::Ask(asked) = target_plan(vec![zone(1, 1, 3), zone(2, 4, (1 << 32) + 1), zone(1, 2, 7), zone(3, 5, 7)])
        else {
            panic!("asks");
        };
        let order: Vec<(u64, u64)> = asked.iter().map(|z| (z.at.tab.0, z.at.zone.0)).collect();
        assert_eq!(order, [(2, 4), (1, 2), (3, 5), (1, 1)]);
    }

    #[test]
    fn a_row_names_the_zone_then_its_tab() {
        assert_eq!(zone(1, 2, 0).label(), "Claude 2 — tab 1");
        let same = AgentTarget { tab_title: "Claude 2".into(), ..zone(1, 2, 0) };
        assert_eq!(same.label(), "Claude 2");
    }
}
