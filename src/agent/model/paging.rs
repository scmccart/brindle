//! Paging (OpenSpec change `agents-log-paging`): the Earlier row, its
//! fetches and the window the zone follows.

use gpui::SharedString;

use crate::agent::transcript::{EntryId, PAGE_BYTES, Window};

use super::{ChatModel, RequestId, SessionHandle};
use super::ledger::{ReplyError, Sent};
use super::phase::{AgentPhase, Link};

/// The entries before the ones the zone holds, as stored (decision 7):
/// whether a fetch is on its way is the request ledger's, and
/// [`ChatModel::earlier`] derives the row from both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Paging {
    None,
    Available { count: u64 },
    Failed { count: u64, why: SharedString },
}

/// The reason a choosing zone shows when the latest was asked for and
/// there is none (`-c` with nothing to continue).
pub fn no_earlier_text(dir: &str) -> String {
    format!("no earlier conversation in {dir}")
}

/// The entries before the ones the zone holds (OpenSpec change
/// `agents-log-paging` decision 11), from the transcript's window: none,
/// or `count` of them, which the zone can fetch, is fetching, or failed
/// to fetch. Derived on each read ([`ChatModel::earlier`]): `Loading` while
/// the request ledger holds a fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Earlier {
    None,
    Available { count: u64 },
    Loading { count: u64 },
    Failed { count: u64, why: SharedString },
}

/// A page to ask for: the entries before `before`, within `max_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fetch {
    pub before: EntryId,
    pub max_bytes: usize,
}

impl ChatModel {
    /// The entries before the ones the zone holds: the stored paging state,
    /// loading while a fetch is on its way (decision 7).
    pub fn earlier(&self) -> Earlier {
        let loading = self.pending.values().any(|sent| *sent == Sent::Fetch);
        match &self.paging {
            Paging::None => Earlier::None,
            Paging::Available { count } | Paging::Failed { count, .. } if loading => Earlier::Loading { count: *count },
            Paging::Available { count } => Earlier::Available { count: *count },
            Paging::Failed { count, why } => Earlier::Failed { count: *count, why: why.clone() },
        }
    }

    /// The paging state's count, when there is one.
    pub(super) fn paging_count(&self) -> Option<u64> {
        match &self.paging {
            Paging::None => None,
            Paging::Available { count } | Paging::Failed { count, .. } => Some(*count),
        }
    }

    /// The page to ask for now (decision 11), given whether the Earlier
    /// row is in view and was in the frame before: while entries are
    /// available, whenever the row is in view; after a failure, only when
    /// the row comes into view again (or on [`Self::retry`]); nothing while
    /// a fetch waits, the link is down, the session can't be followed, or
    /// with nothing earlier.
    pub fn fetch_due(&self, row_in_view: bool, was_in_view: bool) -> Option<Fetch> {
        let ask = match &self.earlier() {
            Earlier::Available { .. } => row_in_view,
            Earlier::Failed { .. } => row_in_view && !was_in_view,
            Earlier::None | Earlier::Loading { .. } => false,
        };
        if ask { self.fetch() } else { None }
    }

    /// The Earlier row was pressed: after a failure, ask again.
    pub fn retry(&self) -> Option<Fetch> {
        match &self.earlier() {
            Earlier::Failed { .. } => self.fetch(),
            Earlier::None | Earlier::Available { .. } | Earlier::Loading { .. } => None,
        }
    }

    /// The page before the window, when one can be asked for now.
    fn fetch(&self) -> Option<Fetch> {
        let live = match &self.phase {
            AgentPhase::Live(_) => true,
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => false,
        };
        let up = match self.link {
            Link::Up => true,
            Link::Reconnecting => false,
        };
        let running = match self.handle {
            SessionHandle::Running => true,
            SessionHandle::Starting => false,
        };
        let window = self.window.filter(|_| live && up && running)?;
        Some(Fetch { before: window.from, max_bytes: PAGE_BYTES })
    }

    /// The fetch [`Self::fetch_due`] or [`Self::retry`] asked for went out
    /// as `request`: the ledger's record is what shows it loading.
    pub fn fetching(&mut self, request: RequestId) {
        self.pending.insert(request, Sent::Fetch);
    }

    /// The reply to a fetch, which [`Self::replied`] took from the ledger:
    /// its page has already been folded, so an `Ok` finds the window moved;
    /// a refusal shows on the row. A reply the zone no longer waits for (a
    /// replay started since) never gets here.
    pub(super) fn fetched(&mut self, outcome: Result<(), ReplyError>) {
        let Some(count) = self.paging_count() else { return };
        self.paging = match outcome {
            Ok(()) => Paging::Available { count },
            Err(why) => Paging::Failed { count, why: why.text().to_string().into() },
        };
    }

    /// Follows the transcript's window: none when it has none; else its
    /// count, available again once a page came (or the zone starts over),
    /// a failure kept otherwise.
    pub(super) fn follow_window(&mut self, window: Option<Window>, reset: bool, paged: bool) {
        self.window = window;
        let Some(Window { earlier: count, .. }) = window else {
            self.paging = Paging::None;
            return;
        };
        let was = std::mem::replace(&mut self.paging, Paging::None);
        self.paging = match was {
            Paging::Failed { why, .. } if !(reset || paged) => Paging::Failed { count, why },
            Paging::None | Paging::Available { .. } | Paging::Failed { .. } => Paging::Available { count },
        };
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::{ToolKind, ToolStatus};

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    fn conversation_line(model: &ChatModel) -> String {
        let dump = model.describe(1, false, &found(model), &linux);
        dump.lines().find(|line| line.starts_with("--- conversation")).unwrap().to_string()
    }

    #[test]
    fn a_paged_zone_fetches_while_the_row_shows_and_folds_each_page_in_front() {
        let server = long_log(40);
        let mut zone = paged(&server);
        let window = zone.shared.transcript().window().expect("800 KiB is paged");
        let count = window.earlier;
        assert_eq!(zone.model.earlier(), Earlier::Available { count });
        assert_eq!(zone.model.title(), "Long talk", "the page carries the title");
        assert_eq!(zone.model.blocks().first().map(|block| block.entry), Some(window.from));

        // Asked for only while the row is in view.
        assert_eq!(zone.model.fetch_due(false, false), None);
        let fetch = zone.model.fetch_due(true, false).expect("a fetch while the row shows");
        assert_eq!(fetch, Fetch { before: window.from, max_bytes: PAGE_BYTES });
        zone.model.fetching(7);
        assert_eq!(zone.model.earlier(), Earlier::Loading { count });
        assert_eq!(zone.model.fetch_due(true, true), None, "nothing more while it loads");
        assert!(conversation_line(&zone.model).ends_with(" earlier=loading"));

        // The page goes in front: its entries' blocks are one splice at 0,
        // their Markdown rendered once, the row's count lower, nothing
        // raised, and a `done` the user saw stays seen.
        assert_eq!(zone.model.seen(), Some(None));
        let shown = zone.model.blocks().len();
        let absorbed = answer_fetch(&mut zone, &server, fetch, 7);
        let added = zone.model.blocks().len() - shown;
        assert_eq!(absorbed.splices, [Splice { range: 0..0, count: added }]);
        assert!(added > 0);
        assert_eq!((absorbed.notice, absorbed.status, absorbed.title_changed), (None, None, false));
        assert!(zone.model.seen && zone.model.shown_status().is_none(), "the prepend unsaw the done");
        let left = zone.shared.transcript().window().map_or(0, |window| window.earlier);
        assert!(left < count);
        assert_eq!(zone.model.earlier(), if left == 0 { Earlier::None } else { Earlier::Available { count: left } });
        let first = zone.model.blocks()[0].entry;
        assert!(zone.model.rendered.contains_key(&first) || matches!(zone.shared.transcript().entry(first), Some(Entry::User { .. })));
        let ids: Vec<EntryId> = zone.model.entries.iter().map(|(id, _)| *id).collect();
        let held: Vec<EntryId> = zone.shared.transcript().entries().map(|(id, _)| id).collect();
        assert_eq!(ids, held, "the zone holds the transcript's entries, in order");

        // Fetched back to the start, the last page drops the window.
        let mut request = 8;
        while let Some(fetch) = zone.model.fetch_due(true, true) {
            zone.model.fetching(request);
            answer_fetch(&mut zone, &server, fetch, request);
            request += 1;
        }
        assert_eq!(zone.model.earlier(), Earlier::None);
        assert_eq!(zone.model.entries.len() as u64, 41);
        assert_eq!(zone.model.entries[0].0, EntryId(1));
        assert!(conversation_line(&zone.model).ends_with(" earlier=none"));
        // The thought (entry 5) has its part, drawn closed.
        assert!(!parts_of(&zone.model, 5).is_empty());
        in_step(&zone.model);
    }

    /// Decision 12: `earlier=N` after `at=`, which `wait_text` drops, so a
    /// wait reads `entries=N earlier=N`; a failure's reason escaped as the
    /// other quoted fields are.
    #[test]
    fn the_conversation_line_counts_the_earlier_entries() {
        let server = long_log(40);
        let mut zone = paged(&server);
        let count = zone.shared.transcript().window().unwrap().earlier;
        let held = zone.model.entries.len();
        assert_eq!(conversation_line(&zone.model), format!("--- conversation entries={held} at=away earlier={count}"));
        let wait = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(wait.contains(&format!("--- conversation entries={held} earlier={count}\n")), "{wait}");
        zone.model.fetching(3);
        zone.model.replied(3, Err("refused: \"no\"\nat all".into()));
        assert!(
            conversation_line(&zone.model).ends_with(&format!(" earlier={count} fetch-error=\"refused: \\\"no\\\"\\nat all\"")),
            "{}",
            conversation_line(&zone.model)
        );
    }

    #[test]
    fn a_failed_fetch_asks_again_only_when_the_row_comes_back_or_is_pressed() {
        let server = long_log(40);
        let mut zone = paged(&server);
        let count = zone.shared.transcript().window().unwrap().earlier;
        assert_eq!(zone.model.retry(), None, "nothing to retry");
        zone.model.fetching(3);
        zone.model.replied(3, Err("the connection to the terminal server is down".into()));
        assert_eq!(
            zone.model.earlier(),
            Earlier::Failed { count, why: "the connection to the terminal server is down".into() }
        );
        assert_eq!(
            conversation_line(&zone.model),
            format!("--- conversation entries={} at=away earlier={count} fetch-error=\"the connection to the terminal server is down\"", zone.model.entries.len())
        );
        assert_eq!(zone.model.fetch_due(true, true), None, "still in view: not again");
        assert!(zone.model.fetch_due(true, false).is_some(), "back in view: again");
        assert!(zone.model.retry().is_some(), "a press: again");
        // Nothing while reconnecting, not even on a press.
        zone.model.set_link(Link::Reconnecting);
        assert_eq!((zone.model.fetch_due(true, false), zone.model.retry()), (None, None));
        zone.model.set_link(Link::Up);
        // A fetch that then succeeds is available again.
        zone.model.fetching(4);
        let fetch = Fetch { before: zone.shared.transcript().window().unwrap().from, max_bytes: PAGE_BYTES };
        answer_fetch(&mut zone, &server, fetch, 4);
        assert!(matches!(zone.model.earlier(), Earlier::Available { .. } | Earlier::None));
    }

    #[test]
    fn a_reset_drops_a_fetch_in_flight_and_its_late_reply() {
        let server = long_log(40);
        let mut zone = paged(&server);
        let count = zone.shared.transcript().window().unwrap().earlier;
        zone.model.fetching(5);
        // A reconnect replays the newest page again.
        zone.replay(&server.page(PAGE_BYTES));
        assert_eq!(zone.model.earlier(), Earlier::Available { count });
        assert_eq!(zone.model.replied(5, Err("late".into())), Replied::Nothing);
        assert_eq!(zone.model.earlier(), Earlier::Available { count }, "the late reply changed nothing");
        assert_eq!(zone.model.refusal(), None);
        // A short log has no window, and nothing to fetch.
        let mut short = Zone::ready();
        short.feed(&[added(1, user("hi"))]);
        assert_eq!(short.model.earlier(), Earlier::None);
        assert_eq!(short.model.fetch_due(true, false), None);
    }

    #[test]
    fn a_tool_call_in_progress_above_the_page_shows_working_until_fetched() {
        let mut server = Transcript::new();
        let mut events = vec![
            ready(),
            added(1, user("go")),
            turn(TurnState::Working),
            added(2, Entry::tool("c1".into(), "cargo build".into(), ToolKind::Execute, ToolStatus::InProgress)),
        ];
        for n in 0..20 {
            events.push(added(n + 3, agent(&"y".repeat(20 * 1024))));
        }
        for event in &events {
            server.apply(event).unwrap();
        }
        let mut zone = paged(&server);
        assert_eq!(zone.model.working_row(), Some(WorkingRow::Working));
        let mut request = 1;
        let mut rows = Vec::new();
        while let Some(fetch) = zone.model.fetch_due(true, true) {
            zone.model.fetching(request);
            let absorbed = answer_fetch(&mut zone, &server, fetch, request);
            let len = zone.model.blocks().len();
            assert!(
                absorbed.splices.iter().all(|splice| splice.range.end <= len),
                "the row shows throughout: no splice reaches past the blocks"
            );
            rows.push(zone.model.working_row());
            request += 1;
        }
        assert!(rows.contains(&Some(WorkingRow::Tool { title: "cargo build".into() })), "{rows:?}");
        assert_eq!(zone.model.working_row(), Some(WorkingRow::Tool { title: "cargo build".into() }));
    }
}
