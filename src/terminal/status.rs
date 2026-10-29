//! The Program Status Protocol (OSC 7501): a program tells the terminal
//! what it is doing (idle, working, waiting on the user, finished, or
//! failed) and why.
//!
//! [`parse`] reads the body of one report, applying every limit and discard
//! rule of the specification (revision 0.1,
//! <https://gist.github.com/mitchellh/7acae3abd8355c1c00287d67e96c913a>),
//! and [`Records`] holds what one terminal stores as a result, with the
//! lifetime rules. Both are pure; the `Terminal` entity drives them.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

use crate::notify::osc::{Notice, Source};

/// What a record says its program is doing, least pressing first: a tab
/// shows the greatest state among its records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// At rest, waiting for the user's next instruction.
    Idle,
    Working,
    /// Finished a piece of work the user hasn't looked at yet.
    Done,
    /// Failed and stopped.
    Error,
    /// Cannot continue until the user does something.
    Blocked,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Error => "error",
            State::Blocked => "blocked",
        }
    }

    /// Whether the state ends at the next shell prompt or process exit.
    fn is_activity(self) -> bool {
        matches!(self, State::Working | State::Blocked)
    }

    /// Whether the state is a result that waits for the user to see it.
    fn is_result(self) -> bool {
        matches!(self, State::Done | State::Error)
    }
}

/// What a `blocked` program is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Approval to do something.
    Permission,
    /// An answer the user must type.
    Question,
    /// A login, token or credential.
    Auth,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Permission => "permission",
            Kind::Question => "question",
            Kind::Auth => "auth",
        }
    }
}

/// One record: exactly the keys of the last report that used its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub state: State,
    pub kind: Option<Kind>,
    /// 0 to 100.
    pub progress: Option<u8>,
    /// A stable machine-readable program name (`cargo`, `claude-code`).
    pub app: Option<String>,
    /// A short label for the record, for programs that report several.
    pub title: Option<String>,
    /// One line: what the record is doing, waiting for, or finished.
    pub msg: Option<String>,
}

/// One parsed OSC 7501 body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// `?`: the program asks whether the protocol is supported.
    Query,
    /// `state=clear`: remove the record with this id and everything beneath
    /// it, or every record when there is no id.
    Clear { id: Option<String> },
    /// A report for the record `id` (the root is `""`).
    Report { id: String, record: Record },
}

/// The reply to [`Message::Query`].
pub const QUERY_REPLY: &[u8] = b"\x1b]7501;?\x1b\\";

/// The whole sequence is capped at 4096 bytes; `ESC ] 7501 ;` and `ESC \`
/// take nine of them.
const MAX_BODY: usize = 4096 - 9;
const MAX_KEY: usize = 16;
const MAX_MSG_ENCODED: usize = 2732;
const MAX_MSG: usize = 2048;
const MAX_TITLE_ENCODED: usize = 256;
const MAX_TITLE: usize = 192;
const MAX_APP: usize = 32;
const MAX_ID: usize = 128;
const MAX_SEGMENT: usize = 32;
const MAX_DEPTH: usize = 8;
/// Records per terminal; the least recently updated one goes past this.
pub const MAX_RECORDS: usize = 256;

/// Standard base64, padding optional.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Parses the body of an OSC 7501 (after `7501;`). `None` is a report the
/// specification says to ignore or discard whole.
pub fn parse(body: &[u8]) -> Option<Message> {
    if body.len() > MAX_BODY {
        return None;
    }
    let body = body.trim_ascii();
    if body == b"?" {
        return Some(Message::Query);
    }
    let mut state = None;
    let mut id = None;
    let mut kind = None;
    let mut progress = None;
    let mut app = None;
    let mut title = None;
    let mut msg = None;
    for pair in body.split(|&b| b == b':') {
        let Some(eq) = pair.iter().position(|&b| b == b'=') else { continue };
        let key = pair[..eq].trim_ascii();
        let value = pair[eq + 1..].trim_ascii();
        if key.len() > MAX_KEY {
            return None;
        }
        if key.is_empty() || !key.iter().all(u8::is_ascii_lowercase) || !value.iter().all(|&b| is_value_byte(b)) {
            continue;
        }
        // The last of a repeated key wins; unknown keys are ignored.
        match key {
            b"state" => state = Some(value),
            b"id" => id = Some(value),
            b"kind" => kind = Some(value),
            b"progress" => progress = Some(value),
            b"app" => app = Some(value),
            b"title" => title = Some(value),
            b"msg" => msg = Some(value),
            _ => {}
        }
    }
    let id = match id {
        Some(id) => Some(parse_id(id)?),
        None => None,
    };
    let state = match state? {
        b"idle" => State::Idle,
        b"working" => State::Working,
        b"done" => State::Done,
        b"blocked" => State::Blocked,
        b"error" => State::Error,
        b"clear" => return Some(Message::Clear { id }),
        _ => return None,
    };
    let app = match app {
        Some(app) => Some(parse_app(app)?),
        None => None,
    };
    let title = match title {
        Some(t) => decode_text(t, MAX_TITLE_ENCODED, MAX_TITLE)?,
        None => None,
    };
    let msg = match msg {
        Some(m) => decode_text(m, MAX_MSG_ENCODED, MAX_MSG)?,
        None => None,
    };
    // Unrecognized or misplaced values are treated as absent.
    let kind = match kind.filter(|_| state == State::Blocked) {
        Some(b"permission") => Some(Kind::Permission),
        Some(b"question") => Some(Kind::Question),
        Some(b"auth") => Some(Kind::Auth),
        _ => None,
    };
    let progress = progress
        .filter(|_| state.is_activity())
        .and_then(|p| std::str::from_utf8(p).ok()?.parse::<u8>().ok())
        .filter(|&p| p <= 100);
    let record = Record { state, kind, progress, app, title, msg };
    Some(Message::Report { id: id.unwrap_or_default(), record })
}

fn is_value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-')
}

/// `segment ("/" segment)*`, within the id limits.
fn parse_id(id: &[u8]) -> Option<String> {
    if id.len() > MAX_ID {
        return None;
    }
    let segments = id.split(|&b| b == b'/');
    let mut depth = 0;
    for segment in segments {
        depth += 1;
        let valid = !segment.is_empty() && segment.len() <= MAX_SEGMENT && segment.iter().all(|&b| is_name_byte(b));
        if !valid || depth > MAX_DEPTH {
            return None;
        }
    }
    String::from_utf8(id.to_vec()).ok()
}

fn parse_app(app: &[u8]) -> Option<String> {
    if app.is_empty() || app.len() > MAX_APP || !app.iter().all(|&b| is_name_byte(b)) {
        return None;
    }
    String::from_utf8(app.to_vec()).ok()
}

/// Decodes a base64 text value. `Some(None)` is an empty value; `None` is a
/// value that breaks a limit, doesn't decode, or holds a control character.
fn decode_text(encoded: &[u8], max_encoded: usize, max_decoded: usize) -> Option<Option<String>> {
    if encoded.len() > max_encoded {
        return None;
    }
    let bytes = BASE64.decode(encoded).ok()?;
    if bytes.len() > max_decoded {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.chars().any(|c| c.is_control()) {
        return None;
    }
    // Direction overrides and other invisible formatting would act on the
    // notification text around them; the record is shown only outside the
    // grid, so they are dropped here.
    let text: String = text.chars().filter(|&c| !is_bidi_control(c)).collect();
    Some(Some(text).filter(|t| !t.is_empty()))
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{061c}')
}

/// What a report changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Update {
    pub changed: bool,
    /// Set when a record became `blocked`, `done` or `error`, or changed
    /// while in one of those states: the user may want telling.
    pub notice: Option<Notice>,
}

#[derive(Debug, Clone)]
struct Entry {
    record: Record,
    /// Order of the last update, for eviction.
    updated: u64,
}

/// One terminal's records, by id (the root is `""`, so it sorts first).
#[derive(Debug, Default)]
pub struct Records {
    map: BTreeMap<String, Entry>,
    stamp: u64,
}

impl Records {
    pub fn get(&self, id: &str) -> Option<&Record> {
        self.map.get(id).map(|e| &e.record)
    }

    /// Every record, root first, then by id.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Record)> {
        self.map.iter().map(|(id, e)| (id.as_str(), &e.record))
    }

    /// Applies a report or a clear (queries are answered elsewhere).
    pub fn apply(&mut self, message: &Message) -> Update {
        match message {
            Message::Query => Update::default(),
            Message::Clear { id: None } => {
                let changed = !self.map.is_empty();
                self.map.clear();
                Update { changed, ..Default::default() }
            }
            Message::Clear { id: Some(id) } => {
                let prefix = format!("{id}/");
                let before = self.map.len();
                self.map.retain(|key, _| key != id && !key.starts_with(&prefix));
                Update { changed: self.map.len() != before, ..Default::default() }
            }
            Message::Report { id, record } => {
                let notice = self.notice_for(id, record);
                self.stamp += 1;
                if !self.map.contains_key(id) && self.map.len() >= MAX_RECORDS {
                    let oldest = self.map.iter().min_by_key(|(_, e)| e.updated).map(|(k, _)| k.clone());
                    if let Some(oldest) = oldest {
                        self.map.remove(&oldest);
                    }
                }
                self.map.insert(id.clone(), Entry { record: record.clone(), updated: self.stamp });
                Update { changed: true, notice }
            }
        }
    }

    /// A notice for a report, when its record is now `blocked`, `done` or
    /// `error` and wasn't just that, with that kind, title and message.
    fn notice_for(&self, id: &str, record: &Record) -> Option<Notice> {
        if !matches!(record.state, State::Blocked | State::Done | State::Error) {
            return None;
        }
        if let Some(old) = self.get(id)
            && (old.state, old.kind, &old.title, &old.msg) == (record.state, record.kind, &record.title, &record.msg)
        {
            return None;
        }
        // A child without `app` takes its nearest ancestor's; the root has
        // none to take, and its old record is about to be replaced.
        let inherited =
            if id.is_empty() { None } else { self.effective_app(id.rsplit_once('/').map_or("", |(parent, _)| parent)) };
        let app = record.app.as_deref().or(inherited);
        let title = record.title.as_deref().or(app).unwrap_or("").to_string();
        let body = record.msg.clone().unwrap_or_else(|| {
            match (record.state, record.kind) {
                (State::Blocked, Some(Kind::Permission)) => "Waiting for permission",
                (State::Blocked, Some(Kind::Question)) => "Waiting for an answer",
                (State::Blocked, Some(Kind::Auth)) => "Waiting for a login",
                (State::Blocked, None) => "Waiting for you",
                (State::Error, _) => "Failed",
                _ => "Done",
            }
            .to_string()
        });
        Some(Notice { source: Source::Osc7501, title, body })
    }

    /// The `app` of the record, or of its nearest ancestor that has one.
    pub fn effective_app(&self, id: &str) -> Option<&str> {
        let mut id = id;
        loop {
            if let Some(app) = self.get(id).and_then(|r| r.app.as_deref()) {
                return Some(app);
            }
            if id.is_empty() {
                return None;
            }
            id = id.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
    }

    /// A new shell prompt began, or the process exited: `working` and
    /// `blocked` records end. Returns whether anything changed.
    pub fn end_activity(&mut self) -> bool {
        self.retain(|r| !r.state.is_activity())
    }

    /// The user typed into the terminal: they have seen its `done` and
    /// `error` records.
    pub fn seen(&mut self) -> bool {
        self.retain(|r| !r.state.is_result())
    }

    /// A full reset (`ESC c`) removes every record.
    pub fn reset(&mut self) -> bool {
        let changed = !self.map.is_empty();
        self.map.clear();
        changed
    }

    fn retain(&mut self, keep: impl Fn(&Record) -> bool) -> bool {
        let before = self.map.len();
        self.map.retain(|_, e| keep(&e.record));
        self.map.len() != before
    }

    /// The most pressing state across the records; `idle` counts as none.
    pub fn summary(&self) -> Option<State> {
        self.map.values().map(|e| e.record.state).filter(|&s| s != State::Idle).max()
    }

    /// One `--- status …` line per record, for `--dump-screen-after`.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for (id, r) in self.iter() {
            out.push_str("--- status state=");
            out.push_str(r.state.name());
            if !id.is_empty() {
                out.push_str(&format!(" id={id:?}"));
            }
            if let Some(kind) = r.kind {
                out.push_str(&format!(" kind={}", kind.name()));
            }
            if let Some(progress) = r.progress {
                out.push_str(&format!(" progress={progress}"));
            }
            for (key, value) in [("app", &r.app), ("title", &r.title), ("msg", &r.msg)] {
                if let Some(value) = value {
                    out.push_str(&format!(" {key}={value:?}"));
                }
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(body: &str) -> (String, Record) {
        match parse(body.as_bytes()) {
            Some(Message::Report { id, record }) => (id, record),
            other => panic!("{body:?} parsed as {other:?}"),
        }
    }

    fn record(state: State) -> Record {
        Record { state, kind: None, progress: None, app: None, title: None, msg: None }
    }

    fn encode(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    #[test]
    fn the_specifications_example() {
        let msg = "QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/";
        let (id, r) = report(&format!("state=blocked:kind=permission:app=terraform:msg={msg}"));
        assert_eq!(id, "");
        assert_eq!(r.state, State::Blocked);
        assert_eq!(r.kind, Some(Kind::Permission));
        assert_eq!(r.app.as_deref(), Some("terraform"));
        assert_eq!(r.msg.as_deref(), Some("Apply 3 to add, 1 to change, 0 to destroy?"));
        assert_eq!(r.title, None);
        assert_eq!(r.progress, None);
    }

    #[test]
    fn query() {
        assert_eq!(parse(b"?"), Some(Message::Query));
        assert_eq!(parse(b" ? "), Some(Message::Query));
        // Not a query; the `?` is a malformed pair and is skipped.
        assert_eq!(report("?:state=idle").1.state, State::Idle);
    }

    #[test]
    fn pairs_are_lenient() {
        // Unknown key, pair without `=`, repeated key (last wins).
        let (_, r) = report("state=idle:colour=red:oops:state=working");
        assert_eq!(r, record(State::Working));
        // Whitespace around keys and values.
        let (_, r) = report(" state = done : app = brew ");
        assert_eq!(r.state, State::Done);
        assert_eq!(r.app.as_deref(), Some("brew"));
        // A pair with a byte outside the value set is skipped, not fatal.
        let (_, r) = report("state=done:msg=a b:app=x");
        assert_eq!(r.msg, None);
        assert_eq!(r.app.as_deref(), Some("x"));
        // Keys are lowercase letters.
        let (_, r) = report("State=idle:state=done:=x");
        assert_eq!(r.state, State::Done);
        // Empty value.
        let (_, r) = report("state=done:msg=:title=");
        assert_eq!((r.msg, r.title), (None, None));
    }

    #[test]
    fn states_and_clears() {
        for (name, state) in [
            ("idle", State::Idle),
            ("working", State::Working),
            ("done", State::Done),
            ("blocked", State::Blocked),
            ("error", State::Error),
        ] {
            assert_eq!(report(&format!("state={name}")).1.state, state);
        }
        assert_eq!(parse(b"state=paused:app=x"), None);
        assert_eq!(parse(b"app=x"), None);
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"state=clear"), Some(Message::Clear { id: None }));
        assert_eq!(parse(b"state=clear:id=build/test"), Some(Message::Clear { id: Some("build/test".into()) }));
        // A malformed id pair is skipped, as any malformed pair is.
        assert_eq!(parse(b"state=clear:id=bad id"), Some(Message::Clear { id: None }));
        assert_eq!(parse(b"state=clear:id=a/"), None, "an id that parses as a pair must be valid");
    }

    #[test]
    fn kind_and_progress_belong_to_their_states() {
        let (_, r) = report("state=blocked:kind=auth:progress=40");
        assert_eq!((r.kind, r.progress), (Some(Kind::Auth), Some(40)));
        let (_, r) = report("state=working:kind=auth:progress=101");
        assert_eq!((r.kind, r.progress), (None, None));
        let (_, r) = report("state=working:progress=100");
        assert_eq!(r.progress, Some(100));
        let (_, r) = report("state=done:progress=50");
        assert_eq!(r.progress, None);
        let (_, r) = report("state=blocked:kind=shrug:progress=x");
        assert_eq!((r.kind, r.progress), (None, None));
        for (kind, want) in [("permission", Kind::Permission), ("question", Kind::Question), ("auth", Kind::Auth)] {
            assert_eq!(report(&format!("state=blocked:kind={kind}")).1.kind, Some(want));
        }
    }

    #[test]
    fn ids() {
        assert_eq!(report("state=idle:id=us-east").0, "us-east");
        assert_eq!(report("state=idle:id=build/test.1/x_y+z").0, "build/test.1/x_y+z");
        for bad in ["", "/", "a/", "/a", "a//b", "a,b", &"x".repeat(33), &["a"; 9].join("/")] {
            assert_eq!(parse(format!("state=idle:id={bad}").as_bytes()), None, "id {bad:?}");
        }
        // A value outside the pair character set makes the pair malformed,
        // which skips it rather than the report.
        assert_eq!(report("state=idle:id=a b").0, "");
        assert_eq!(report("state=idle:id=a:b").0, "a");
        let deep = ["a"; 8].join("/");
        assert_eq!(report(&format!("state=idle:id={deep}")).0, deep);
        let long = format!("{}/{}/{}/{}", "x".repeat(32), "x".repeat(32), "x".repeat(32), "x".repeat(32));
        assert_eq!(long.len(), 131);
        assert_eq!(parse(format!("state=idle:id={long}").as_bytes()), None);
    }

    #[test]
    fn app_grammar() {
        assert_eq!(report("state=idle:app=claude-code").1.app.as_deref(), Some("claude-code"));
        let long = "a".repeat(32);
        assert_eq!(report(&format!("state=idle:app={long}")).1.app.as_deref(), Some(long.as_str()));
        assert_eq!(parse(format!("state=idle:app={}", "a".repeat(33)).as_bytes()), None);
        assert_eq!(parse(b"state=idle:app=a/b"), None);
        assert_eq!(parse(b"state=idle:app="), None);
    }

    #[test]
    fn text_values() {
        let (_, r) = report(&format!("state=done:msg={}:title={}", encode("Upgraded 12 packages"), encode("US East")));
        assert_eq!(r.msg.as_deref(), Some("Upgraded 12 packages"));
        assert_eq!(r.title.as_deref(), Some("US East"));
        // Padding is optional.
        assert_eq!(report("state=done:msg=SGk").1.msg.as_deref(), Some("Hi"));
        assert_eq!(report("state=done:msg=SGk=").1.msg.as_deref(), Some("Hi"));
        // Not base64, not UTF-8, a control character: the whole report goes.
        assert_eq!(parse(b"state=done:msg=SGk=x"), None);
        assert_eq!(parse(format!("state=done:msg={}", BASE64.encode([0xff, 0xfe])).as_bytes()), None);
        assert_eq!(parse(format!("state=done:msg={}", encode("a\x1bb")).as_bytes()), None);
        assert_eq!(parse(format!("state=done:msg={}", encode("a\u{85}b")).as_bytes()), None);
        assert_eq!(parse(format!("state=done:title={}", encode("\x7f")).as_bytes()), None);
        // Direction overrides are dropped.
        let overrides = encode("a\u{202e}b\u{2066}c");
        assert_eq!(report(&format!("state=done:msg={overrides}")).1.msg.as_deref(), Some("abc"));
        assert_eq!(report(&format!("state=done:msg={}", encode("\u{202e}"))).1.msg, None);
        // Non-ASCII text.
        assert_eq!(report(&format!("state=done:msg={}", encode("café ❤"))).1.msg.as_deref(), Some("café ❤"));
    }

    #[test]
    fn limits() {
        let msg = encode(&"m".repeat(2048));
        assert_eq!(msg.len(), 2732);
        assert_eq!(report(&format!("state=done:msg={msg}")).1.msg.as_deref().map(str::len), Some(2048));
        let msg = encode(&"m".repeat(2049));
        assert_eq!(parse(format!("state=done:msg={msg}").as_bytes()), None);
        // Encoded size is checked before decoding.
        assert_eq!(parse(format!("state=done:msg={}", "A".repeat(2736)).as_bytes()), None);
        let title = encode(&"t".repeat(192));
        assert_eq!(title.len(), 256);
        assert_eq!(report(&format!("state=done:title={title}")).1.title.as_deref().map(str::len), Some(192));
        assert_eq!(parse(format!("state=done:title={}", encode(&"t".repeat(193))).as_bytes()), None);
        // Key length.
        assert_eq!(report("state=done:abcdefghijklmnop=1").1.state, State::Done);
        assert_eq!(parse(b"state=done:abcdefghijklmnopq=1"), None);
        // Whole body.
        let body = format!("state=done:{}", "x".repeat(MAX_BODY - 11));
        assert_eq!(body.len(), MAX_BODY);
        assert_eq!(report(&body).1.state, State::Done);
        assert_eq!(parse(format!("{body}y").as_bytes()), None);
    }

    fn apply(records: &mut Records, body: &str) -> Update {
        records.apply(&parse(body.as_bytes()).expect("a valid report"))
    }

    fn states(records: &Records) -> Vec<(&str, State)> {
        records.iter().map(|(id, r)| (id, r.state)).collect()
    }

    #[test]
    fn records_are_replaced_whole() {
        let mut records = Records::default();
        apply(&mut records, "state=working:app=brew:msg=SGk=");
        apply(&mut records, "state=done");
        assert_eq!(records.get(""), Some(&record(State::Done)));
        assert_eq!(records.summary(), Some(State::Done));
    }

    #[test]
    fn clearing() {
        let mut records = Records::default();
        for id in ["", "build", "build/test", "buildx", "deploy"] {
            let id = if id.is_empty() { String::new() } else { format!(":id={id}") };
            apply(&mut records, &format!("state=working{id}"));
        }
        let update = apply(&mut records, "state=clear:id=build");
        assert!(update.changed && update.notice.is_none());
        assert_eq!(states(&records), [("", State::Working), ("buildx", State::Working), ("deploy", State::Working)]);
        assert!(!apply(&mut records, "state=clear:id=nothing").changed);
        assert!(apply(&mut records, "state=clear").changed);
        assert_eq!(states(&records), []);
        assert!(!apply(&mut records, "state=clear").changed);
    }

    #[test]
    fn lifetimes() {
        let mut records = Records::default();
        for (id, state) in [("i", "idle"), ("w", "working"), ("d", "done"), ("b", "blocked"), ("e", "error")] {
            apply(&mut records, &format!("state={state}:id={id}"));
        }
        assert_eq!(records.summary(), Some(State::Blocked));
        assert!(records.end_activity());
        assert_eq!(states(&records), [("d", State::Done), ("e", State::Error), ("i", State::Idle)]);
        assert_eq!(records.summary(), Some(State::Error));
        assert!(!records.end_activity());
        assert!(records.seen());
        assert_eq!(states(&records), [("i", State::Idle)]);
        assert_eq!(records.summary(), None);
        assert!(!records.seen());
        assert!(records.reset());
        assert_eq!(states(&records), []);
        assert!(!records.reset());
    }

    #[test]
    fn eviction_past_the_cap() {
        let mut records = Records::default();
        for i in 0..MAX_RECORDS {
            apply(&mut records, &format!("state=idle:id=r{i}"));
        }
        // Touch the oldest so it isn't the one to go.
        apply(&mut records, "state=idle:id=r0");
        apply(&mut records, "state=idle:id=new");
        assert_eq!(records.map.len(), MAX_RECORDS);
        assert!(records.get("r0").is_some());
        assert!(records.get("r1").is_none());
        assert!(records.get("new").is_some());
    }

    #[test]
    fn app_is_inherited() {
        let mut records = Records::default();
        apply(&mut records, "state=working:app=deploy");
        apply(&mut records, "state=working:id=eu/west");
        apply(&mut records, "state=working:id=us:app=other");
        apply(&mut records, "state=working:id=us/east");
        assert_eq!(records.effective_app("eu/west"), Some("deploy"));
        assert_eq!(records.effective_app("eu"), Some("deploy"), "a missing parent is skipped");
        assert_eq!(records.effective_app("us/east"), Some("other"));
        assert_eq!(Records::default().effective_app("x/y"), None);
    }

    #[test]
    fn notices() {
        let mut records = Records::default();
        let msg = encode("Run cargo test?");
        assert_eq!(apply(&mut records, "state=working:app=claude-code").notice, None);
        let update = apply(&mut records, &format!("state=blocked:kind=permission:app=claude-code:msg={msg}"));
        assert_eq!(
            update.notice,
            Some(Notice { source: Source::Osc7501, title: "claude-code".into(), body: "Run cargo test?".into() })
        );
        // The same report again is quiet; a different message isn't.
        let again = apply(&mut records, &format!("state=blocked:kind=permission:app=claude-code:msg={msg}"));
        assert_eq!(again.notice, None);
        let text = |update: Update| update.notice.map(|n| (n.title, n.body));
        let update = apply(&mut records, "state=blocked:kind=permission:app=claude-code");
        assert_eq!(text(update), Some(("claude-code".into(), "Waiting for permission".into())));
        let update = apply(&mut records, "state=blocked:kind=question");
        assert_eq!(text(update), Some((String::new(), "Waiting for an answer".into())));
        let body = |update: Update| update.notice.map(|n| n.body);
        assert_eq!(body(apply(&mut records, "state=blocked:kind=auth")), Some("Waiting for a login".into()));
        assert_eq!(body(apply(&mut records, "state=blocked")), Some("Waiting for you".into()));
        assert_eq!(body(apply(&mut records, "state=error")), Some("Failed".into()));
        assert_eq!(text(apply(&mut records, "state=done:app=brew")), Some(("brew".into(), "Done".into())));
        assert_eq!(apply(&mut records, "state=idle").notice, None);
        assert_eq!(apply(&mut records, "state=clear").notice, None);
        // The title key wins over the app, and a child inherits the app.
        apply(&mut records, "state=working:app=deploy");
        let title = encode("EU West");
        let update = apply(&mut records, &format!("state=done:id=eu-west:title={title}"));
        assert_eq!(update.notice.map(|n| n.title), Some("EU West".into()));
        let update = apply(&mut records, "state=error:id=us-east");
        assert_eq!(update.notice.map(|n| n.title), Some("deploy".into()));
    }

    #[test]
    fn describe_lines() {
        let mut records = Records::default();
        apply(&mut records, "state=done:app=brew:msg=SGk=");
        let title = encode("EU \"West\"");
        apply(&mut records, &format!("state=blocked:id=eu-west:kind=permission:progress=40:title={title}"));
        assert_eq!(
            records.describe(),
            "--- status state=done app=\"brew\" msg=\"Hi\"\n\
             --- status state=blocked id=\"eu-west\" kind=permission progress=40 title=\"EU \\\"West\\\"\"\n"
        );
        assert_eq!(Records::default().describe(), "");
    }
}
