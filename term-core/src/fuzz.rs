//! Seeded fuzz tests for the parsers of a program's output: the OSC
//! scanner, the status parser and the whole [`Emulator`]. Each feeds random
//! streams, biased toward the sequences the parsers act on, and asserts a
//! property of what comes out, not only that nothing panics. Every
//! generator has a fixed seed, so a failure reproduces.
//!
//! The private parsers in `shell_integration.rs` are fuzzed from its own
//! `mod tests`, which uses the builders here.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::emulator::Emulator;
use crate::osc::{self, Found};
use crate::shell_integration::PathStyle;
use crate::status::{self, Message};

/// A fixed-seed xorshift generator.
///
/// It is a copy of the one in `remote-protocol/src/fuzz.rs`: the shared
/// crates never reference each other, and a crate of their own for 25 lines
/// would join the version alignment (`ports R3`).
pub(crate) struct XorShift(u64);

impl XorShift {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

const NAME_BYTES: &[u8] = b"abcdefghijklmnopqrstuvwxyzABC019_.+-";
const PATH_BYTES: &[u8] = b"abcxyz019_.-/ %:\\\"~";

fn pick<'a>(rng: &mut XorShift, items: &[&'a str]) -> &'a str {
    items[rng.below(items.len())]
}

fn name(rng: &mut XorShift, max: usize) -> String {
    let len = 1 + rng.below(max);
    (0..len).map(|_| NAME_BYTES[rng.below(NAME_BYTES.len())] as char).collect()
}

/// A path-ish string, with `%XX` escapes (valid and not) and separators.
fn path(rng: &mut XorShift) -> String {
    let mut out = String::new();
    for _ in 0..rng.below(6) {
        match rng.below(6) {
            0 => out.push_str(&format!("%{:02X}", rng.next() as u8)),
            1 => out.push_str(pick(rng, &["%20", "%2F", "%E2%82%AC", "%", "%G1", "%C3"])),
            2 => out.push_str(pick(rng, &["/", "/", "\\", "C:", "C:/", "//", "\\\\srv\\share"])),
            3 => out.push(char::from_u32(0x80 + rng.below(0x500) as u32).unwrap_or('?')),
            _ => {
                let len = 1 + rng.below(6);
                out.extend((0..len).map(|_| PATH_BYTES[rng.below(PATH_BYTES.len())] as char));
            }
        }
    }
    out
}

/// One OSC 7501 body: `state=…` plus some of the other keys, in any order,
/// with the odd corrupt value.
pub(crate) fn status_body(rng: &mut XorShift) -> Vec<u8> {
    let mut parts: Vec<String> = Vec::new();
    if rng.below(12) == 0 {
        return b"?".to_vec();
    }
    if rng.below(10) != 0 {
        parts.push(format!(
            "state={}",
            pick(rng, &["idle", "working", "done", "blocked", "error", "clear", "bogus", ""])
        ));
    }
    if rng.below(2) == 0 {
        let segments: Vec<String> = (0..1 + rng.below(4)).map(|_| name(rng, 8)).collect();
        parts.push(format!("id={}", segments.join("/")));
    }
    if rng.below(3) == 0 {
        parts.push(format!("kind={}", pick(rng, &["permission", "question", "auth", "other"])));
    }
    if rng.below(3) == 0 {
        parts.push(format!("progress={}", pick(rng, &["0", "50", "100", "101", "-1", "x", "999999999999"])));
    }
    if rng.below(3) == 0 {
        parts.push(format!("app={}", name(rng, 12)));
    }
    for key in ["title", "msg"] {
        if rng.below(3) == 0 {
            let text = match rng.below(5) {
                0 => String::new(),
                1 => "line\none".to_string(),
                2 => "\u{202e}bidi\u{202c}".to_string(),
                _ => name(rng, 30) + " é ✓",
            };
            let mut encoded = STANDARD.encode(text);
            if rng.below(8) == 0 {
                encoded.push_str(pick(rng, &["!", "=", "A", "%"]));
            }
            parts.push(format!("{key}={encoded}"));
        }
    }
    if rng.below(8) == 0 {
        parts.push(format!("{}={}", name(rng, 6), name(rng, 6)));
    }
    // Shuffle a little: swap random pairs.
    for _ in 0..rng.below(3) {
        let (a, b) = (rng.below(parts.len().max(1)), rng.below(parts.len().max(1)));
        if a < parts.len() && b < parts.len() {
            parts.swap(a, b);
        }
    }
    let mut body = parts.join(":").into_bytes();
    if rng.below(10) == 0 && !body.is_empty() {
        let at = rng.below(body.len());
        body[at] = rng.next() as u8;
    }
    body
}

/// One OSC 99 body (after `99;`): a few metadata keys, good and bad, with
/// a payload of every type, plain or base64 (cut mid-quad, padded or not,
/// or corrupt), button labels with U+2028, and the odd missing `;`.
pub(crate) fn kitty_body(rng: &mut XorShift) -> Vec<u8> {
    let mut parts: Vec<String> = Vec::new();
    if rng.below(3) != 0 {
        parts.push(format!("i={}", pick(rng, &["a", "b", "job", "q", "0", "a b", "", "x/y", "é"])));
    }
    if rng.below(2) == 0 {
        parts.push(format!("p={}", pick(rng, &["title", "body", "buttons", "close", "?", "alive", "icon", "bogus"])));
    }
    if rng.below(2) == 0 {
        parts.push(format!("d={}", pick(rng, &["0", "0", "1", "x"])));
    }
    let encoded = rng.below(3) == 0;
    if encoded {
        parts.push("e=1".to_string());
    }
    for (key, values) in [
        ("a", &["focus", "-focus", "report", "-focus,report", "wiggle"][..]),
        ("c", &["0", "1"][..]),
        ("o", &["always", "unfocused", "invisible", "never"][..]),
        ("u", &["0", "1", "2", "9"][..]),
        ("w", &["-1", "0", "1500", "-7", "x", "99999999999999999999"][..]),
        ("s", &["c2lsZW50", "c3lzdGVt", "silent", "!!"][..]),
        ("f", &["a2l0dHk=", "?"][..]),
    ] {
        if rng.below(5) == 0 {
            parts.push(format!("{key}={}", pick(rng, values)));
        }
    }
    if rng.below(10) == 0 {
        parts.push(pick(rng, &["novalue", "zz=1", "=x"]).to_string());
    }
    let text = match rng.below(5) {
        0 => String::new(),
        1 => pick(rng, &["Allow", "Deny", "", "é"]).to_string() + "\u{2028}" + &name(rng, 8),
        2 => "\u{202e}bidi ✓".to_string(),
        _ => name(rng, 30),
    };
    let payload = if encoded {
        let mut out = STANDARD.encode(text);
        match rng.below(6) {
            0 => out.truncate(rng.below(out.len() + 1)),
            1 => out.push_str(pick(rng, &["!", "=", "A", " "])),
            2 => out = out.trim_end_matches('=').to_string(),
            _ => {}
        }
        out
    } else {
        text
    };
    let separator = if rng.below(20) == 0 { "" } else { ";" };
    format!("{}{separator}{payload}", parts.join(":")).into_bytes()
}

/// One OSC body (after `ESC ]`, before the terminator) of the numbers the
/// parsers act on, or noise.
pub(crate) fn osc_body(rng: &mut XorShift) -> Vec<u8> {
    let text = match rng.below(14) {
        0..=2 => format!("7;file://{}/{}", pick(rng, &["", "host", "localhost", "h:1"]), path(rng)),
        3 => format!("7;{}", path(rng)),
        4 => {
            let quote = pick(rng, &["", "\""]);
            format!("9;9;{quote}{}{quote}", path(rng))
        }
        // VS Code's directory report and command line, with its escapes.
        5 => {
            let escape = pick(rng, &["", "\\\\", "\\x3b", "\\x5c", "\\x0a", "\\x07", "\\xff", "\\x", "\\q", ";", ";nonce"]);
            let kind = pick(rng, &["P;Cwd=", "E;", "E", "P;IsWindows=True;", "C;cmdline_url="]);
            format!("633;{kind}{}{escape}{}", path(rng), path(rng))
        }
        6 | 7 => {
            let number = pick(rng, &["133", "633"]);
            let mark = pick(rng, &["A", "B", "C", "D", "D;0", "D;1", "D;-3", "D;x", "D;999999999999", "E", "F", ""]);
            format!("{number};{mark}")
        }
        8 | 9 => return [b"7501;".as_slice(), &status_body(rng)].concat(),
        // ConEmu's progress: every state, a known one more often, and
        // percents in range, over it, negative and not numbers.
        10 if rng.below(2) == 0 => {
            let state = pick(rng, &["0", "1", "1", "2", "3", "4", "4", "5", "7", "", "x", "01"]);
            let percent = pick(rng, &["", ";0", ";30", ";100", ";250", ";-1", ";x", ";99999999999999999999", ";5;extra"]);
            format!("9;4;{state}{percent}")
        }
        10 => format!("{};{}", pick(rng, &["9", "777", "777;notify", "0009;x", "9;4"]), name(rng, 20)),
        // UAPI.15's contexts: starts and ends of a few ids (so they nest,
        // update and end), escapes good and bad, long values, unknown
        // types and fields, and a missing or misplaced `start=`.
        11 if rng.below(2) == 0 => {
            let id = pick(rng, &["a", "b", "c", "d", "a\\x3bb", "", "\\x", "\\x0a", "é"]);
            let verb = pick(rng, &["start=", "start=", "start=", "end=", "type=shell;start=", "", "start"]);
            let kind = pick(rng, &["", ";type=elevate", ";type=container", ";type=remote", ";type=shell", ";type=spaceship"]);
            let long = "v".repeat(rng.below(300));
            let field = pick(rng, &["", ";targetuser=root", ";container=web\\x3bdb", ";vm=\\xff", ";hostname=h\\q", ";pid=12", ";novalue"]);
            format!("3008;{verb}{id}{kind}{field};comm={long}")
        }
        // OSC 22 pointer shapes: every first character, names Brindle
        // shows and others, empty names, other characters and queries.
        12 if rng.below(3) != 0 => {
            let first = pick(rng, &["", "=", ">", ">", "<", "?", "x"]);
            let names: Vec<&str> = (0..rng.below(5))
                .map(|_| {
                    pick(rng, &["pointer", "crosshair", "ew-resize", "text", "", "wait", "zoom-in", "__current__", "__default__", "Pointer", "é", "a b"])
                })
                .collect();
            format!("22;{first}{}", names.join(","))
        }
        13 if rng.below(4) != 0 => return [b"99;".as_slice(), &kitty_body(rng)].concat(),
        _ => {
            let len = rng.below(24);
            return [format!("{};", rng.below(10_000)).into_bytes(), rng.bytes(len)].concat();
        }
    };
    text.into_bytes()
}

const TERMINATORS: &[&[u8]] = &[b"\x07", b"\x1b\\", b"\x18", b"\x1a", b"\x1b"];

/// A random terminal stream: text, the sequences under test with every
/// terminator, noise, then random cuts, splices and duplicated runs.
pub(crate) fn stream(rng: &mut XorShift, pieces: usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for _ in 0..pieces {
        match rng.below(14) {
            0..=5 => {
                out.extend_from_slice(b"\x1b]");
                out.extend_from_slice(&osc_body(rng));
                out.extend_from_slice(TERMINATORS[rng.below(TERMINATORS.len())]);
            }
            6 => out.extend_from_slice(b"\x1bc"),
            7 => out.extend_from_slice(pick(rng, &["\x1b[?1049h", "\x1b[?1049l", "\x1b[?2026h", "\x1b[?2026l"]).as_bytes()),
            8 => out.extend_from_slice(pick(rng, &["\x1b[3J", "\x1b[J", "\x1b[2J", "\x1b[H", "\x1b[1;1H"]).as_bytes()),
            9 => out.extend_from_slice(format!("\x1b[{};{}m", rng.below(110), rng.below(256)).as_bytes()),
            10 => out.extend_from_slice(b"\r\n"),
            11 => {
                let len = rng.below(40);
                out.extend(rng.bytes(len));
            }
            _ => {
                let len = rng.below(30);
                out.extend((0..len).map(|_| b' ' + rng.below(95) as u8));
                out.extend_from_slice(pick(rng, &["", "é", "✓", "😀", "漢"]).as_bytes());
            }
        }
    }
    // Random cuts, splices and repeats.
    for _ in 0..rng.below(4) {
        if out.is_empty() {
            break;
        }
        let a = rng.below(out.len());
        let b = a + rng.below(out.len() - a + 1);
        match rng.below(3) {
            0 => out.truncate(b.max(1)),
            1 => {
                let run = out[a..b].to_vec();
                let at = rng.below(out.len() + 1);
                out.splice(at..at, run);
            }
            _ => drop(out.drain(a..b)),
        }
    }
    out
}

/// `bytes` cut into pieces of random size (never empty unless `bytes` is).
fn pieces<'a>(rng: &mut XorShift, bytes: &'a [u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let take = (1 + rng.below(rest.len().min(48))).min(rest.len());
        let (head, tail) = rest.split_at(take);
        out.push(head);
        rest = tail;
    }
    out
}

#[test]
fn the_emulator_never_panics_on_random_streams() {
    let mut rng = XorShift::new(0x7e57_c0de_0000_0001);
    for round in 0..120 {
        // Both path styles, every other round, without taking from `rng`.
        let style = [PathStyle::Posix, PathStyle::Windows][round % 2];
        let (mut cols, mut rows) = (1 + rng.below(30) as u16, 1 + rng.below(10) as u16);
        let scrollback = 5 + rng.below(40);
        let mut emulator = Emulator::new(cols, rows, scrollback, style);
        let count = 1 + rng.below(60);
        let bytes = stream(&mut rng, count);
        for piece in pieces(&mut rng, &bytes) {
            emulator.feed(piece);
            match rng.below(8) {
                0 => {
                    (cols, rows) = (1 + rng.below(30) as u16, 1 + rng.below(10) as u16);
                    emulator.resize(cols, rows);
                }
                1 => {
                    emulator.input_seen();
                }
                _ => {}
            }
            let records = emulator.records().iter().count();
            assert!(records <= status::MAX_RECORDS, "{records} records");
        }
        let replay = emulator.render();
        let mut fresh = Emulator::new(cols, rows, scrollback, style);
        fresh.feed(&replay);
        assert!(fresh.records().iter().count() <= status::MAX_RECORDS);
        // The progress is replayed last, so nothing in the replay drops it.
        assert_eq!(fresh.progress(), emulator.progress(), "round {round}");
        // Nothing in a replay ends a context, and none outlives the limit.
        assert!(emulator.contexts().iter().count() <= crate::context::MAX_DEPTH);
        assert_eq!(fresh.contexts(), emulator.contexts(), "round {round}");
    }
}

#[test]
fn the_osc_scanner_finds_the_same_sequences_however_the_stream_is_cut() {
    let mut rng = XorShift::new(0x7e57_c0de_0000_0002);
    let mut found_any = 0usize;
    for _ in 0..400 {
        let count = 1 + rng.below(40);
        let bytes = stream(&mut rng, count);
        let whole: Vec<Found> = osc::Scanner::new().scan(&bytes);
        let mut scanner = osc::Scanner::new();
        let mut cut: Vec<Found> = Vec::new();
        for piece in pieces(&mut rng, &bytes) {
            cut.extend(scanner.scan(piece));
        }
        assert_eq!(whole, cut, "stream {bytes:?}");
        found_any += whole.len();
    }
    assert!(found_any > 0, "the generator never produced a sequence the scanner finds");
}

#[test]
fn pointer_stacks_stay_bounded_and_every_query_gets_a_well_formed_reply() {
    use crate::pointer::{STACK_DEPTH, Screen, Shape, Stacks};
    let mut rng = XorShift::new(0x7e57_c0de_0000_0004);
    let (mut ops, mut replies) = (0usize, 0usize);
    for _ in 0..2000 {
        let count = 1 + rng.below(40);
        let bytes = stream(&mut rng, count);
        let mut stacks = Stacks::default();
        for found in osc::Scanner::new().scan(&bytes) {
            let Found::Pointer(op) = found else { continue };
            ops += 1;
            let screen = Screen::of(rng.below(2) == 0);
            if let Some(reply) = stacks.apply(screen, op) {
                replies += 1;
                let answers = reply
                    .strip_prefix("\x1b]22;")
                    .and_then(|rest| rest.strip_suffix("\x1b\\"))
                    .unwrap_or_else(|| panic!("reply {reply:?}"));
                for answer in answers.split(',') {
                    let known = ["0", "1"].contains(&answer) || Shape::from_name(answer.as_bytes()).is_some();
                    assert!(known, "answer {answer:?} in {reply:?}");
                }
            }
            assert!(stacks.depth(Screen::Primary) <= STACK_DEPTH && stacks.depth(Screen::Alternate) <= STACK_DEPTH);
        }
    }
    assert!(ops > 100 && replies > 10, "only {ops} pointer ops and {replies} replies");
}

#[test]
fn kitty_chunks_hold_only_checked_ids_and_bounded_data() {
    use crate::kitty::{self, Data, KittyId};
    let mut rng = XorShift::new(0x7e57_c0de_0000_0005);
    let mut chunks = 0usize;
    for _ in 0..3000 {
        let body = kitty_body(&mut rng);
        let Some(chunk) = kitty::parse(&body) else { continue };
        chunks += 1;
        if let Some(id) = &chunk.id {
            assert_eq!(KittyId::parse(id.as_str().as_bytes()).as_ref(), Some(id), "body {body:?}");
        }
        match &chunk.data {
            Data::Plain(bytes) => assert!(bytes.len() <= kitty::MAX_PAYLOAD),
            Data::Base64(bytes) => {
                assert!(bytes.len() <= kitty::MAX_PAYLOAD);
                assert!(bytes.iter().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(b)), "body {body:?}");
            }
        }
        assert!(chunk.payload.is_text() || chunk.data.is_empty());
    }
    assert!(chunks > 1000, "only {chunks} bodies parsed as chunks");
    // And in streams, among everything else.
    let found = (0..400)
        .flat_map(|_| {
            let count = 1 + rng.below(40);
            osc::Scanner::new().scan(&stream(&mut rng, count))
        })
        .filter(|found| matches!(found, Found::Kitty(_)))
        .count();
    assert!(found > 20, "only {found} OSC 99 chunks in the streams");
}

#[test]
fn status_reports_survive_encode_and_a_second_parse() {
    let mut rng = XorShift::new(0x7e57_c0de_0000_0003);
    let mut reports = 0usize;
    for _ in 0..3000 {
        let body = status_body(&mut rng);
        if let Some(Message::Report { id, record }) = status::parse(&body) {
            let encoded = record.encode(&id);
            assert_eq!(
                status::parse(encoded.as_bytes()),
                Some(Message::Report { id: id.clone(), record: record.clone() }),
                "body {:?} encoded as {encoded:?}",
                String::from_utf8_lossy(&body),
            );
            reports += 1;
        }
    }
    assert!(reports > 100, "only {reports} bodies parsed as reports");
}
