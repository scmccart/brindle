//! Finds the sequences Brindle acts on beside alacritty's parser, which
//! drops OSCs it doesn't know: OSC 9 and OSC 777 notifications, OSC 7501
//! program status reports, and the OSC 133 `A` prompt starts and `ESC c`
//! resets that end status records.
//!
//! The scanner keeps its state between calls, so a sequence split across
//! reads is still found, and it never buffers anything but the payload of
//! an OSC it wants.

use crate::terminal::status::{self, Message};

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

/// What the scanner found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Notice(Notice),
    /// An OSC 7501 report, clear or query.
    Status(Message),
    /// OSC 133 `A`: a new shell prompt begins.
    PromptStart,
    /// `ESC c`: a full reset.
    Reset,
}

/// Payloads longer than this are ignored.
pub const MAX_PAYLOAD: usize = 4096;
/// Longest title and body shown, in characters.
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_BODY_CHARS: usize = 1000;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    /// After ESC.
    Esc,
    /// After `ESC ]`, reading the OSC number.
    Number,
    /// In the payload of a wanted OSC (`collect`), or of any other OSC.
    Payload { collect: bool },
    /// ESC inside an OSC: `\` completes it, anything else cancels it.
    PayloadEsc { collect: bool },
}

#[derive(Debug)]
pub struct Scanner {
    state: State,
    number: u32,
    /// Digits seen in `number`, which must match the wanted number's.
    digits: u8,
    payload: Vec<u8>,
    overflow: bool,
}

impl Default for Scanner {
    fn default() -> Self {
        Self { state: State::Ground, number: 0, digits: 0, payload: Vec::new(), overflow: false }
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans the next chunk of output and returns what completed in it.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<Found> {
        let mut found = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                // Fast path: jump to the next ESC.
                match bytes[i..].iter().position(|&b| b == ESC) {
                    Some(offset) => {
                        i += offset + 1;
                        self.state = State::Esc;
                    }
                    None => break,
                }
                continue;
            }
            let byte = bytes[i];
            i += 1;
            if let Some(item) = self.step(byte) {
                found.push(item);
            }
        }
        found
    }

    fn step(&mut self, byte: u8) -> Option<Found> {
        match self.state {
            State::Ground => {
                if byte == ESC {
                    self.state = State::Esc;
                }
            }
            State::Esc => {
                self.state = match byte {
                    b']' => {
                        self.number = 0;
                        self.digits = 0;
                        State::Number
                    }
                    ESC => State::Esc,
                    b'c' => {
                        self.state = State::Ground;
                        return Some(Found::Reset);
                    }
                    _ => State::Ground,
                };
            }
            State::Number => match byte {
                b'0'..=b'9' => {
                    self.digits = self.digits.saturating_add(1);
                    self.number = self.number * 10 % 100_000 + u32::from(byte - b'0');
                }
                b';' => {
                    // Written as the number is: `0009` isn't 9.
                    let wanted = matches!((self.number, self.digits), (9, 1) | (777, 3) | (133, 3) | (7501, 4));
                    self.payload.clear();
                    self.overflow = false;
                    self.state = State::Payload { collect: wanted };
                }
                _ => return self.payload_byte(byte, false),
            },
            State::Payload { collect } => return self.payload_byte(byte, collect),
            State::PayloadEsc { collect } => {
                if byte == b'\\' {
                    self.state = State::Ground;
                    return self.finish(collect);
                }
                // Not ST: the OSC is cancelled and this ESC starts a new
                // sequence.
                self.state = State::Esc;
                return self.step(byte);
            }
        }
        None
    }

    fn payload_byte(&mut self, byte: u8, collect: bool) -> Option<Found> {
        match byte {
            BEL => {
                self.state = State::Ground;
                return self.finish(collect);
            }
            ESC => self.state = State::PayloadEsc { collect },
            CAN | SUB => self.state = State::Ground,
            // Other C0 controls are ignored inside an OSC, as alacritty does.
            0x00..=0x1f => self.state = State::Payload { collect },
            _ => {
                self.state = State::Payload { collect };
                if collect {
                    if self.payload.len() < MAX_PAYLOAD {
                        self.payload.push(byte);
                    } else {
                        self.overflow = true;
                    }
                }
            }
        }
        None
    }

    fn finish(&mut self, collect: bool) -> Option<Found> {
        if !collect || self.overflow {
            return None;
        }
        let payload = std::mem::take(&mut self.payload);
        match self.number {
            9 => parse_osc9(&payload).map(Found::Notice),
            777 => parse_osc777(&payload).map(Found::Notice),
            // `133;A` may carry options after a `;`.
            133 => (payload == b"A" || payload.starts_with(b"A;")).then_some(Found::PromptStart),
            7501 => status::parse(&payload).map(Found::Status),
            _ => None,
        }
    }
}

/// `OSC 9 ; <message>`. ConEmu uses `OSC 9 ; <1-12> [; ...]` for commands
/// (progress, working directory, ...), which are not messages.
pub fn parse_osc9(payload: &[u8]) -> Option<Notice> {
    let text = String::from_utf8_lossy(payload);
    let first = text.split(';').next().unwrap_or("");
    if !first.is_empty()
        && first.bytes().all(|b| b.is_ascii_digit())
        && first.parse::<u32>().is_ok_and(|n| (1..=12).contains(&n))
    {
        return None;
    }
    let body = clean(&text, MAX_BODY_CHARS);
    (!body.is_empty()).then(|| Notice { source: Source::Osc9, title: String::new(), body })
}

/// `OSC 777 ; notify ; <title> ; <body>`; the body may contain `;`.
pub fn parse_osc777(payload: &[u8]) -> Option<Notice> {
    let text = String::from_utf8_lossy(payload);
    let mut parts = text.splitn(3, ';');
    if parts.next() != Some("notify") {
        return None;
    }
    let title = clean(parts.next().unwrap_or(""), MAX_TITLE_CHARS);
    let body = clean(parts.next().unwrap_or(""), MAX_BODY_CHARS);
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(Notice { source: Source::Osc777, title, body })
}

/// Removes control characters, trims, and shortens to `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    truncate(text.trim(), max)
}

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(bytes: &[u8]) -> Vec<Found> {
        Scanner::new().scan(bytes)
    }

    fn osc9(body: &str) -> Found {
        Found::Notice(Notice { source: Source::Osc9, title: String::new(), body: body.into() })
    }

    fn osc777(title: &str, body: &str) -> Found {
        Found::Notice(Notice { source: Source::Osc777, title: title.into(), body: body.into() })
    }

    fn body(found: &Found) -> &str {
        match found {
            Found::Notice(notice) => &notice.body,
            other => panic!("not a notice: {other:?}"),
        }
    }

    #[test]
    fn bel_and_st_terminate() {
        assert_eq!(scan(b"\x1b]9;hello\x07"), [osc9("hello")]);
        assert_eq!(scan(b"\x1b]9;hello\x1b\\"), [osc9("hello")]);
        assert_eq!(scan(b"a\x1b]777;notify;T;B\x07b"), [osc777("T", "B")]);
    }

    #[test]
    fn several_in_one_chunk_among_other_output() {
        let out = b"\x1b[31mred\x1b[0m\x1b]0;title\x07\x1b]9;one\x07text\x1b]777;notify;Two;2\x1b\\";
        assert_eq!(scan(out), [osc9("one"), osc777("Two", "2")]);
    }

    #[test]
    fn split_at_every_byte() {
        let input = b"x\x1b]777;notify;Tests;12 passed; 0 failed\x1b\\y\x1b]9;done\x07";
        for split in 0..=input.len() {
            let mut scanner = Scanner::new();
            let mut found = scanner.scan(&input[..split]);
            found.extend(scanner.scan(&input[split..]));
            assert_eq!(found, [osc777("Tests", "12 passed; 0 failed"), osc9("done")], "split at {split}");
        }
        // And one byte at a time.
        let mut scanner = Scanner::new();
        let found: Vec<Found> = input.iter().flat_map(|b| scanner.scan(std::slice::from_ref(b))).collect();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn cancelled_sequences_do_not_notify() {
        assert!(scan(b"\x1b]9;hi\x18").is_empty());
        assert!(scan(b"\x1b]9;hi\x1a").is_empty());
        // ESC that isn't ST cancels, and starts the next sequence.
        assert_eq!(scan(b"\x1b]9;lost\x1b]9;kept\x07"), [osc9("kept")]);
        assert!(scan(b"\x1b]9;hi\x1b[0m").is_empty());
        // Unterminated: nothing yet.
        assert!(scan(b"\x1b]9;pending").is_empty());
    }

    #[test]
    fn other_oscs_are_ignored() {
        assert!(scan(b"\x1b]0;9;x\x07\x1b]99;x\x07\x1b]7770;notify;a;b\x07\x1b]52;c;aGk=\x07").is_empty());
        assert!(scan(b"\x1b]0009;x\x07\x1b]07501;state=idle\x07\x1b]75010;state=idle\x07").is_empty());
        // A number with no payload.
        assert!(scan(b"\x1b]9\x07").is_empty());
    }

    #[test]
    fn oversize_payload_is_dropped() {
        let mut big = b"\x1b]9;".to_vec();
        big.extend(std::iter::repeat_n(b'a', MAX_PAYLOAD + 1));
        big.push(BEL);
        big.extend_from_slice(b"\x1b]9;after\x07");
        assert_eq!(scan(&big), [osc9("after")]);
    }

    #[test]
    fn conemu_subcommands_are_not_messages() {
        for seq in ["4;1;50", "4;0", "9;/home/me", "1;100", "12", "2;box", "3;tab"] {
            assert!(scan(format!("\x1b]9;{seq}\x07").as_bytes()).is_empty(), "{seq}");
        }
        assert_eq!(scan(b"\x1b]9;42\x07"), [osc9("42")]);
        assert_eq!(scan(b"\x1b]9;0\x07"), [osc9("0")]);
        assert_eq!(scan(b"\x1b]9;3 files copied\x07"), [osc9("3 files copied")]);
        assert_eq!(scan(b"\x1b]9;a;b\x07"), [osc9("a;b")]);
        assert!(scan(b"\x1b]9;\x07").is_empty());
    }

    #[test]
    fn osc777_forms() {
        assert_eq!(scan(b"\x1b]777;notify;;only body\x07"), [osc777("", "only body")]);
        assert_eq!(scan(b"\x1b]777;notify;only title\x07"), [osc777("only title", "")]);
        assert!(scan(b"\x1b]777;notify;;\x07").is_empty());
        assert!(scan(b"\x1b]777;precmd\x07\x1b]777;preexec\x07\x1b]777;container;push;x\x07").is_empty());
    }

    #[test]
    fn text_is_cleaned() {
        // C0 bytes inside the payload are dropped by the scanner; other
        // control characters (C1, DEL) by the clean-up.
        assert_eq!(scan(b"\x1b]9; a\x01b\x7f\xc2\x85c \x07"), [osc9("abc")]);
        assert_eq!(scan(b"\x1b]9;caf\xc3\xa9 \xff\x07"), [osc9("café \u{fffd}")]);
        let long = "x".repeat(MAX_BODY_CHARS + 10);
        let notice = parse_osc9(long.as_bytes()).unwrap();
        assert_eq!(notice.body.chars().count(), MAX_BODY_CHARS);
        assert!(notice.body.ends_with('…'));
    }

    #[test]
    fn program_status() {
        use crate::terminal::status::{Record, State};
        let report = Found::Status(Message::Report {
            id: String::new(),
            record: Record {
                state: State::Working,
                kind: None,
                progress: None,
                app: Some("x".into()),
                title: None,
                msg: None,
            },
        });
        assert_eq!(scan(b"\x1b]7501;state=working:app=x\x1b\\"), std::slice::from_ref(&report));
        assert_eq!(scan(b"\x1b]7501;state=working:app=x\x07"), std::slice::from_ref(&report));
        assert_eq!(scan(b"\x1b]7501;?\x1b\\"), [Found::Status(Message::Query)]);
        assert_eq!(scan(b"\x1b]7501;state=clear\x07"), [Found::Status(Message::Clear { id: None })]);
        // Ignored reports yield nothing, and don't disturb what follows.
        assert_eq!(scan(b"\x1b]7501;state=odd\x07\x1b]9;hi\x07"), [osc9("hi")]);
        // Split anywhere.
        let input = b"\x1b]7501;state=working:app=x\x1b\\";
        for split in 0..=input.len() {
            let mut scanner = Scanner::new();
            let mut found = scanner.scan(&input[..split]);
            found.extend(scanner.scan(&input[split..]));
            assert_eq!(found, std::slice::from_ref(&report), "split at {split}");
        }
    }

    #[test]
    fn prompt_starts_and_resets() {
        let two_prompts = b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;A;k=i\x1b\\";
        assert_eq!(scan(two_prompts), [Found::PromptStart, Found::PromptStart]);
        assert!(scan(b"\x1b]133;D;0\x07\x1b]133;AB\x07").is_empty());
        assert_eq!(scan(b"a\x1bcb\x1b[0m\x1b]9;x\x07"), [Found::Reset, osc9("x")]);
        // `ESC c` inside an OSC cancels it, as any other ESC does.
        assert_eq!(scan(b"\x1b]9;lost\x1bc"), [Found::Reset]);
        assert!(scan(b"\x1b[c").is_empty(), "DA, not RIS");
        assert_eq!(body(&scan(b"\x1b]9;hi\x07")[0]), "hi");
    }
}
