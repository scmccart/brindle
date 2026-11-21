//! The notice line: a failure that belongs to a running tab, shown as one
//! dim line in its terminal, as the "connection lost" line is.
//!
//! The code says "note" because [`brindle_term_core::Notice`] already names
//! a program-status desktop notice. Both pieces are pure: the sink and
//! [`Terminal::note`](super::Terminal::note) write what they give.

/// `text` as a dim line of terminal output: `\r\n`, the text in the dim
/// style, `\r\n`.
///
/// Every C0 and C1 control in `text` is dropped, so text from a server or
/// a relay can't move the cursor, retitle the tab or end the dim style
/// (`railway R5`). Its lines are joined with `; `, so it stays one
/// line.
pub fn line(text: &str) -> String {
    let lines: Vec<String> = text
        .split(['\n', '\r'])
        .map(|part| part.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    format!("\r\n\x1b[2m{}\x1b[22m\r\n", lines.join("; "))
}

/// Which notice was shown last, so a holder that fails on every message of
/// one session shows one line until the session prints again, with no clock
/// (`fcis R2`).
#[derive(Debug, Default)]
pub struct NoteGate {
    last: Option<String>,
}

impl NoteGate {
    /// Whether `text` should be shown: not when it was the last one shown
    /// and no output has come since.
    pub fn admit(&mut self, text: &str) -> bool {
        if self.last.as_deref() == Some(text) {
            return false;
        }
        self.last = Some(text.to_string());
        true
    }

    /// The session printed something: a repeat is shown again.
    pub fn output(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_dropped() {
        // C0 (including ESC and BEL), DEL, and C1 (a CSI and an OSC start).
        let text = "a\x1b[31mb\x07c\x08d\x7fe\u{9b}f\u{9d}g\x00h\ti";
        let shown = line(text);
        assert_eq!(shown, "\r\n\x1b[2ma[31mbcdefghi\x1b[22m\r\n");
        let inside = &shown["\r\n\x1b[2m".len()..shown.len() - "\x1b[22m\r\n".len()];
        assert!(!inside.chars().any(char::is_control), "{inside:?}");
    }

    #[test]
    fn lines_are_joined() {
        assert_eq!(line("first\nsecond\r\nthird"), "\r\n\x1b[2mfirst; second; third\x1b[22m\r\n");
        assert_eq!(line("one\n\n  \ntwo\n"), "\r\n\x1b[2mone; two\x1b[22m\r\n");
    }

    #[test]
    fn a_repeat_without_output_is_refused() {
        let mut gate = NoteGate::default();
        assert!(gate.admit("the terminal server: no"));
        assert!(!gate.admit("the terminal server: no"));
        assert!(gate.admit("the terminal server: another"));
        assert!(gate.admit("the terminal server: no"));
    }

    #[test]
    fn a_repeat_after_output_is_admitted() {
        let mut gate = NoteGate::default();
        assert!(gate.admit("no"));
        gate.output();
        assert!(gate.admit("no"));
        assert!(!gate.admit("no"));
    }
}
