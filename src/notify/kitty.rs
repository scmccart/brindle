//! kitty's OSC 99 notifications, client side: putting a terminal's chunks
//! together into notifications ([`Assembly`]), the replies Brindle writes
//! back to the program, what a click or a close writes ([`activation`],
//! [`on_close`]), and which notifications are open ([`Open`]).
//!
//! Everything here is pure. `Terminal` owns its [`Assembly`] and writes the
//! replies; `Notifier` owns the [`Open`] table and asks what each click and
//! close does. Replies are built from checked ids ([`KittyId`]) and fixed
//! text only, so a program can't make Brindle echo other bytes.

use std::collections::{HashSet, VecDeque};

pub use brindle_term_core::kitty::{Chunk, Data, Expiry, KittyId, Meta, Occasion, Payload, Sound, Urgency};

use super::osc::{MAX_BODY_CHARS, MAX_TITLE_CHARS, clean};

/// Incomplete notifications a terminal holds; the oldest goes past this.
pub const MAX_PENDING: usize = 8;
/// Bytes of title, of body and of button labels kept per notification.
pub const MAX_TEXT: usize = 2048;
/// Buttons shown; later ones are dropped.
pub const MAX_BUTTONS: usize = 4;
/// Longest button label, in characters.
pub const MAX_LABEL_CHARS: usize = 64;
/// Open notifications remembered, across terminals; the oldest is forgotten
/// past this (it stays on the desktop, but no longer replaces or reports).
pub const MAX_OPEN: usize = 256;

/// What separates button labels.
const LINE_SEPARATOR: char = '\u{2028}';

/// A button label: cleaned of controls, not empty, at most
/// [`MAX_LABEL_CHARS`] characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label(String);

impl Label {
    pub fn new(text: &str) -> Option<Self> {
        let text = clean(text, MAX_LABEL_CHARS);
        (!text.is_empty()).then_some(Label(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A button's number, 1 to [`MAX_BUTTONS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button(u8);

impl Button {
    /// The button a sink's action key names (`"1"` to `"4"`).
    pub fn from_key(key: &str) -> Option<Self> {
        let n: u8 = key.parse().ok().filter(|_| key.len() == 1)?;
        (1..=MAX_BUTTONS as u8).contains(&n).then_some(Button(n))
    }

    /// The key a sink's action for button `index` (from 0) carries.
    pub fn key(index: usize) -> String {
        (index + 1).to_string()
    }

    pub fn number(self) -> u8 {
        self.0
    }
}

/// What a finished notification asks of the click, the close and the sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: Option<KittyId>,
    /// `a`'s `focus`: a click raises the window and shows the tab.
    pub focus: bool,
    /// `a`'s `report`: clicks and button presses are reported.
    pub report: bool,
    /// `c=1`: the close is reported.
    pub close_report: bool,
    pub urgency: Urgency,
    pub expiry: Expiry,
    pub sound: Sound,
    pub buttons: Vec<Label>,
}

/// A finished notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice99 {
    pub title: String,
    pub body: String,
    pub occasion: Occasion,
    pub program: Program,
}

/// What a chunk comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Held for more chunks.
    Pending,
    Show(Notice99),
    /// `p=close` of this id.
    Close(KittyId),
    /// `p=?`, with the id to answer under.
    Query(Option<KittyId>),
    /// `p=alive`, with the id to answer under.
    Alive(KittyId),
    /// Nothing to do: an icon, a notification with no text, a close or an
    /// alive with no id.
    Ignored,
}

/// One payload's bytes, decoded, with an unfinished base64 quad.
#[derive(Debug, Default)]
struct Text {
    bytes: Vec<u8>,
    carry: Vec<u8>,
}

impl Text {
    fn add(&mut self, data: Data) {
        match data {
            Data::Plain(bytes) => {
                self.flush();
                self.push(&bytes);
            }
            Data::Base64(encoded) => {
                self.carry.extend_from_slice(&encoded);
                let whole = self.carry.len() / 4 * 4;
                let quads: Vec<u8> = self.carry.drain(..whole).collect();
                for quad in quads.chunks(4) {
                    if let Some((bytes, len)) = decode_quad(quad) {
                        self.push(&bytes[..len]);
                    }
                }
            }
        }
    }

    /// Decodes what is left of a quad: the last chunk may leave its padding
    /// out.
    fn flush(&mut self) {
        let carry = std::mem::take(&mut self.carry);
        if let Some((bytes, len)) = decode_quad(&carry) {
            self.push(&bytes[..len]);
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let room = MAX_TEXT.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    /// The text, read as UTF-8: a character cut off by the cap goes, and
    /// other bad bytes become U+FFFD.
    fn finish(mut self) -> String {
        self.flush();
        let end = match std::str::from_utf8(&self.bytes) {
            Err(err) if err.error_len().is_none() => err.valid_up_to(),
            Ok(_) | Err(_) => self.bytes.len(),
        };
        String::from_utf8_lossy(&self.bytes[..end]).into_owned()
    }
}

/// The 6-bit value of one base64 character.
fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes one quad of 2 to 4 characters, padded or not, into up to three
/// bytes and their count. `None` for a quad that isn't one (padding before
/// a character, a lone character).
fn decode_quad(quad: &[u8]) -> Option<([u8; 3], usize)> {
    let chars = quad.iter().position(|&b| b == b'=').unwrap_or(quad.len());
    if chars < 2 || quad.len() > 4 || quad[chars..].iter().any(|&b| b != b'=') {
        return None;
    }
    let mut value: u32 = 0;
    for &byte in &quad[..chars] {
        value = value << 6 | u32::from(sextet(byte)?);
    }
    value <<= 6 * (4 - chars as u32);
    let [_, a, b, c] = value.to_be_bytes();
    Some(([a, b, c], chars - 1))
}

/// A notification being put together.
#[derive(Debug)]
struct Pending {
    id: Option<KittyId>,
    meta: Meta,
    title: Text,
    body: Text,
    buttons: Text,
}

impl Pending {
    fn new(id: Option<KittyId>) -> Self {
        Self { id, meta: Meta::default(), title: Text::default(), body: Text::default(), buttons: Text::default() }
    }

    /// The finished notification, or `None` when it has no text: a
    /// notification with no title takes its body as the title.
    fn finish(self) -> Option<Notice99> {
        let title = clean(&self.title.finish(), MAX_TITLE_CHARS);
        let body = clean(&self.body.finish(), MAX_BODY_CHARS);
        let (title, body) = if title.is_empty() { (clean(&body, MAX_TITLE_CHARS), String::new()) } else { (title, body) };
        if title.is_empty() {
            return None;
        }
        let buttons = self.buttons.finish();
        let buttons = buttons.split(LINE_SEPARATOR).filter_map(Label::new).take(MAX_BUTTONS).collect();
        let meta = self.meta;
        Some(Notice99 {
            title,
            body,
            occasion: meta.occasion.unwrap_or(Occasion::Always),
            program: Program {
                id: self.id,
                focus: meta.focus.unwrap_or(true),
                report: meta.report.unwrap_or(false),
                close_report: meta.close_report.unwrap_or(false),
                urgency: meta.urgency.unwrap_or(Urgency::Normal),
                expiry: meta.expiry.unwrap_or(Expiry::Default),
                sound: meta.sound.unwrap_or(Sound::Default),
                buttons,
            },
        })
    }
}

/// One terminal's incomplete notifications, oldest first, at most
/// [`MAX_PENDING`]; the unnamed one is the one with no id.
#[derive(Debug, Default)]
pub struct Assembly {
    pending: VecDeque<Pending>,
}

impl Assembly {
    /// Applies one chunk from the terminal's output.
    pub fn feed(&mut self, chunk: Chunk) -> Step {
        /// Which of a notification's texts a chunk adds to.
        enum Part {
            Title,
            Body,
            Buttons,
            /// An icon or an unknown type: its metadata counts, its data
            /// doesn't.
            Skipped,
        }
        let Chunk { id, payload, done, data, meta } = chunk;
        let part = match payload {
            Payload::Query => return Step::Query(id),
            Payload::Alive => return id.map_or(Step::Ignored, Step::Alive),
            Payload::Close => return id.map_or(Step::Ignored, Step::Close),
            Payload::Title => Part::Title,
            Payload::Body => Part::Body,
            Payload::Buttons => Part::Buttons,
            Payload::Unsupported => Part::Skipped,
        };
        let mut pending = match self.pending.iter().position(|p| p.id == id) {
            Some(at) => self.pending.remove(at).unwrap_or_else(|| Pending::new(id)),
            None => Pending::new(id),
        };
        pending.meta = pending.meta.then(meta);
        match part {
            Part::Title => pending.title.add(data),
            Part::Body => pending.body.add(data),
            Part::Buttons => pending.buttons.add(data),
            Part::Skipped => {}
        }
        if done {
            return pending.finish().map_or(Step::Ignored, Step::Show);
        }
        self.pending.push_back(pending);
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
        }
        Step::Pending
    }

    /// How many notifications wait for more chunks.
    #[cfg(test)]
    fn pending(&self) -> usize {
        self.pending.len()
    }
}

/// What the platform's notification service can do, for the query's reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// Urgency and expiry (`u`, `w`).
    pub hints: bool,
    /// Buttons are shown.
    pub buttons: bool,
}

/// The id a reply carries: the notification's, or `0` for one without.
fn reply_id(id: Option<&KittyId>) -> &str {
    id.map_or("0", KittyId::as_str)
}

/// The reply to `p=?`, in kitty's order of keys.
pub fn reply_query(id: Option<&KittyId>, caps: Caps) -> String {
    let mut reply = format!("\x1b]99;i={}:p=?;a=focus,report:o=always,unfocused,invisible", reply_id(id));
    if caps.hints {
        reply.push_str(":u=0,1,2");
    }
    reply.push_str(":p=title,body,?,close,alive");
    if caps.buttons {
        reply.push_str(",buttons");
    }
    reply.push_str(":c=1");
    if caps.hints {
        reply.push_str(":w=1");
    }
    reply.push_str(":s=system,silent\x1b\\");
    reply
}

/// The reply to `p=alive`: the terminal's open notifications' ids.
pub fn reply_alive(id: &KittyId, open: &[KittyId]) -> String {
    let ids: Vec<&str> = open.iter().map(KittyId::as_str).collect();
    format!("\x1b]99;i={}:p=alive;{}\x1b\\", id.as_str(), ids.join(","))
}

/// A click on the notification, or a press of one of its buttons.
pub fn click_report(id: Option<&KittyId>, button: Option<Button>) -> String {
    let button = button.map(|b| b.number().to_string()).unwrap_or_default();
    format!("\x1b]99;i={};{button}\x1b\\", reply_id(id))
}

/// The notification closed.
pub fn close_report(id: Option<&KittyId>) -> String {
    format!("\x1b]99;i={}:p=close;\x1b\\", reply_id(id))
}

/// What a sink's action on a notification does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activation {
    /// Show the tab and raise its window.
    pub raise: bool,
    /// Written to the notification's terminal.
    pub write: Option<String>,
}

/// The action `key` (the body's `"default"`, or a button's number) on a
/// notification, OSC 99's (`program`) or another. A body click raises
/// unless the program turned `focus` off; a button never raises. Only
/// `report` writes anything (kitty's rule, buttons included).
pub fn activation(program: Option<&Program>, key: &str) -> Activation {
    let body = key == "default";
    let Some(program) = program else {
        return Activation { raise: body, write: None };
    };
    if body {
        let write = program.report.then(|| click_report(program.id.as_ref(), None));
        return Activation { raise: program.focus, write };
    }
    let write = Button::from_key(key)
        .filter(|b| usize::from(b.number()) <= program.buttons.len() && program.report)
        .map(|b| click_report(program.id.as_ref(), Some(b)));
    Activation { raise: false, write }
}

/// What the close of a notification writes: its close report, when it
/// asked for one.
pub fn on_close(program: Option<&Program>) -> Option<String> {
    program.filter(|p| p.close_report).map(|p| close_report(p.id.as_ref()))
}

/// Where an open notification stands with its sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Sent with this key, waiting for the sink's id.
    Sent(u64),
    /// Shown under this sink id.
    Shown(u32),
}

/// What sending an update does to the notification it updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replace {
    /// Nothing shown to replace: a new notification (any earlier one still
    /// waiting for its id is closed once it has one).
    Nothing,
    /// Replace this sink id.
    Id(u32),
}

/// What the sink's id for a sent key means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// Remember it.
    Keep,
    /// An update or a close came first: close it, and forget it.
    Close,
}

/// What a program's `p=close` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    /// No open notification has that id.
    Unknown,
    /// Close this sink id.
    Now(u32),
    /// Not shown yet (sent as this key): it is closed once it is.
    WhenShown(u64),
}

/// The open OSC 99 notifications that have ids, by terminal (`K`, the
/// terminal's entity id) and id, oldest first; and the sent ones to close
/// once their sink names them. At most [`MAX_OPEN`] of each.
#[derive(Debug)]
pub struct Open<K> {
    slots: VecDeque<((K, KittyId), Slot)>,
    doomed: HashSet<u64>,
    doomed_order: VecDeque<u64>,
}

impl<K> Default for Open<K> {
    fn default() -> Self {
        Self { slots: VecDeque::new(), doomed: HashSet::new(), doomed_order: VecDeque::new() }
    }
}

impl<K: Copy + Eq> Open<K> {
    fn find(&self, terminal: K, id: &KittyId) -> Option<usize> {
        self.slots.iter().position(|((k, i), _)| *k == terminal && i == id)
    }

    fn doom(&mut self, key: u64) {
        if self.doomed.insert(key) {
            self.doomed_order.push_back(key);
        }
        while self.doomed_order.len() > MAX_OPEN {
            if let Some(old) = self.doomed_order.pop_front() {
                self.doomed.remove(&old);
            }
        }
    }

    /// A notification of `terminal` with `id` is sent as `key`: what it
    /// replaces. An update keeps the notification's place in the order.
    pub fn send(&mut self, terminal: K, id: &KittyId, key: u64) -> Replace {
        match self.find(terminal, id) {
            Some(at) => {
                let replace = match self.slots[at].1 {
                    Slot::Shown(sink) => Replace::Id(sink),
                    Slot::Sent(earlier) => {
                        self.doom(earlier);
                        Replace::Nothing
                    }
                };
                self.slots[at].1 = Slot::Sent(key);
                replace
            }
            None => {
                self.slots.push_back(((terminal, id.clone()), Slot::Sent(key)));
                while self.slots.len() > MAX_OPEN {
                    self.slots.pop_front();
                }
                Replace::Nothing
            }
        }
    }

    /// The sink showed `key` as `sink`.
    pub fn shown(&mut self, key: u64, sink: u32) -> Arrival {
        if self.doomed.remove(&key) {
            self.doomed_order.retain(|&k| k != key);
            return Arrival::Close;
        }
        if let Some(slot) = self.slots.iter_mut().find(|(_, slot)| *slot == Slot::Sent(key)) {
            slot.1 = Slot::Shown(sink);
        }
        Arrival::Keep
    }

    /// The program closes its notification `id`.
    pub fn close(&mut self, terminal: K, id: &KittyId) -> Closing {
        let Some(at) = self.find(terminal, id) else { return Closing::Unknown };
        match self.slots.remove(at).map(|(_, slot)| slot) {
            Some(Slot::Shown(sink)) => Closing::Now(sink),
            Some(Slot::Sent(key)) => {
                self.doom(key);
                Closing::WhenShown(key)
            }
            None => Closing::Unknown,
        }
    }

    /// The sink says `sink` closed.
    pub fn closed(&mut self, sink: u32) {
        self.slots.retain(|(_, slot)| *slot != Slot::Shown(sink));
    }

    /// `terminal`'s open notifications' ids, oldest first.
    pub fn alive(&self, terminal: K) -> Vec<KittyId> {
        self.slots.iter().filter(|((k, _), _)| *k == terminal).map(|((_, id), _)| id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn chunk(body: &str) -> Chunk {
        brindle_term_core::kitty::parse(body.as_bytes()).unwrap_or_else(|| panic!("{body:?} should parse"))
    }

    fn id(text: &str) -> KittyId {
        KittyId::parse(text.as_bytes()).unwrap()
    }

    fn feed(assembly: &mut Assembly, bodies: &[&str]) -> Vec<Step> {
        bodies.iter().map(|b| assembly.feed(chunk(b))).collect()
    }

    fn shown(step: &Step) -> &Notice99 {
        match step {
            Step::Show(notice) => notice,
            other => panic!("not shown: {other:?}"),
        }
    }

    #[test]
    fn the_simplest_form_shows_at_once_with_the_defaults() {
        let steps = feed(&mut Assembly::default(), &[";Hello world"]);
        assert_eq!(
            steps,
            [Step::Show(Notice99 {
                title: "Hello world".into(),
                body: String::new(),
                occasion: Occasion::Always,
                program: Program {
                    id: None,
                    focus: true,
                    report: false,
                    close_report: false,
                    urgency: Urgency::Normal,
                    expiry: Expiry::Default,
                    sound: Sound::Default,
                    buttons: Vec::new(),
                },
            })]
        );
    }

    #[test]
    fn title_and_body_in_chunks() {
        let mut assembly = Assembly::default();
        let steps = feed(&mut assembly, &["i=b:d=0;Build", "i=b:d=0:p=body;3 failed", "i=b:p=body;, 9 passed"]);
        assert_eq!(steps[..2], [Step::Pending, Step::Pending]);
        let notice = shown(&steps[2]);
        assert_eq!((notice.title.as_str(), notice.body.as_str()), ("Build", "3 failed, 9 passed"));
        assert_eq!(notice.program.id, Some(id("b")));
        assert_eq!(assembly.pending(), 0);
    }

    #[test]
    fn metadata_from_any_chunk_applies_the_latest_winning() {
        let mut assembly = Assembly::default();
        let steps = feed(&mut assembly, &["i=m:d=0:a=-focus:u=2:c=1;T", "i=m:a=report:u=0:o=unfocused:p=body;B"]);
        let notice = shown(&steps[1]);
        assert!(!notice.program.focus && notice.program.report && notice.program.close_report);
        assert_eq!(notice.program.urgency, Urgency::Low);
        assert_eq!(notice.occasion, Occasion::Unfocused);
    }

    #[test]
    fn a_body_alone_is_the_title_and_no_text_shows_nothing() {
        let steps = feed(&mut Assembly::default(), &["p=body;just a body", "p=body;", ";", "i=x:p=buttons;A"]);
        let notice = shown(&steps[0]);
        assert_eq!((notice.title.as_str(), notice.body.as_str()), ("just a body", ""));
        assert_eq!(steps[1..], [Step::Ignored, Step::Ignored, Step::Ignored]);
    }

    #[test]
    fn interleaved_notifications_keep_apart() {
        let mut assembly = Assembly::default();
        let steps = feed(&mut assembly, &["i=a:d=0;A1", "d=0;N", "i=b:d=0;B", "i=a;A2", ";!", "i=b:p=body;b"]);
        assert_eq!(shown(&steps[3]).title, "A1A2");
        assert_eq!(shown(&steps[4]).title, "N!");
        assert_eq!((shown(&steps[5]).title.as_str(), shown(&steps[5]).body.as_str()), ("B", "b"));
    }

    #[test]
    fn base64_cut_anywhere_padded_or_not() {
        // Cut after encoding, mid-quad.
        let steps = feed(&mut Assembly::default(), &["i=s:d=0:e=1;SGV", "i=s:e=1;sbG8="]);
        assert_eq!(shown(&steps[1]).title, "Hello");
        // Cut before encoding, each piece padded.
        let steps = feed(&mut Assembly::default(), &["i=u:d=0:e=1;SGk=", "i=u:e=1;IQ=="]);
        assert_eq!(shown(&steps[1]).title, "Hi!");
        // The last piece without its padding.
        let steps = feed(&mut Assembly::default(), &["e=1;SGk"]);
        assert_eq!(shown(&steps[0]).title, "Hi");
        // Plain after unpadded base64 on the same payload (`w6k` is `é`),
        // and a character split across two plain chunks.
        let steps = feed(&mut Assembly::default(), &["d=0:e=1;w6k", "; ok"]);
        assert_eq!(shown(&steps[1]).title, "é ok");
        let mut assembly = Assembly::default();
        let mut split = b"d=0;caf".to_vec();
        split.push(0xc3);
        assembly.feed(brindle_term_core::kitty::parse(&split).unwrap());
        let mut rest = b";".to_vec();
        rest.extend_from_slice(&[0xa9, b'!']);
        assert_eq!(shown(&assembly.feed(brindle_term_core::kitty::parse(&rest).unwrap())).title, "café!");
    }

    #[test]
    fn decoded_text_is_cleaned() {
        // `\n` and `\x07` inside base64, which the scanner never sees.
        let steps = feed(&mut Assembly::default(), &["e=1;YQpiB2M="]);
        assert_eq!(shown(&steps[0]).title, "abc");
        // Bad UTF-8 becomes U+FFFD.
        let steps = feed(&mut Assembly::default(), &["e=1;/w=="]);
        assert_eq!(shown(&steps[0]).title, "\u{fffd}");
    }

    #[test]
    fn quads() {
        assert_eq!(decode_quad(b"SGVs"), Some((*b"Hel", 3)));
        assert_eq!(decode_quad(b"SGk="), Some(([b'H', b'i', 0], 2)));
        assert_eq!(decode_quad(b"SGk"), Some(([b'H', b'i', 0], 2)));
        assert_eq!(decode_quad(b"IQ=="), Some(([b'!', 0, 0], 1)));
        assert_eq!(decode_quad(b"IQ"), Some(([b'!', 0, 0], 1)));
        for bad in [&b""[..], b"I", b"I===", b"=IQ=", b"I=Q=", b"SG!s", b"SGVsb"] {
            assert_eq!(decode_quad(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn title_and_body_are_capped() {
        let mut assembly = Assembly::default();
        let piece = "x".repeat(1500);
        for _ in 0..3 {
            assembly.feed(chunk(&format!("i=c:d=0;{piece}")));
            assembly.feed(chunk(&format!("i=c:d=0:p=body;{piece}")));
        }
        let notice = assembly.feed(chunk("i=c;"));
        let notice = shown(&notice);
        assert_eq!(notice.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(notice.body.chars().count(), MAX_BODY_CHARS);
        // A character the byte cap cuts goes whole.
        let mut text = Text::default();
        text.add(Data::Plain("a".repeat(MAX_TEXT - 1).into_bytes()));
        text.add(Data::Plain("é".into()));
        assert_eq!(text.finish(), "a".repeat(MAX_TEXT - 1));
    }

    #[test]
    fn the_oldest_incomplete_notification_goes_past_eight() {
        let mut assembly = Assembly::default();
        for n in 0..=MAX_PENDING {
            assert_eq!(assembly.feed(chunk(&format!("i=n{n}:d=0;title {n}"))), Step::Pending);
        }
        assert_eq!(assembly.pending(), MAX_PENDING);
        // n0 was dropped: its next chunk starts over.
        assert_eq!(shown(&assembly.feed(chunk("i=n0;again"))).title, "again");
        assert_eq!(shown(&assembly.feed(chunk("i=n1;!"))).title, "title 1!");
    }

    #[test]
    fn closes_queries_and_alives() {
        let mut assembly = Assembly::default();
        let steps = feed(&mut assembly, &["i=n1:p=close;", "p=close;", "i=q:p=?;", "p=?;", "i=x:p=alive;", "p=alive;"]);
        assert_eq!(
            steps,
            [
                Step::Close(id("n1")),
                Step::Ignored,
                Step::Query(Some(id("q"))),
                Step::Query(None),
                Step::Alive(id("x")),
                Step::Ignored
            ]
        );
    }

    #[test]
    fn icons_and_unknown_payloads_are_skipped() {
        let mut assembly = Assembly::default();
        let steps = feed(&mut assembly, &["i=x:d=0:p=icon;data", "i=x:d=0:p=bogus;more", "i=x;Done"]);
        assert_eq!(steps[..2], [Step::Pending, Step::Pending]);
        assert_eq!(shown(&steps[2]).title, "Done");
    }

    #[test]
    fn buttons_split_on_line_separators_with_limits() {
        let labels = ["Allow", "", "Deny", "  ", "Three", "Four", "Five"].join("\u{2028}");
        let mut assembly = Assembly::default();
        let steps = feed(&mut assembly, &[&format!("i=p:d=0:p=buttons;{labels}"), "i=p;Run cargo test?"]);
        let buttons: Vec<&str> = shown(&steps[1]).program.buttons.iter().map(Label::as_str).collect();
        assert_eq!(buttons, ["Allow", "Deny", "Three", "Four"]);
        // Long labels are cut; base64 labels and chunked ones join.
        let long = "L".repeat(100);
        let steps = feed(&mut Assembly::default(), &[&format!("i=l:d=0:p=buttons;{long}"), "i=l;t"]);
        assert_eq!(shown(&steps[1]).program.buttons[0].as_str().chars().count(), MAX_LABEL_CHARS);
        // "Yes No" in base64.
        let steps = feed(&mut Assembly::default(), &["i=e:d=0:p=buttons:e=1;WWVz4oCoTm8=", "i=e;t"]);
        let buttons: Vec<&str> = shown(&steps[1]).program.buttons.iter().map(Label::as_str).collect();
        assert_eq!(buttons, ["Yes", "No"]);
    }

    #[test]
    fn an_update_without_buttons_has_none() {
        let mut assembly = Assembly::default();
        let first = feed(&mut assembly, &["i=p:d=0:p=buttons;A\u{2028}B", "i=p;Ask"]);
        assert_eq!(shown(&first[1]).program.buttons.len(), 2);
        let update = feed(&mut assembly, &["i=p;Asked"]);
        assert!(shown(&update[0]).program.buttons.is_empty());
    }

    #[test]
    fn hints_and_sounds_reach_the_notice() {
        let steps = feed(&mut Assembly::default(), &["u=2:w=0:s=c2lsZW50;Disk full"]);
        let program = &shown(&steps[0]).program;
        assert_eq!((program.urgency, program.expiry, program.sound), (Urgency::Critical, Expiry::Never, Sound::Silent));
        let steps = feed(&mut Assembly::default(), &["w=250;x"]);
        assert_eq!(shown(&steps[0]).program.expiry, Expiry::After(Duration::from_millis(250)));
    }

    const LINUX: Caps = Caps { hints: true, buttons: true };
    const WINDOWS: Caps = Caps { hints: false, buttons: true };

    #[test]
    fn query_replies_byte_for_byte() {
        assert_eq!(
            reply_query(Some(&id("q")), LINUX),
            "\x1b]99;i=q:p=?;a=focus,report:o=always,unfocused,invisible:u=0,1,2:p=title,body,?,close,alive,buttons:c=1:w=1:s=system,silent\x1b\\"
        );
        assert_eq!(
            reply_query(Some(&id("q")), Caps { hints: true, buttons: false }),
            "\x1b]99;i=q:p=?;a=focus,report:o=always,unfocused,invisible:u=0,1,2:p=title,body,?,close,alive:c=1:w=1:s=system,silent\x1b\\"
        );
        assert_eq!(
            reply_query(Some(&id("q")), WINDOWS),
            "\x1b]99;i=q:p=?;a=focus,report:o=always,unfocused,invisible:p=title,body,?,close,alive,buttons:c=1:s=system,silent\x1b\\"
        );
        assert!(reply_query(None, LINUX).starts_with("\x1b]99;i=0:p=?;"));
    }

    #[test]
    fn reports_byte_for_byte() {
        assert_eq!(reply_alive(&id("x"), &[id("a"), id("b")]), "\x1b]99;i=x:p=alive;a,b\x1b\\");
        assert_eq!(reply_alive(&id("x"), &[]), "\x1b]99;i=x:p=alive;\x1b\\");
        assert_eq!(click_report(Some(&id("r")), None), "\x1b]99;i=r;\x1b\\");
        assert_eq!(click_report(None, None), "\x1b]99;i=0;\x1b\\");
        assert_eq!(click_report(Some(&id("p")), Button::from_key("2")), "\x1b]99;i=p;2\x1b\\");
        assert_eq!(close_report(Some(&id("d"))), "\x1b]99;i=d:p=close;\x1b\\");
        assert_eq!(close_report(None), "\x1b]99;i=0:p=close;\x1b\\");
    }

    #[test]
    fn button_keys() {
        assert_eq!(Button::from_key("1").map(Button::number), Some(1));
        assert_eq!(Button::from_key("4").map(Button::number), Some(4));
        for bad in ["0", "5", "default", "", "01", "+1", "-1", "1 "] {
            assert_eq!(Button::from_key(bad), None, "{bad:?}");
        }
        assert_eq!(Button::key(0), "1");
    }

    fn program(focus: bool, report: bool, buttons: usize) -> Program {
        Program {
            id: Some(id("p")),
            focus,
            report,
            close_report: false,
            urgency: Urgency::Normal,
            expiry: Expiry::Default,
            sound: Sound::Default,
            buttons: (0..buttons).map(|n| Label::new(&format!("b{n}")).unwrap()).collect(),
        }
    }

    #[test]
    fn what_a_click_does() {
        // Other sources: the body click raises, nothing is written.
        assert_eq!(activation(None, "default"), Activation { raise: true, write: None });
        assert_eq!(activation(None, "1"), Activation { raise: false, write: None });
        // OSC 99's body click: raise unless `-focus`, report with `report`.
        assert_eq!(activation(Some(&program(true, false, 0)), "default"), Activation { raise: true, write: None });
        assert_eq!(
            activation(Some(&program(false, true, 0)), "default"),
            Activation { raise: false, write: Some("\x1b]99;i=p;\x1b\\".into()) }
        );
        // A button never raises, and reports only with `report`.
        assert_eq!(
            activation(Some(&program(true, true, 2)), "2"),
            Activation { raise: false, write: Some("\x1b]99;i=p;2\x1b\\".into()) }
        );
        assert_eq!(activation(Some(&program(true, false, 2)), "2"), Activation { raise: false, write: None });
        // A key past its buttons, or no button at all, does nothing.
        assert_eq!(activation(Some(&program(true, true, 2)), "3"), Activation { raise: false, write: None });
        assert_eq!(activation(Some(&program(true, true, 2)), "bogus"), Activation { raise: false, write: None });
    }

    #[test]
    fn what_a_close_writes() {
        assert_eq!(on_close(None), None);
        assert_eq!(on_close(Some(&program(true, true, 0))), None);
        let asked = Program { close_report: true, ..program(true, false, 0) };
        assert_eq!(on_close(Some(&asked)).as_deref(), Some("\x1b]99;i=p:p=close;\x1b\\"));
        let unnamed = Program { id: None, ..asked };
        assert_eq!(on_close(Some(&unnamed)).as_deref(), Some("\x1b]99;i=0:p=close;\x1b\\"));
    }

    #[test]
    fn an_update_replaces_the_shown_one_in_its_place() {
        let mut open: Open<u8> = Open::default();
        assert_eq!(open.send(1, &id("a"), 10), Replace::Nothing);
        assert_eq!(open.shown(10, 100), Arrival::Keep);
        assert_eq!(open.send(1, &id("b"), 11), Replace::Nothing);
        assert_eq!(open.shown(11, 101), Arrival::Keep);
        assert_eq!(open.send(1, &id("a"), 12), Replace::Id(100));
        assert_eq!(open.shown(12, 100), Arrival::Keep);
        assert_eq!(open.alive(1), [id("a"), id("b")]);
        // Ids are per terminal.
        assert_eq!(open.send(2, &id("a"), 13), Replace::Nothing);
        assert_eq!(open.alive(2), [id("a")]);
    }

    #[test]
    fn an_update_before_the_first_is_shown_closes_the_first_when_it_is() {
        let mut open: Open<u8> = Open::default();
        open.send(1, &id("a"), 10);
        assert_eq!(open.send(1, &id("a"), 11), Replace::Nothing);
        assert_eq!(open.shown(10, 100), Arrival::Close);
        assert_eq!(open.shown(11, 101), Arrival::Keep);
        assert_eq!(open.alive(1), [id("a")]);
        assert_eq!(open.close(1, &id("a")), Closing::Now(101));
    }

    #[test]
    fn closes_by_the_program_and_by_the_sink() {
        let mut open: Open<u8> = Open::default();
        assert_eq!(open.close(1, &id("a")), Closing::Unknown);
        open.send(1, &id("a"), 10);
        open.shown(10, 100);
        open.send(1, &id("b"), 11);
        // Not shown yet: closed once it is.
        assert_eq!(open.close(1, &id("b")), Closing::WhenShown(11));
        assert_eq!(open.shown(11, 101), Arrival::Close);
        assert_eq!(open.close(2, &id("a")), Closing::Unknown, "another terminal's");
        assert_eq!(open.close(1, &id("a")), Closing::Now(100));
        assert!(open.alive(1).is_empty());
        // The sink's own close.
        open.send(1, &id("c"), 12);
        open.shown(12, 102);
        open.closed(102);
        assert!(open.alive(1).is_empty());
        // Once closed, the id starts over.
        assert_eq!(open.send(1, &id("c"), 13), Replace::Nothing);
    }

    #[test]
    fn the_table_stays_bounded() {
        let mut open: Open<u8> = Open::default();
        for n in 0..(MAX_OPEN as u64 + 10) {
            open.send(1, &id(&format!("n{n}")), n);
        }
        assert_eq!(open.alive(1).len(), MAX_OPEN);
        assert_eq!(open.alive(1)[0], id("n10"));
        let mut open: Open<u8> = Open::default();
        for n in 0..(MAX_OPEN as u64 + 10) {
            open.send(1, &id("same"), n);
        }
        assert!(open.doomed.len() <= MAX_OPEN && open.doomed_order.len() <= MAX_OPEN);
    }
}
