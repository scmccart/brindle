//! kitty's desktop notification protocol, `OSC 99 ; <metadata> ; <payload>`:
//! one sequence parsed into a [`Chunk`]. A notification may take several
//! chunks, which the client puts together (`src/notify/kitty.rs`); the
//! server core drops them, as it drops every notice.
//!
//! The metadata is `:`-separated `key=value` pairs with one-letter keys.
//! Unknown keys, and values a known key doesn't take, are ignored; a part
//! that isn't `key=value`, a bad id, an over-long part or a base64 payload
//! with a byte outside the alphabet makes the whole sequence ignored, so a
//! program's bytes never panic or reach a reply unchecked.
//!
//! A base64 payload is checked here but decoded by the assembly, since a
//! program may cut it anywhere between chunks.

use std::time::Duration;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Longest payload (after the second `;`), as kitty's protocol allows.
pub const MAX_PAYLOAD: usize = 4096;
/// Longest metadata (between `99;` and the second `;`).
pub const MAX_METADATA: usize = 1024;
/// The most an OSC 99 body can hold: metadata, its `;` and the payload.
pub const MAX_BODY: usize = MAX_METADATA + 1 + MAX_PAYLOAD;
/// Longest notification id.
pub const MAX_ID: usize = 64;

/// Standard base64, padding optional (for the `s` value).
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// A notification id: 1 to [`MAX_ID`] characters of `[A-Za-z0-9_+.-]`, so
/// it can be written back in a reply as is.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KittyId(String);

impl KittyId {
    /// The id `bytes` spell, if they are one.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let allowed = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'.' | b'-');
        if bytes.is_empty() || bytes.len() > MAX_ID || !bytes.iter().all(allowed) {
            return None;
        }
        // Checked ASCII above, so this never fails.
        String::from_utf8(bytes.to_vec()).ok().map(KittyId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a chunk's payload is (`p`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    Title,
    Body,
    /// Button labels, separated by U+2028.
    Buttons,
    /// Close the notification with this id.
    Close,
    /// `p=?`: what the terminal supports.
    Query,
    /// Which of this terminal's notifications are still open.
    Alive,
    /// `icon`, or a type no one knows: ignored whole.
    Unsupported,
}

impl Payload {
    fn from_value(value: &[u8]) -> Self {
        match value {
            b"title" => Payload::Title,
            b"body" => Payload::Body,
            b"buttons" => Payload::Buttons,
            b"close" => Payload::Close,
            b"?" => Payload::Query,
            b"alive" => Payload::Alive,
            _ => Payload::Unsupported,
        }
    }

    /// Whether the payload is text the notification shows.
    pub fn is_text(self) -> bool {
        match self {
            Payload::Title | Payload::Body | Payload::Buttons => true,
            Payload::Close | Payload::Query | Payload::Alive | Payload::Unsupported => false,
        }
    }
}

/// A chunk's payload bytes, as sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Data {
    /// Plain text (C0 controls already dropped by the scanner), not yet
    /// read as UTF-8: a character may continue in the next chunk.
    Plain(Vec<u8>),
    /// `e=1`: base64, every byte from its alphabet (`A-Za-z0-9+/=`).
    Base64(Vec<u8>),
}

impl Data {
    pub fn is_empty(&self) -> bool {
        match self {
            Data::Plain(bytes) | Data::Base64(bytes) => bytes.is_empty(),
        }
    }
}

/// The sound a notification asks for (`s`, base64 of its name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// The platform's own sound: `system`, and every name but `silent`.
    Default,
    Silent,
}

/// When a notification may be shown (`o`), on top of the user's `when`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occasion {
    Always,
    /// Only when the tab isn't in front of the user.
    Unfocused,
    /// Only when the tab isn't its window's active tab.
    Invisible,
}

/// `u`: 0, 1 or 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// `w`, in milliseconds: `-1` (or less) the server's default, `0` never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    Default,
    Never,
    After(Duration),
}

/// The metadata a chunk names; `None` where it says nothing, so a later
/// chunk of the same notification replaces only what it names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// `a`'s `focus`: on unless removed.
    pub focus: Option<bool>,
    /// `a`'s `report`: a click report written back.
    pub report: Option<bool>,
    /// `c`: a report when the notification closes.
    pub close_report: Option<bool>,
    pub occasion: Option<Occasion>,
    pub urgency: Option<Urgency>,
    pub expiry: Option<Expiry>,
    pub sound: Option<Sound>,
}

impl Meta {
    /// This metadata with `later`'s values over it.
    pub fn then(self, later: Meta) -> Meta {
        Meta {
            focus: later.focus.or(self.focus),
            report: later.report.or(self.report),
            close_report: later.close_report.or(self.close_report),
            occasion: later.occasion.or(self.occasion),
            urgency: later.urgency.or(self.urgency),
            expiry: later.expiry.or(self.expiry),
            sound: later.sound.or(self.sound),
        }
    }
}

/// One OSC 99 sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub id: Option<KittyId>,
    pub payload: Payload,
    /// `d`: false (`d=0`) holds the notification for more chunks.
    pub done: bool,
    /// The payload of a text chunk; empty for the others.
    pub data: Data,
    pub meta: Meta,
}

/// Parses the body of an OSC 99 (after `99;`). `None` is a sequence the
/// protocol says to ignore.
pub fn parse(body: &[u8]) -> Option<Chunk> {
    let split = body.iter().position(|&b| b == b';')?;
    let (metadata, payload) = (&body[..split], &body[split + 1..]);
    if metadata.len() > MAX_METADATA || payload.len() > MAX_PAYLOAD {
        return None;
    }
    let mut id = None;
    let mut kind = Payload::Title;
    let mut done = true;
    let mut encoded = false;
    let mut meta = Meta::default();
    for part in metadata.split(|&b| b == b':').filter(|part| !part.is_empty()) {
        let eq = part.iter().position(|&b| b == b'=')?;
        let (key, value) = (&part[..eq], &part[eq + 1..]);
        match key {
            b"i" => id = Some(KittyId::parse(value)?),
            b"p" => kind = Payload::from_value(value),
            b"d" => done = value != b"0",
            b"e" => encoded = value == b"1",
            b"a" => actions(value, &mut meta),
            b"c" => meta.close_report = Some(value != b"0"),
            b"o" => meta.occasion = occasion(value).or(meta.occasion),
            b"u" => meta.urgency = urgency(value).or(meta.urgency),
            b"w" => meta.expiry = expiry(value).or(meta.expiry),
            b"s" => meta.sound = sound(value).or(meta.sound),
            // `f`, `g`, `n`, `t` and keys no one knows.
            _ => {}
        }
    }
    let data = if !kind.is_text() {
        Data::Plain(Vec::new())
    } else if encoded {
        if !payload.iter().all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')) {
            return None;
        }
        Data::Base64(payload.to_vec())
    } else {
        Data::Plain(payload.to_vec())
    };
    Some(Chunk { id, payload: kind, done, data, meta })
}

/// `a`: `,`-separated actions, a leading `-` removing one. Unknown ones are
/// skipped.
fn actions(value: &[u8], meta: &mut Meta) {
    for action in value.split(|&b| b == b',') {
        let (on, name) = match action.strip_prefix(b"-") {
            Some(name) => (false, name),
            None => (true, action),
        };
        match name {
            b"focus" => meta.focus = Some(on),
            b"report" => meta.report = Some(on),
            _ => {}
        }
    }
}

fn occasion(value: &[u8]) -> Option<Occasion> {
    match value {
        b"always" => Some(Occasion::Always),
        b"unfocused" => Some(Occasion::Unfocused),
        b"invisible" => Some(Occasion::Invisible),
        _ => None,
    }
}

fn urgency(value: &[u8]) -> Option<Urgency> {
    match value {
        b"0" => Some(Urgency::Low),
        b"1" => Some(Urgency::Normal),
        b"2" => Some(Urgency::Critical),
        _ => None,
    }
}

fn expiry(value: &[u8]) -> Option<Expiry> {
    // At most a 19-digit number, so the parse can't overflow quietly.
    let text = std::str::from_utf8(value).ok().filter(|t| t.len() <= 19)?;
    let ms: i64 = text.parse().ok()?;
    Some(match ms {
        i64::MIN..=-1 => Expiry::Default,
        0 => Expiry::Never,
        ms => Expiry::After(Duration::from_millis(ms.unsigned_abs())),
    })
}

/// `s`: base64 of a sound name. One that doesn't decode is ignored.
fn sound(value: &[u8]) -> Option<Sound> {
    let name = BASE64.decode(value).ok()?;
    Some(if name == b"silent" { Sound::Silent } else { Sound::Default })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(body: &str) -> Chunk {
        parse(body.as_bytes()).unwrap_or_else(|| panic!("{body:?} should parse"))
    }

    fn id(text: &str) -> KittyId {
        KittyId::parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn the_simplest_form_is_a_finished_title() {
        assert_eq!(
            chunk(";Hello world"),
            Chunk {
                id: None,
                payload: Payload::Title,
                done: true,
                data: Data::Plain(b"Hello world".to_vec()),
                meta: Meta::default(),
            }
        );
        // The payload keeps its `;`s.
        assert_eq!(chunk(";a;b").data, Data::Plain(b"a;b".to_vec()));
        assert_eq!(chunk(";").data, Data::Plain(Vec::new()));
    }

    #[test]
    fn without_the_second_semicolon_it_is_ignored() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"i=1"), None);
        assert_eq!(parse(b"Hello"), None);
    }

    #[test]
    fn metadata_keys() {
        let c = chunk("i=b:d=0:p=body:a=-focus,report:c=1:o=invisible:u=2:w=1500:s=c2lsZW50;x");
        assert_eq!(c.id, Some(id("b")));
        assert_eq!(c.payload, Payload::Body);
        assert!(!c.done);
        assert_eq!(
            c.meta,
            Meta {
                focus: Some(false),
                report: Some(true),
                close_report: Some(true),
                occasion: Some(Occasion::Invisible),
                urgency: Some(Urgency::Critical),
                expiry: Some(Expiry::After(Duration::from_millis(1500))),
                sound: Some(Sound::Silent),
            }
        );
        // `d` is done unless `0`; `c` is on unless `0`.
        assert!(chunk("d=1;x").done && chunk("d=2;x").done && chunk("d=;x").done);
        assert_eq!(chunk("c=0;x").meta.close_report, Some(false));
        // Empty parts are skipped.
        assert_eq!(chunk("::i=a::;x").id, Some(id("a")));
    }

    #[test]
    fn unknown_keys_and_values_are_ignored() {
        let c = chunk("f=a2l0dHk=:g=x:n=ZXJyb3I=:t=aW0=:z=9:long=1:o=sometimes:u=7:w=soon:a=wiggle,+focus;x");
        assert_eq!(c.meta, Meta::default());
        assert_eq!(c.payload, Payload::Title);
        // A later value of the same key wins; an unknown one keeps the last.
        assert_eq!(chunk("u=0:u=2;x").meta.urgency, Some(Urgency::Critical));
        assert_eq!(chunk("u=0:u=9;x").meta.urgency, Some(Urgency::Low));
    }

    #[test]
    fn a_part_without_a_value_ignores_the_sequence() {
        assert_eq!(parse(b"i=a:d;x"), None);
        assert_eq!(parse(b"oops;x"), None);
    }

    #[test]
    fn ids() {
        assert_eq!(chunk("i=Az09_+.-;x").id, Some(id("Az09_+.-")));
        let longest = "a".repeat(MAX_ID);
        assert_eq!(chunk(&format!("i={longest};x")).id.map(|i| i.as_str().len()), Some(MAX_ID));
        for bad in ["i=a b;x", "i=;x", "i=é;x", "i=a/b;x", "i=a\\x3b;x"] {
            assert_eq!(parse(bad.as_bytes()), None, "{bad:?}");
        }
        assert_eq!(parse(format!("i={};x", "a".repeat(MAX_ID + 1)).as_bytes()), None);
    }

    #[test]
    fn base64_payloads_are_checked_not_decoded() {
        assert_eq!(chunk("e=1;SGk=").data, Data::Base64(b"SGk=".to_vec()));
        // Cut anywhere: decoded by the assembly.
        assert_eq!(chunk("e=1;SGV").data, Data::Base64(b"SGV".to_vec()));
        assert_eq!(chunk("e=0;SGk=").data, Data::Plain(b"SGk=".to_vec()));
        for bad in ["e=1;SG k=", "e=1;SGk!", "e=1;é"] {
            assert_eq!(parse(bad.as_bytes()), None, "{bad:?}");
        }
        // Only text payloads carry data: an icon's isn't checked.
        assert_eq!(chunk("e=1:p=icon;!!").data, Data::Plain(Vec::new()));
    }

    #[test]
    fn payload_types() {
        let kinds = [
            ("", Payload::Title),
            ("title", Payload::Title),
            ("body", Payload::Body),
            ("buttons", Payload::Buttons),
            ("close", Payload::Close),
            ("?", Payload::Query),
            ("alive", Payload::Alive),
            ("icon", Payload::Unsupported),
            ("Title", Payload::Unsupported),
        ];
        for (value, kind) in kinds {
            let body = if value.is_empty() { "i=q;text".to_string() } else { format!("i=q:p={value};text") };
            let c = chunk(&body);
            assert_eq!(c.payload, kind, "{value:?}");
            assert_eq!(c.data.is_empty(), !kind.is_text(), "{value:?}");
        }
    }

    #[test]
    fn expiry_values() {
        assert_eq!(chunk("w=-1;x").meta.expiry, Some(Expiry::Default));
        assert_eq!(chunk("w=-50;x").meta.expiry, Some(Expiry::Default));
        assert_eq!(chunk("w=0;x").meta.expiry, Some(Expiry::Never));
        assert_eq!(chunk("w=10;x").meta.expiry, Some(Expiry::After(Duration::from_millis(10))));
        for bad in ["w=;x", "w=1.5;x", "w=99999999999999999999;x", "w=1e3;x"] {
            assert_eq!(chunk(bad).meta.expiry, None, "{bad:?}");
        }
    }

    #[test]
    fn sounds_are_base64_names() {
        assert_eq!(chunk("s=c2lsZW50;x").meta.sound, Some(Sound::Silent));
        assert_eq!(chunk("s=c2lsZW50:s=c3lzdGVt;x").meta.sound, Some(Sound::Default), "`system` after it");
        assert_eq!(chunk("s=d2FybmluZw==;x").meta.sound, Some(Sound::Default), "`warning`");
        // Not base64: ignored.
        assert_eq!(chunk("s=silent;x").meta.sound, None);
    }

    #[test]
    fn over_long_parts_are_ignored() {
        let payload = "x".repeat(MAX_PAYLOAD);
        assert!(parse(format!(";{payload}").as_bytes()).is_some());
        assert_eq!(parse(format!(";{payload}x").as_bytes()), None);
        let metadata = format!("z={}", "m".repeat(MAX_METADATA - 2));
        assert!(parse(format!("{metadata};x").as_bytes()).is_some());
        assert_eq!(parse(format!("{metadata}m;x").as_bytes()), None);
    }

    #[test]
    fn later_metadata_wins_and_the_rest_is_kept() {
        let first = chunk("a=report:u=2:c=1;x").meta;
        let later = chunk("a=-report:o=unfocused;y").meta;
        let merged = first.then(later);
        assert_eq!(merged.report, Some(false));
        assert_eq!(merged.urgency, Some(Urgency::Critical));
        assert_eq!(merged.close_report, Some(true));
        assert_eq!(merged.occasion, Some(Occasion::Unfocused));
        assert_eq!(merged.focus, None);
    }
}
