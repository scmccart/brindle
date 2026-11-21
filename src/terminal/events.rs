//! What bounds a terminal's event channel (`process R9`): the gate every
//! event passes on its producer's thread before it is queued for the UI
//! thread.
//!
//! - **Merged at the source:** `Wakeup`, title changes, foreground reports
//!   and clipboard writes need only their latest value, so each kind has at
//!   most one entry waiting (clipboard writes, one per selection). The
//!   producer puts the value in the gate's slot and queues a marker only
//!   when none is waiting; the consumer takes the slot's value when it
//!   reaches the marker.
//! - **Life events** (replay start and end, exit, loss, failure) are always
//!   queued, and counted.
//! - **Side events** (bells, notifications, status reports, replies to the
//!   program's queries) are counted against [`EVENT_MARKS`]: from the high
//!   mark until the count falls under the low one they are dropped, and the
//!   consumer is told once how many went and whether a status report was
//!   among them ([`Recovered`]).
//! - **Ignored events** are never queued: the consumer has nothing to do
//!   with them.
//!
//! Output itself is never queued: it goes straight into the grid under the
//! term lock, and only the `Wakeup` that announces it passes here.

use alacritty_terminal::event::Event as AlacEvent;
use alacritty_terminal::term::ClipboardType;

use super::ChannelEvent;
use crate::notify::osc::Found;
use crate::queue_marks::QueueMarks;

/// The marks on a terminal's waiting life and side events. The consumer
/// takes up to 1024 a chunk, so 2048 means the UI thread is more than two
/// chunks behind, and 512 lets the next update clear the state.
pub const EVENT_MARKS: QueueMarks = QueueMarks::new(2048, 512);

/// What an event is to the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Only its latest value counts; at most one waits.
    Merged(Merge),
    /// Always queued.
    Life,
    /// Dropped past the high mark; `status` for a status report, whose loss
    /// leaves the tab's records stale.
    Side { status: bool },
    /// Never queued.
    Ignored,
}

/// The events merged at the source, each with one waiting entry at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    Wakeup,
    /// The title, or its reset; one slot for both.
    Title,
    /// A remote host's report of another foreground program.
    Foreground,
    /// A clipboard write, one slot per selection.
    Clipboard(ClipboardType),
}

/// What the producer does with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Send it (for a merged kind, its marker).
    Queue,
    /// An entry of its kind is waiting already; its slot has the value.
    Merged,
    /// Past the high mark: send nothing.
    Dropped,
    /// Nothing waits for it: send nothing.
    Ignored,
}

/// Side events were dropped, and the queue is back under its low mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovered {
    pub dropped: u64,
    /// A status, progress or context report was among them.
    pub statuses: bool,
}

/// What `event` is to the gate (design, decision 2's table). It names every
/// variant of [`ChannelEvent`], [`Found`] and alacritty's event, so a new
/// one can't pass unclassified.
pub fn kind_of(event: &ChannelEvent) -> Kind {
    const SIDE: Kind = Kind::Side { status: false };
    match event {
        ChannelEvent::Alac(event) => match event {
            AlacEvent::Wakeup => Kind::Merged(Merge::Wakeup),
            AlacEvent::Title(_) | AlacEvent::ResetTitle => Kind::Merged(Merge::Title),
            AlacEvent::ClipboardStore(ty, _) => Kind::Merged(Merge::Clipboard(*ty)),
            AlacEvent::Exit | AlacEvent::ChildExit(_) => Kind::Life,
            AlacEvent::Bell
            | AlacEvent::PtyWrite(_)
            | AlacEvent::ColorRequest(..)
            | AlacEvent::TextAreaSizeRequest(_)
            | AlacEvent::ClipboardLoad(..) => SIDE,
            AlacEvent::MouseCursorDirty | AlacEvent::CursorBlinkingChange => Kind::Ignored,
        },
        ChannelEvent::Osc(found) => match found {
            Found::Notice(_) => SIDE,
            // A dropped progress report or context message leaves the
            // progress or the contexts stale, as a dropped status report
            // leaves the records: all three start over.
            Found::Status(_) | Found::Progress(_) | Found::Context(_) | Found::PromptStart | Found::Reset => {
                Kind::Side { status: true }
            }
        },
        ChannelEvent::Merged(merge) => Kind::Merged(*merge),
        ChannelEvent::Foreground => Kind::Merged(Merge::Foreground),
        ChannelEvent::Replay(_)
        | ChannelEvent::Lost
        | ChannelEvent::Failed
        | ChannelEvent::Broken(_)
        | ChannelEvent::InputRefused => Kind::Life,
    }
}

/// The gate's state: how many counted events wait, whether that is over
/// the marks, what was dropped, which merged kinds wait, and their values.
#[derive(Debug, Default)]
pub struct EventGate {
    counted: usize,
    over: bool,
    dropped: u64,
    dropped_status: bool,
    waiting: Waiting,
    /// The latest title: `Some(None)` is a reset, `None` nothing new.
    title: Option<Option<String>>,
    clipboard: Option<String>,
    selection: Option<String>,
}

/// Which merged kinds have an entry waiting.
#[derive(Debug, Default)]
struct Waiting {
    wakeup: bool,
    title: bool,
    foreground: bool,
    clipboard: bool,
    selection: bool,
}

impl Waiting {
    fn flag(&mut self, merge: Merge) -> &mut bool {
        match merge {
            Merge::Wakeup => &mut self.wakeup,
            Merge::Title => &mut self.title,
            Merge::Foreground => &mut self.foreground,
            Merge::Clipboard(ClipboardType::Clipboard) => &mut self.clipboard,
            Merge::Clipboard(ClipboardType::Selection) => &mut self.selection,
        }
    }
}

impl EventGate {
    /// What to queue for `event`, if anything: a merged event's value goes
    /// in its slot and its marker ([`ChannelEvent::Merged`]) is queued only
    /// when none waits; the others are queued, dropped or ignored as
    /// [`EventGate::admit`] says.
    pub fn pass(&mut self, event: ChannelEvent) -> Option<ChannelEvent> {
        let kind = kind_of(&event);
        let queued = match kind {
            Kind::Merged(merge) => {
                self.put(event);
                ChannelEvent::Merged(merge)
            }
            Kind::Life | Kind::Side { .. } | Kind::Ignored => event,
        };
        match self.admit(kind) {
            Admit::Queue => Some(queued),
            Admit::Merged | Admit::Dropped | Admit::Ignored => None,
        }
    }

    /// Puts a merged event's value in its slot. A wakeup, a foreground
    /// report and a marker carry none.
    fn put(&mut self, event: ChannelEvent) {
        match event {
            ChannelEvent::Alac(AlacEvent::Title(title)) => self.put_title(Some(title)),
            ChannelEvent::Alac(AlacEvent::ResetTitle) => self.put_title(None),
            ChannelEvent::Alac(AlacEvent::ClipboardStore(ty, text)) => self.put_clipboard(ty, text),
            ChannelEvent::Alac(_)
            | ChannelEvent::Osc(_)
            | ChannelEvent::Merged(_)
            | ChannelEvent::Foreground
            | ChannelEvent::Replay(_)
            | ChannelEvent::Lost
            | ChannelEvent::Failed
            | ChannelEvent::Broken(_)
            | ChannelEvent::InputRefused => {}
        }
    }

    /// Whether to queue an event of `kind`. A merged kind's value goes in
    /// its slot first ([`EventGate::pass`] does both).
    pub fn admit(&mut self, kind: Kind) -> Admit {
        match kind {
            Kind::Merged(merge) => {
                let waiting = self.waiting.flag(merge);
                if *waiting {
                    Admit::Merged
                } else {
                    *waiting = true;
                    Admit::Queue
                }
            }
            Kind::Life => {
                self.counted += 1;
                Admit::Queue
            }
            Kind::Side { status } => {
                self.over = EVENT_MARKS.over(self.over, self.counted);
                if self.over {
                    self.dropped += 1;
                    self.dropped_status |= status;
                    Admit::Dropped
                } else {
                    self.counted += 1;
                    Admit::Queue
                }
            }
            Kind::Ignored => Admit::Ignored,
        }
    }

    /// The consumer took a chunk of events of these kinds: they no longer
    /// wait. Returns what was dropped the first time the queue is back under
    /// its low mark after drops.
    pub fn took(&mut self, kinds: impl IntoIterator<Item = Kind>) -> Option<Recovered> {
        for kind in kinds {
            match kind {
                Kind::Merged(merge) => *self.waiting.flag(merge) = false,
                Kind::Life | Kind::Side { .. } => self.counted = self.counted.saturating_sub(1),
                Kind::Ignored => {}
            }
        }
        self.over = EVENT_MARKS.over(self.over, self.counted);
        if self.over || self.dropped == 0 {
            return None;
        }
        let recovered = Recovered {
            dropped: self.dropped,
            statuses: self.dropped_status,
        };
        self.dropped = 0;
        self.dropped_status = false;
        Some(recovered)
    }

    /// A title change (`None`: reset to the default), for a
    /// `Merged(Title)` entry.
    fn put_title(&mut self, title: Option<String>) {
        self.title = Some(title);
    }

    /// The latest title change since the last take: `Some(None)` is a
    /// reset, `None` nothing new.
    pub fn take_title(&mut self) -> Option<Option<String>> {
        self.title.take()
    }

    /// A clipboard write to `ty`, for a `Merged(Clipboard(ty))` entry.
    fn put_clipboard(&mut self, ty: ClipboardType, text: String) {
        *self.clipboard_slot(ty) = Some(text);
    }

    /// The latest write to `ty` since the last take.
    pub fn take_clipboard(&mut self, ty: ClipboardType) -> Option<String> {
        self.clipboard_slot(ty).take()
    }

    fn clipboard_slot(&mut self, ty: ClipboardType) -> &mut Option<String> {
        match ty {
            ClipboardType::Clipboard => &mut self.clipboard,
            ClipboardType::Selection => &mut self.selection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIGH: usize = 2048;
    const LOW: usize = 512;
    const SIDE: Kind = Kind::Side { status: false };
    const STATUS: Kind = Kind::Side { status: true };

    /// A gate with `n` side events queued.
    fn with_side_events(n: usize) -> EventGate {
        let mut gate = EventGate::default();
        for _ in 0..n {
            assert_eq!(gate.admit(SIDE), Admit::Queue);
        }
        gate
    }

    #[test]
    fn one_entry_waits_per_merge_kind_and_the_latest_value_wins() {
        let mut gate = EventGate::default();
        let clipboard = Merge::Clipboard(ClipboardType::Clipboard);
        let selection = Merge::Clipboard(ClipboardType::Selection);
        for merge in [
            Merge::Wakeup,
            Merge::Title,
            Merge::Foreground,
            clipboard,
            selection,
        ] {
            assert_eq!(gate.admit(Kind::Merged(merge)), Admit::Queue, "{merge:?}");
            assert_eq!(gate.admit(Kind::Merged(merge)), Admit::Merged, "{merge:?}");
        }

        gate.put_title(Some("first".into()));
        gate.put_title(Some("second".into()));
        gate.put_clipboard(ClipboardType::Clipboard, "a".into());
        gate.put_clipboard(ClipboardType::Selection, "b".into());
        gate.put_clipboard(ClipboardType::Clipboard, "c".into());

        // The consumer reaches the markers.
        let markers = [
            Merge::Wakeup,
            Merge::Title,
            Merge::Foreground,
            clipboard,
            selection,
        ];
        assert_eq!(gate.took(markers.map(Kind::Merged)), None);
        assert_eq!(gate.take_title(), Some(Some("second".into())));
        assert_eq!(gate.take_title(), None, "taken");
        assert_eq!(
            gate.take_clipboard(ClipboardType::Clipboard).as_deref(),
            Some("c")
        );
        assert_eq!(
            gate.take_clipboard(ClipboardType::Selection).as_deref(),
            Some("b")
        );
        assert_eq!(gate.take_clipboard(ClipboardType::Clipboard), None, "taken");

        // Once taken, the next one queues again; a reset is a title of `None`.
        assert_eq!(gate.admit(Kind::Merged(Merge::Title)), Admit::Queue);
        gate.put_title(Some("third".into()));
        gate.put_title(None);
        assert_eq!(gate.took([Kind::Merged(Merge::Title)]), None);
        assert_eq!(gate.take_title(), Some(None));

        // Merged entries don't count against the marks.
        let mut gate = with_side_events(HIGH - 1);
        for _ in 0..10 {
            gate.admit(Kind::Merged(Merge::Wakeup));
        }
        assert_eq!(
            gate.admit(SIDE),
            Admit::Queue,
            "the last one under the high mark"
        );
    }

    #[test]
    fn life_events_are_queued_past_the_high_mark() {
        let mut gate = with_side_events(HIGH);
        assert_eq!(gate.admit(SIDE), Admit::Dropped);
        for _ in 0..HIGH {
            assert_eq!(gate.admit(Kind::Life), Admit::Queue);
        }
        // They count: taking every side event leaves the queue over.
        assert_eq!(gate.took(vec![SIDE; HIGH]), None);
        assert_eq!(gate.admit(SIDE), Admit::Dropped);
        assert_eq!(
            gate.took(vec![Kind::Life; HIGH]),
            Some(Recovered {
                dropped: 2,
                statuses: false
            })
        );
        assert_eq!(gate.admit(SIDE), Admit::Queue);
    }

    #[test]
    fn side_events_are_dropped_from_the_high_mark_until_under_the_low_mark() {
        let mut gate = with_side_events(HIGH - 1);
        assert_eq!(gate.admit(SIDE), Admit::Queue, "the high mark's own");
        assert_eq!(gate.admit(SIDE), Admit::Dropped);
        assert_eq!(gate.admit(STATUS), Admit::Dropped);

        // Down to the low mark: still over.
        assert_eq!(gate.took(vec![SIDE; HIGH - LOW]), None);
        assert_eq!(gate.admit(SIDE), Admit::Dropped);
        // Under it.
        assert!(gate.took([SIDE]).is_some());
        assert_eq!(gate.admit(SIDE), Admit::Queue);
        // And it takes the high mark again to drop.
        let mut gate = with_side_events(LOW);
        for _ in LOW..HIGH {
            assert_eq!(gate.admit(STATUS), Admit::Queue);
        }
        assert_eq!(gate.admit(STATUS), Admit::Dropped);
    }

    #[test]
    fn recovery_is_reported_once_with_the_count_and_whether_a_status_was_dropped() {
        let mut gate = with_side_events(HIGH);
        for _ in 0..3 {
            assert_eq!(gate.admit(SIDE), Admit::Dropped);
        }
        assert_eq!(gate.took(vec![SIDE; HIGH - LOW]), None, "still over");
        assert_eq!(
            gate.took([SIDE]),
            Some(Recovered {
                dropped: 3,
                statuses: false
            })
        );
        assert_eq!(gate.took([SIDE]), None, "reported once");

        // A status report among the dropped is noted.
        let mut gate = with_side_events(HIGH);
        assert_eq!(gate.admit(SIDE), Admit::Dropped);
        assert_eq!(gate.admit(STATUS), Admit::Dropped);
        assert_eq!(
            gate.took(vec![SIDE; HIGH]),
            Some(Recovered {
                dropped: 2,
                statuses: true
            })
        );

        // Going over with nothing dropped reports nothing.
        let mut gate = with_side_events(HIGH);
        assert_eq!(gate.took(vec![SIDE; HIGH]), None);
    }

    #[test]
    fn ignored_events_are_never_queued() {
        let mut gate = with_side_events(HIGH - 1);
        for _ in 0..HIGH {
            assert_eq!(gate.admit(Kind::Ignored), Admit::Ignored);
        }
        assert_eq!(gate.admit(SIDE), Admit::Queue, "ignored events don't count");
        assert_eq!(gate.admit(SIDE), Admit::Dropped);
    }

    #[test]
    fn every_event_has_the_kind_the_design_gives_it() {
        use std::sync::Arc;

        use crate::notify::osc::{Notice, Source};
        use crate::terminal::status::Message;

        let clipboard = ClipboardType::Clipboard;
        let selection = ClipboardType::Selection;
        let alac = |event: AlacEvent| kind_of(&ChannelEvent::Alac(event));
        let found = |found: Found| kind_of(&ChannelEvent::Osc(found));

        assert_eq!(alac(AlacEvent::Wakeup), Kind::Merged(Merge::Wakeup));
        assert_eq!(alac(AlacEvent::Title("t".into())), Kind::Merged(Merge::Title));
        assert_eq!(alac(AlacEvent::ResetTitle), Kind::Merged(Merge::Title));
        assert_eq!(alac(AlacEvent::ClipboardStore(clipboard, "c".into())), Kind::Merged(Merge::Clipboard(clipboard)));
        assert_eq!(alac(AlacEvent::ClipboardStore(selection, "s".into())), Kind::Merged(Merge::Clipboard(selection)));
        assert_eq!(kind_of(&ChannelEvent::Foreground), Kind::Merged(Merge::Foreground));
        for merge in [Merge::Wakeup, Merge::Title, Merge::Foreground, Merge::Clipboard(selection)] {
            assert_eq!(kind_of(&ChannelEvent::Merged(merge)), Kind::Merged(merge), "a marker is its kind");
        }

        for life in [
            ChannelEvent::Replay(true),
            ChannelEvent::Replay(false),
            ChannelEvent::Lost,
            ChannelEvent::Failed,
            ChannelEvent::Broken("why".into()),
            ChannelEvent::InputRefused,
            ChannelEvent::Alac(AlacEvent::Exit),
            ChannelEvent::Alac(AlacEvent::ChildExit(std::process::ExitStatus::default())),
        ] {
            assert_eq!(kind_of(&life), Kind::Life, "{}", described(&life));
        }

        for side in [
            AlacEvent::Bell,
            AlacEvent::PtyWrite("reply".into()),
            AlacEvent::ColorRequest(0, Arc::new(|_| String::new())),
            AlacEvent::TextAreaSizeRequest(Arc::new(|_| String::new())),
            AlacEvent::ClipboardLoad(clipboard, Arc::new(|_| String::new())),
        ] {
            assert_eq!(alac(side.clone()), SIDE, "{side:?}");
        }
        let notice = Notice { source: Source::Osc9, title: "t".into(), body: "b".into() };
        assert_eq!(found(Found::Notice(notice)), SIDE);
        for status in [
            Found::Status(Message::Query),
            Found::Status(Message::Clear { id: None }),
            Found::Progress(brindle_term_core::progress::Report::Clear),
            Found::Context(brindle_term_core::context::parse(b"end=a").unwrap()),
            Found::PromptStart,
            Found::Reset,
        ] {
            assert_eq!(found(status), STATUS);
        }

        assert_eq!(alac(AlacEvent::MouseCursorDirty), Kind::Ignored);
        assert_eq!(alac(AlacEvent::CursorBlinkingChange), Kind::Ignored);
    }

    fn described(event: &ChannelEvent) -> String {
        match event {
            ChannelEvent::Alac(event) => format!("{event:?}"),
            ChannelEvent::Replay(active) => format!("Replay({active})"),
            ChannelEvent::Broken(why) => format!("Broken({why})"),
            ChannelEvent::Lost => "Lost".into(),
            ChannelEvent::Failed => "Failed".into(),
            ChannelEvent::InputRefused => "InputRefused".into(),
            ChannelEvent::Osc(_) | ChannelEvent::Merged(_) | ChannelEvent::Foreground => "other".into(),
        }
    }

    #[test]
    fn a_merged_event_queues_its_marker_once_and_its_slot_keeps_the_latest_value() {
        let mut gate = EventGate::default();
        let title = |title: &str| ChannelEvent::Alac(AlacEvent::Title(title.into()));
        assert!(matches!(gate.pass(title("one")), Some(ChannelEvent::Merged(Merge::Title))));
        assert!(gate.pass(title("two")).is_none(), "its marker waits");
        assert!(gate.pass(ChannelEvent::Alac(AlacEvent::Wakeup)).is_some());
        assert!(gate.pass(ChannelEvent::Alac(AlacEvent::Wakeup)).is_none());
        assert!(gate.pass(ChannelEvent::Alac(AlacEvent::MouseCursorDirty)).is_none());
        assert!(matches!(gate.pass(ChannelEvent::Lost), Some(ChannelEvent::Lost)));

        assert_eq!(gate.took([Kind::Merged(Merge::Title), Kind::Merged(Merge::Wakeup), Kind::Life]), None);
        assert_eq!(gate.take_title(), Some(Some("two".into())));
        assert!(matches!(gate.pass(ChannelEvent::Alac(AlacEvent::ResetTitle)), Some(ChannelEvent::Merged(Merge::Title))));
        assert_eq!(gate.take_title(), Some(None));
    }
}
