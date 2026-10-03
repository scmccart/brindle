//! tmux control-mode protocol (`tmux -C`): line parsing and output decoding.
//!
//! Every line tmux writes is either a notification starting with `%`, or
//! part of a command response framed by `%begin` / `%end` (or `%error`).

/// tmux ids: `@1` windows, `%1` panes, `$1` sessions.
pub type WindowId = u32;
pub type PaneId = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    /// Start of a command response. `ours` is set for commands sent by this
    /// client (flags bit 0).
    Begin { number: u64, ours: bool },
    End { number: u64, ours: bool },
    Error { number: u64, ours: bool },
    Output { pane: PaneId, data: Vec<u8> },
    LayoutChange { window: WindowId, layout: String, flags: String },
    WindowAdd(WindowId),
    WindowClose(WindowId),
    WindowRenamed { window: WindowId, name: String },
    WindowPaneChanged { window: WindowId, pane: PaneId },
    SessionWindowChanged { window: WindowId },
    SessionChanged { name: String },
    SessionRenamed { name: String },
    Exit(Option<String>),
    /// Anything else (`%sessions-changed`, `%message`, …) or an unparseable line.
    Other(String),
}

fn id(s: &str, sigil: char) -> Option<u32> {
    s.strip_prefix(sigil)?.parse().ok()
}

fn begin_fields(rest: &str) -> Option<(u64, bool)> {
    let mut parts = rest.split(' ');
    let _time = parts.next()?;
    let number = parts.next()?.parse().ok()?;
    let flags: u32 = parts.next()?.parse().ok()?;
    Some((number, flags & 1 != 0))
}

/// Parses one notification line (without the trailing newline). Lines that
/// are part of a command response should not be passed here.
pub fn parse_line(line: &[u8]) -> Notification {
    // `%output` is the hot path and may carry arbitrary bytes, so handle it
    // before any UTF-8 conversion.
    if let Some(rest) = line.strip_prefix(b"%output ") {
        if let Some(space) = rest.iter().position(|&b| b == b' ') {
            if let Some(pane) = std::str::from_utf8(&rest[..space]).ok().and_then(|s| id(s, '%')) {
                return Notification::Output { pane, data: decode_output(&rest[space + 1..]) };
            }
        } else if let Some(pane) = std::str::from_utf8(rest).ok().and_then(|s| id(s, '%')) {
            return Notification::Output { pane, data: Vec::new() };
        }
    }

    let text = String::from_utf8_lossy(line);
    let (kind, rest) = text.split_once(' ').unwrap_or((&text, ""));
    let other = || Notification::Other(text.to_string());
    let parsed = match kind {
        "%begin" => begin_fields(rest).map(|(number, ours)| Notification::Begin { number, ours }),
        "%end" => begin_fields(rest).map(|(number, ours)| Notification::End { number, ours }),
        "%error" => begin_fields(rest).map(|(number, ours)| Notification::Error { number, ours }),
        "%layout-change" => {
            let mut parts = rest.split(' ');
            let window = parts.next().and_then(|s| id(s, '@'));
            let full = parts.next();
            // The visible layout reflects zoom; fall back to the full one.
            let layout = parts.next().or(full).map(str::to_string);
            let flags = parts.next().unwrap_or("").to_string();
            window.zip(layout).map(|(window, layout)| Notification::LayoutChange { window, layout, flags })
        }
        "%window-add" => id(rest, '@').map(Notification::WindowAdd),
        "%window-close" | "%unlinked-window-close" => id(rest, '@').map(Notification::WindowClose),
        "%window-renamed" => rest.split_once(' ').and_then(|(w, name)| {
            Some(Notification::WindowRenamed { window: id(w, '@')?, name: name.to_string() })
        }),
        "%window-pane-changed" => rest.split_once(' ').and_then(|(w, p)| {
            Some(Notification::WindowPaneChanged { window: id(w, '@')?, pane: id(p, '%')? })
        }),
        "%session-window-changed" => rest
            .split_once(' ')
            .and_then(|(_, w)| Some(Notification::SessionWindowChanged { window: id(w, '@')? })),
        "%session-changed" => rest
            .split_once(' ')
            .map(|(_, name)| Notification::SessionChanged { name: name.to_string() }),
        "%session-renamed" => {
            let name = rest.split_once(' ').map(|(_, n)| n).unwrap_or(rest);
            Some(Notification::SessionRenamed { name: name.to_string() })
        }
        "%exit" => Some(Notification::Exit((!rest.is_empty()).then(|| rest.to_string()))),
        _ => None,
    };
    parsed.unwrap_or_else(other)
}

/// A parsed unit of control-mode output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Notification(Notification),
    /// A complete command response. `ours` is false for responses to
    /// commands this client didn't send (e.g. the initial attach).
    Response { ours: bool, ok: bool, body: Vec<Vec<u8>> },
}

/// Groups lines into notifications and `%begin`…`%end` responses. tmux
/// never sends notifications inside a response block, and a block is only
/// closed by an `%end`/`%error` carrying the same command number.
#[derive(Default)]
pub struct Collector {
    current: Option<(u64, bool, Vec<Vec<u8>>)>,
}

impl Collector {
    pub fn feed_line(&mut self, line: &[u8]) -> Option<Event> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some((number, ours, body)) = &mut self.current {
            if line.starts_with(b"%end ") || line.starts_with(b"%error ") {
                let closing = match parse_line(line) {
                    Notification::End { number: n, .. } if n == *number => Some(true),
                    Notification::Error { number: n, .. } if n == *number => Some(false),
                    _ => None,
                };
                if let Some(ok) = closing {
                    let ours = *ours;
                    let body = std::mem::take(body);
                    self.current = None;
                    return Some(Event::Response { ours, ok, body });
                }
            }
            body.push(line.to_vec());
            return None;
        }
        match parse_line(line) {
            Notification::Begin { number, ours } => {
                self.current = Some((number, ours, Vec::new()));
                None
            }
            notification => Some(Event::Notification(notification)),
        }
    }
}

/// Decodes `%output` data: tmux escapes bytes < 0x20 and `\` as `\ooo` octal.
pub fn decode_output(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b == b'\\'
            && i + 3 < data.len()
            && data[i + 1..i + 4].iter().all(|d| (b'0'..=b'7').contains(d))
        {
            let v = (data[i + 1] - b'0') as u32 * 64 + (data[i + 2] - b'0') as u32 * 8 + (data[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

/// Makes a user-typed command line start new windows and panes in the
/// client's current pane directory: `-c "#{pane_current_path}"` is inserted
/// after each top-level `new-window` / `split-window` (or alias). Quoted
/// text, escapes and `{ … }` blocks are copied untouched, and a `-c` the user
/// gave comes later on the line, so it wins.
pub fn with_start_dir(line: &str) -> String {
    const COMMANDS: [&str; 4] = ["new-window", "neww", "split-window", "splitw"];
    const START_DIR: &str = " -c \"#{pane_current_path}\"";

    let mut inserts = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut expecting_command = true;
    let mut word_start: Option<usize> = None;
    let mut finish_word = |start: &mut Option<usize>, end: usize, expecting: &mut bool| {
        if let Some(begin) = start.take() {
            if COMMANDS.contains(&&line[begin..end]) {
                inserts.push(end);
            }
            *expecting = false;
        }
    };
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        if depth == 0 && (c.is_whitespace() || c == ';') {
            finish_word(&mut word_start, i, &mut expecting_command);
            if c == ';' {
                expecting_command = true;
            }
            continue;
        }
        if depth == 0 && expecting_command && word_start.is_none() {
            word_start = Some(i);
        }
        match c {
            '\\' => escaped = true,
            '\'' | '"' => quote = Some(c),
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    finish_word(&mut word_start, line.len(), &mut expecting_command);

    let mut out = String::with_capacity(line.len() + inserts.len() * START_DIR.len());
    let mut last = 0;
    for at in inserts {
        out.push_str(&line[last..at]);
        out.push_str(START_DIR);
        last = at;
    }
    out.push_str(&line[last..]);
    out
}

/// Quotes an argument for a tmux command line.
pub fn quote(arg: &str) -> String {
    let escaped = arg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', " ");
    format!("\"{escaped}\"")
}

/// `send-keys` commands delivering `bytes` literally to a pane, chunked so
/// no single command line gets too long.
pub fn send_keys_commands(pane: PaneId, bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(256)
        .map(|chunk| {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut cmd = format!("send-keys -t %{pane} -H");
            cmd.reserve(chunk.len() * 3);
            for &b in chunk {
                cmd.push(' ');
                cmd.push(HEX[(b >> 4) as usize] as char);
                cmd.push(HEX[(b & 0xf) as usize] as char);
            }
            cmd
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines below are taken from a real tmux 3.6 `-C` session.

    #[test]
    fn start_dir_is_added_to_top_level_window_and_pane_commands() {
        const C: &str = r##"-c "#{pane_current_path}""##;
        assert_eq!(with_start_dir("split-window -h"), format!("split-window {C} -h"));
        assert_eq!(with_start_dir("new-window"), format!("new-window {C}"));
        assert_eq!(with_start_dir("  splitw"), format!("  splitw {C}"));
        assert_eq!(with_start_dir("neww ; splitw -v"), format!("neww {C} ; splitw {C} -v"));
        assert_eq!(with_start_dir("neww; splitw"), format!("neww {C}; splitw {C}"));
        assert_eq!(with_start_dir("new-window -c /tmp"), format!("new-window {C} -c /tmp"));
        assert_eq!(with_start_dir("rename-window x ; neww"), format!("rename-window x ; neww {C}"));
    }

    #[test]
    fn start_dir_leaves_other_text_alone() {
        for line in [
            "select-layout tiled",
            "rename-window 'a;b'",
            "rename-window 'a; neww'",
            r#"display-message "x; neww""#,
            r"rename-window a\; neww",
            "if-shell true { split-window ; neww }",
            "set -g status off",
            "",
        ] {
            assert_eq!(with_start_dir(line), line);
        }
        // Commands after a brace block are still top level.
        assert_eq!(
            with_start_dir("if-shell true { neww } ; splitw"),
            r##"if-shell true { neww } ; splitw -c "#{pane_current_path}""##
        );
    }

    #[test]
    fn command_framing() {
        assert_eq!(parse_line(b"%begin 1791034936 272 0"), Notification::Begin { number: 272, ours: false });
        assert_eq!(parse_line(b"%end 1791034938 283 1"), Notification::End { number: 283, ours: true });
        assert_eq!(parse_line(b"%error 1791034943 302 1"), Notification::Error { number: 302, ours: true });
    }

    #[test]
    fn output_is_decoded() {
        let line = b"%output %0 ls\\015\\012\\033[?2004l\\015";
        assert_eq!(parse_line(line), Notification::Output { pane: 0, data: b"ls\r\n\x1b[?2004l\r".to_vec() });
        // Backslash is escaped as \134; UTF-8 passes through untouched.
        let line = "%output %12 a\\134b \u{e0b0}".as_bytes();
        assert_eq!(
            parse_line(line),
            Notification::Output { pane: 12, data: "a\\b \u{e0b0}".as_bytes().to_vec() }
        );
    }

    #[test]
    fn decode_edge_cases() {
        assert_eq!(decode_output(b"\\033"), b"\x1b");
        assert_eq!(decode_output(b"trailing\\"), b"trailing\\");
        assert_eq!(decode_output(b"\\09x"), b"\\09x");
        assert_eq!(decode_output(b"\\000\\377"), &[0u8, 255]);
    }

    #[test]
    fn layout_and_window_notifications() {
        assert_eq!(
            parse_line(b"%layout-change @0 f91d,120x40,0,0{60x40,0,0,0,59x40,61,0,1} f91d,120x40,0,0{60x40,0,0,0,59x40,61,0,1} *"),
            Notification::LayoutChange {
                window: 0,
                layout: "f91d,120x40,0,0{60x40,0,0,0,59x40,61,0,1}".into(),
                flags: "*".into()
            }
        );
        assert_eq!(parse_line(b"%window-add @1"), Notification::WindowAdd(1));
        assert_eq!(parse_line(b"%unlinked-window-close @1"), Notification::WindowClose(1));
        assert_eq!(parse_line(b"%window-close @3"), Notification::WindowClose(3));
        assert_eq!(
            parse_line(b"%window-renamed @1 my window"),
            Notification::WindowRenamed { window: 1, name: "my window".into() }
        );
        assert_eq!(
            parse_line(b"%window-pane-changed @0 %2"),
            Notification::WindowPaneChanged { window: 0, pane: 2 }
        );
        assert_eq!(parse_line(b"%session-window-changed $0 @1"), Notification::SessionWindowChanged { window: 1 });
        assert_eq!(parse_line(b"%session-changed $0 spike"), Notification::SessionChanged { name: "spike".into() });
        assert_eq!(parse_line(b"%exit"), Notification::Exit(None));
        assert_eq!(parse_line(b"%exit server exited"), Notification::Exit(Some("server exited".into())));
        assert_eq!(parse_line(b"%sessions-changed"), Notification::Other("%sessions-changed".into()));
    }

    #[test]
    fn collector_matches_the_spike_transcript() {
        let lines: &[&[u8]] = &[
            b"%begin 1791034936 272 0",
            b"%end 1791034936 272 0",
            b"%window-add @0",
            b"%session-changed $0 spike",
            b"%output %0 hi",
            b"%begin 1791034937 280 1",
            b"%end 1791034937 280 1",
            b"%layout-change @0 aafd,120x40,0,0,0 aafd,120x40,0,0,0 *",
            b"%begin 1791034938 283 1",
            b"@0 bash aafd,120x40,0,0,0 1",
            b"%end 1791034938 283 1",
            b"%begin 1791034942 301 1",
            b"%end of a captured line that looks like framing",
            b"",
            b"%end 1791034942 301 1",
            b"%begin 1791034943 302 1",
            b"parse error: unknown command: bogus-command",
            b"%error 1791034943 302 1",
            b"%exit",
        ];
        let mut collector = Collector::default();
        let events: Vec<Event> = lines.iter().filter_map(|l| collector.feed_line(l)).collect();
        let n = |n| Event::Notification(n);
        assert_eq!(
            events,
            vec![
                Event::Response { ours: false, ok: true, body: vec![] },
                n(Notification::WindowAdd(0)),
                n(Notification::SessionChanged { name: "spike".into() }),
                n(Notification::Output { pane: 0, data: b"hi".to_vec() }),
                Event::Response { ours: true, ok: true, body: vec![] },
                n(Notification::LayoutChange { window: 0, layout: "aafd,120x40,0,0,0".into(), flags: "*".into() }),
                Event::Response { ours: true, ok: true, body: vec![b"@0 bash aafd,120x40,0,0,0 1".to_vec()] },
                Event::Response {
                    ours: true,
                    ok: true,
                    body: vec![b"%end of a captured line that looks like framing".to_vec(), b"".to_vec()]
                },
                Event::Response {
                    ours: true,
                    ok: false,
                    body: vec![b"parse error: unknown command: bogus-command".to_vec()]
                },
                n(Notification::Exit(None)),
            ]
        );
    }

    #[test]
    fn send_keys_hex() {
        assert_eq!(send_keys_commands(3, b"ls\r"), vec!["send-keys -t %3 -H 6c 73 0d".to_string()]);
        assert_eq!(send_keys_commands(1, &[0u8; 600]).len(), 3);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("a \"b\" $c\\"), "\"a \\\"b\\\" \\$c\\\\\"");
    }
}
