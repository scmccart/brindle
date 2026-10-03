//! Translates GPUI keystrokes into the byte sequences a terminal application
//! expects (xterm conventions).
//!
//! Returns `None` for plain printable input; that text arrives through the
//! platform input handler instead, which is what makes IME and dead keys work.

use std::borrow::Cow;

use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers};

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn modifier_param(m: &Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8
}

fn any_modifier(m: &Modifiers) -> bool {
    m.shift || m.alt || m.control
}

/// `CSI 1 ; m X` when modified, otherwise `CSI X` or `SS3 X` in app-cursor mode.
fn cursor_key(letter: char, m: &Modifiers, mode: TermMode) -> Vec<u8> {
    if any_modifier(m) {
        format!("\x1b[1;{}{letter}", modifier_param(m)).into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

/// `CSI n ~` or `CSI n ; m ~`.
fn tilde_key(n: u8, m: &Modifiers) -> Vec<u8> {
    if any_modifier(m) {
        format!("\x1b[{n};{}~", modifier_param(m)).into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

fn with_alt(m: &Modifiers, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if m.alt {
        out.push(0x1b);
    }
    out.extend_from_slice(bytes);
    out
}

/// Control code for `ctrl-<key>`, following xterm/VT220.
fn control_code(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return match key {
            "space" => Some(0x00),
            _ => None,
        };
    }
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | '2' | ' ' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' | '~' | '`' => 0x1e,
        '_' | '7' | '-' | '/' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

pub fn to_esc_str(keystroke: &Keystroke, mode: TermMode) -> Option<Cow<'static, [u8]>> {
    let m = &keystroke.modifiers;
    // Super/command and fn combinations are reserved for the desktop and app bindings.
    if m.platform {
        return None;
    }

    let bytes: Vec<u8> = match keystroke.key.as_str() {
        "enter" => with_alt(m, b"\r"),
        "tab" if m.shift && !m.control && !m.alt => b"\x1b[Z".to_vec(),
        "tab" if m.control => return None, // tab switching
        "tab" => with_alt(m, b"\t"),
        "backspace" if m.control => with_alt(m, b"\x08"),
        "backspace" => with_alt(m, b"\x7f"),
        "escape" => with_alt(m, b"\x1b"),
        "space" if m.control => with_alt(m, b"\x00"),
        "space" if m.alt => b"\x1b ".to_vec(),
        "up" => cursor_key('A', m, mode),
        "down" => cursor_key('B', m, mode),
        "right" => cursor_key('C', m, mode),
        "left" => cursor_key('D', m, mode),
        "home" => cursor_key('H', m, mode),
        "end" => cursor_key('F', m, mode),
        "insert" => tilde_key(2, m),
        "delete" => tilde_key(3, m),
        "pageup" => tilde_key(5, m),
        "pagedown" => tilde_key(6, m),
        // F1-F4 use SS3 when unmodified, like app-mode cursor keys.
        "f1" => cursor_key('P', m, TermMode::APP_CURSOR),
        "f2" => cursor_key('Q', m, TermMode::APP_CURSOR),
        "f3" => cursor_key('R', m, TermMode::APP_CURSOR),
        "f4" => cursor_key('S', m, TermMode::APP_CURSOR),
        "f5" => tilde_key(15, m),
        "f6" => tilde_key(17, m),
        "f7" => tilde_key(18, m),
        "f8" => tilde_key(19, m),
        "f9" => tilde_key(20, m),
        "f10" => tilde_key(21, m),
        "f11" => tilde_key(23, m),
        "f12" => tilde_key(24, m),
        "f13" => tilde_key(25, m),
        "f14" => tilde_key(26, m),
        "f15" => tilde_key(28, m),
        "f16" => tilde_key(29, m),
        "f17" => tilde_key(31, m),
        "f18" => tilde_key(32, m),
        "f19" => tilde_key(33, m),
        "f20" => tilde_key(34, m),
        key if m.control => {
            let code = control_code(key)?;
            with_alt(m, &[code])
        }
        key if m.alt => {
            let text = keystroke.key_char.as_deref().unwrap_or(key);
            // Multi-character names (e.g. "menu") are not text.
            if text.chars().count() != 1 {
                return None;
            }
            let text = if m.shift && keystroke.key_char.is_none() {
                text.to_uppercase()
            } else {
                text.to_string()
            };
            with_alt(m, text.as_bytes())
        }
        _ => return None,
    };
    Some(Cow::Owned(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> Keystroke {
        let mut ks = Keystroke::parse(s).unwrap();
        // Mirror what the Linux platform layer produces for plain letters.
        if ks.key.chars().count() == 1 && !ks.modifiers.control {
            let c = if ks.modifiers.shift { ks.key.to_uppercase() } else { ks.key.clone() };
            ks.key_char = Some(c);
        }
        ks
    }

    fn esc(s: &str) -> Option<Vec<u8>> {
        to_esc_str(&ks(s), TermMode::NONE).map(|c| c.into_owned())
    }

    fn esc_app(s: &str) -> Option<Vec<u8>> {
        to_esc_str(&ks(s), TermMode::APP_CURSOR).map(|c| c.into_owned())
    }

    #[test]
    fn plain_text_goes_through_ime() {
        assert_eq!(esc("a"), None);
        assert_eq!(esc("shift-a"), None);
        assert_eq!(esc("space"), None);
    }

    #[test]
    fn control_letters() {
        assert_eq!(esc("ctrl-a"), Some(vec![1]));
        assert_eq!(esc("ctrl-c"), Some(vec![3]));
        assert_eq!(esc("ctrl-b"), Some(vec![2])); // tmux prefix
        assert_eq!(esc("ctrl-z"), Some(vec![26]));
        assert_eq!(esc("ctrl-space"), Some(vec![0]));
        assert_eq!(esc("ctrl-["), Some(vec![0x1b]));
        assert_eq!(esc("ctrl-\\"), Some(vec![0x1c]));
        assert_eq!(esc("ctrl-/"), Some(vec![0x1f]));
        assert_eq!(esc("ctrl-alt-a"), Some(vec![0x1b, 1]));
    }

    #[test]
    fn alt_is_meta() {
        assert_eq!(esc("alt-a"), Some(b"\x1ba".to_vec()));
        assert_eq!(esc("alt-shift-a"), Some(b"\x1bA".to_vec()));
        assert_eq!(esc("alt-enter"), Some(b"\x1b\r".to_vec()));
        assert_eq!(esc("alt-backspace"), Some(b"\x1b\x7f".to_vec()));
    }

    #[test]
    fn special_keys() {
        assert_eq!(esc("enter"), Some(b"\r".to_vec()));
        assert_eq!(esc("tab"), Some(b"\t".to_vec()));
        assert_eq!(esc("shift-tab"), Some(b"\x1b[Z".to_vec()));
        assert_eq!(esc("backspace"), Some(b"\x7f".to_vec()));
        assert_eq!(esc("ctrl-backspace"), Some(b"\x08".to_vec()));
        assert_eq!(esc("escape"), Some(b"\x1b".to_vec()));
        assert_eq!(esc("delete"), Some(b"\x1b[3~".to_vec()));
        assert_eq!(esc("pageup"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(esc("shift-pageup"), Some(b"\x1b[5;2~".to_vec()));
    }

    #[test]
    fn cursor_keys_respect_app_mode_and_modifiers() {
        assert_eq!(esc("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(esc_app("up"), Some(b"\x1bOA".to_vec()));
        assert_eq!(esc("ctrl-left"), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(esc_app("ctrl-left"), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(esc("shift-alt-right"), Some(b"\x1b[1;4C".to_vec()));
        assert_eq!(esc("home"), Some(b"\x1b[H".to_vec()));
        assert_eq!(esc_app("end"), Some(b"\x1bOF".to_vec()));
    }

    #[test]
    fn function_keys() {
        assert_eq!(esc("f1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(esc("shift-f1"), Some(b"\x1b[1;2P".to_vec()));
        assert_eq!(esc("f5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(esc("ctrl-f12"), Some(b"\x1b[24;5~".to_vec()));
    }

    #[test]
    fn platform_modifier_is_ignored() {
        assert_eq!(esc("super-a"), None);
        assert_eq!(esc("ctrl-tab"), None);
    }
}
