//! A command's output as plain text (design decision 6 of
//! `agents-tool-content`): [`feed`] removes escape sequences (CSI, the
//! string forms OSC, DCS, SOS, PM and APC, and single-character escapes)
//! and other controls, keeps `\n` and `\t`, and resolves a carriage return
//! by keeping only what follows a line's last one, so a progress bar leaves
//! its final state.
//!
//! Text already returned can't be taken back, so the line being written is
//! held in the [`State`] until its `\n` arrives (or [`finish`] at the
//! command's exit). A held line past [`ITEM_LIMIT`] is let go as it stands.
//! The state is part of the session and of its checkpoint, so a sequence
//! split across two updates is still removed.

use brindle_transcript::ITEM_LIMIT;

/// The most of one string form (an OSC title, a DCS payload) skipped before
/// it is taken as unterminated and the text after it shown again.
pub const MAX_STRING: usize = 64 * 1024;

/// Where the filter is between two pieces of output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub mode: Mode,
    /// The line being written, not yet returned.
    pub line: String,
    /// A carriage return seen last: the line restarts unless `\n` follows.
    pub cr: bool,
    /// The start of a UTF-8 character cut off by the end of a piece.
    pub partial: Vec<u8>,
}

/// Inside which kind of sequence the filter is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Text,
    /// After `ESC`.
    Escape,
    /// After `ESC` and an intermediate byte.
    EscapeIntermediate,
    /// A control sequence (`ESC [` or `CSI`), up to its final byte.
    Csi,
    /// A string form, up to `BEL` or `ST`, with how much of it was skipped.
    String { skipped: usize },
    /// `ESC` inside a string form: `\` ends it.
    StringEscape { skipped: usize },
}

/// The plain text `bytes` add: every line they complete, with its `\n`.
pub fn feed(state: &mut State, bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut bytes_in = std::mem::take(&mut state.partial);
    bytes_in.extend_from_slice(bytes);
    let mut rest = bytes_in.as_slice();
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                chars(state, text, &mut out);
                break;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                chars(
                    state,
                    std::str::from_utf8(valid).unwrap_or_default(),
                    &mut out,
                );
                match error.error_len() {
                    Some(len) => {
                        chars(state, "\u{fffd}", &mut out);
                        rest = after.get(len..).unwrap_or_default();
                    }
                    // A character the next piece completes.
                    None => {
                        state.partial = after.to_vec();
                        break;
                    }
                }
            }
        }
    }
    out
}

/// The held line, at the command's exit: what [`feed`] would return once
/// a `\n` came. The state is then as new.
pub fn finish(state: &mut State) -> String {
    let mut out = String::new();
    if !state.partial.is_empty() {
        chars(state, "\u{fffd}", &mut out);
        state.partial.clear();
    }
    let line = std::mem::take(&mut state.line);
    *state = State::default();
    out.push_str(&line);
    out
}

fn chars(state: &mut State, text: &str, out: &mut String) {
    for c in text.chars() {
        char(state, c, out);
    }
}

fn char(state: &mut State, c: char, out: &mut String) {
    match state.mode {
        Mode::Text => text(state, c, out),
        Mode::Escape => {
            state.mode = match c {
                '[' => Mode::Csi,
                ']' | 'P' | 'X' | '^' | '_' => Mode::String { skipped: 0 },
                ' '..='/' => Mode::EscapeIntermediate,
                _ => Mode::Text,
            };
        }
        Mode::EscapeIntermediate => {
            if !(' '..='/').contains(&c) {
                state.mode = Mode::Text;
            }
        }
        Mode::Csi => match c {
            // Parameters and intermediates.
            ' '..='?' => {}
            // The final byte.
            '@'..='~' => state.mode = Mode::Text,
            // Anything else cuts the sequence short and is read as text.
            _ => {
                state.mode = Mode::Text;
                text(state, c, out);
            }
        },
        Mode::String { skipped } => match c {
            '\u{7}' | '\u{9c}' => state.mode = Mode::Text,
            '\u{1b}' => state.mode = Mode::StringEscape { skipped },
            _ => skip(state, skipped, c),
        },
        Mode::StringEscape { .. } => {
            if c == '\\' {
                state.mode = Mode::Text;
            } else {
                // Another escape ends the string and starts itself.
                state.mode = Mode::Escape;
                char(state, c, out);
            }
        }
    }
}

/// One more character of a string form, which past [`MAX_STRING`] is
/// taken as unterminated: what follows is text again.
fn skip(state: &mut State, skipped: usize, c: char) {
    let skipped = skipped.saturating_add(c.len_utf8());
    state.mode = if skipped > MAX_STRING {
        Mode::Text
    } else {
        Mode::String { skipped }
    };
}

fn text(state: &mut State, c: char, out: &mut String) {
    if state.cr {
        state.cr = false;
        if c != '\n' {
            state.line.clear();
        }
    }
    match c {
        '\n' => {
            out.push_str(&state.line);
            out.push('\n');
            state.line.clear();
        }
        '\r' => state.cr = true,
        '\t' => hold(state, c, out),
        '\u{1b}' => state.mode = Mode::Escape,
        '\u{9b}' => state.mode = Mode::Csi,
        '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => {
            state.mode = Mode::String { skipped: 0 };
        }
        c if c.is_control() => {}
        c => hold(state, c, out),
    }
}

/// Adds `c` to the held line, letting the line go as it stands once it
/// passes [`ITEM_LIMIT`].
fn hold(state: &mut State, c: char, out: &mut String) {
    if state.line.len() + c.len_utf8() > ITEM_LIMIT {
        out.push_str(&state.line);
        state.line.clear();
    }
    state.line.push(c);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(bytes: &[u8]) -> String {
        let mut state = State::default();
        let mut out = feed(&mut state, bytes);
        out.push_str(&finish(&mut state));
        out
    }

    #[test]
    fn each_escape_form_is_removed() {
        let table: [(&[u8], &str); 12] = [
            (b"\x1b[31mred\x1b[0m\n", "red\n"),
            (b"\x1b[38;2;1;2;3mrgb\n", "rgb\n"),
            (b"\x1b[?25lhidden\x1b[?25h\n", "hidden\n"),
            (b"\x1b]0;title\x07after\n", "after\n"),
            (b"\x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\\n", "link\n"),
            (b"\x1bPq#0;2;0;0;0\x1b\\dcs\n", "dcs\n"),
            (b"\x1b_apc\x1b\\\x1b^pm\x1b\\\x1bXsos\x1b\\x\n", "x\n"),
            (b"\x1b=\x1b>\x1b7\x1b8\x1b(Bkept\n", "kept\n"),
            ("\u{9b}1mc1\u{9d}title\u{9c}!\n".as_bytes(), "c1!\n"),
            (b"a\tb\x07\x08\x00c\n", "a\tbc\n"),
            (b"\x1b[1;\x01x\n", "x\n"),
            (b"no newline", "no newline"),
        ];
        for (bytes, want) in table {
            assert_eq!(plain(bytes), want, "{bytes:?}");
        }
    }

    #[test]
    fn a_carriage_return_keeps_what_follows_it() {
        assert_eq!(plain(b"10%\r50%\r100%\ndone\n"), "100%\ndone\n");
        assert_eq!(plain(b"a\r\nb\r\n"), "a\nb\n");
        assert_eq!(plain(b"partial\r"), "partial");
        // A progress line in pieces, its bar redrawn in place.
        let mut state = State::default();
        let mut out = String::new();
        for step in 0..=10 {
            let piece = format!(
                "\r\x1b[2K[{}{}] {}%",
                "#".repeat(step),
                " ".repeat(10 - step),
                step * 10
            );
            out.push_str(&feed(&mut state, piece.as_bytes()));
        }
        out.push_str(&feed(&mut state, b"\n"));
        assert_eq!(out, "[##########] 100%\n");
    }

    #[test]
    fn every_split_of_a_sequence_across_two_pieces_reads_as_whole() {
        let whole = "a\x1b[31mb\x1b]0;t\x1b\\c\u{9b}0m\u{e9}\u{1f600}\r\nd\x1bPx\x07e\n".as_bytes();
        let want = plain(whole);
        assert_eq!(want, "abc\u{e9}\u{1f600}\nde\n");
        for at in 0..=whole.len() {
            let mut state = State::default();
            let mut out = feed(&mut state, &whole[..at]);
            out.push_str(&feed(&mut state, &whole[at..]));
            out.push_str(&finish(&mut state));
            assert_eq!(out, want, "split at {at}");
        }
    }

    #[test]
    fn an_unterminated_string_form_stops_hiding_text_past_its_bound() {
        let mut state = State::default();
        let mut bytes = b"\x1b]0;".to_vec();
        // `0;` and the x's reach the bound; the `y` passes it.
        bytes.extend(std::iter::repeat_n(b'x', MAX_STRING - 2));
        assert_eq!(feed(&mut state, &bytes), "");
        assert!(matches!(state.mode, Mode::String { .. }));
        assert!(state.line.is_empty());
        assert_eq!(feed(&mut state, b"yshown\n"), "shown\n");
    }

    #[test]
    fn invalid_utf8_reads_as_replacement_characters() {
        assert_eq!(plain(b"a\xffb\xc3\n"), "a\u{fffd}b\u{fffd}\n");
        let mut state = State::default();
        assert_eq!(feed(&mut state, b"x\xe2\x82"), "");
        assert_eq!(state.partial, b"\xe2\x82");
        assert_eq!(feed(&mut state, b"\xac\n"), "x\u{20ac}\n");
        // A cut character the command never finished.
        assert_eq!(feed(&mut state, b"y\xe2"), "");
        assert_eq!(finish(&mut state), "y\u{fffd}");
    }

    #[test]
    fn a_line_past_the_item_limit_is_let_go() {
        let mut state = State::default();
        let long = "z".repeat(ITEM_LIMIT + 10);
        let out = feed(&mut state, long.as_bytes());
        assert_eq!(out.len(), ITEM_LIMIT);
        assert_eq!(state.line.len(), 10);
    }
}
