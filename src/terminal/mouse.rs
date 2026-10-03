//! Mouse reporting (X10 / normal / UTF-8 / SGR encodings), as used by tmux,
//! vim, htop and friends.

use alacritty_terminal::term::TermMode;
use gpui::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    /// No button held (motion reports in any-event mode).
    None,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press,
    Release,
    Motion,
}

impl Button {
    pub fn from_gpui(button: gpui::MouseButton) -> Option<Self> {
        match button {
            gpui::MouseButton::Left => Some(Self::Left),
            gpui::MouseButton::Middle => Some(Self::Middle),
            gpui::MouseButton::Right => Some(Self::Right),
            gpui::MouseButton::Navigate(_) => None,
        }
    }

    fn code(self) -> u8 {
        match self {
            Button::Left => 0,
            Button::Middle => 1,
            Button::Right => 2,
            Button::None => 3,
            Button::WheelUp => 64,
            Button::WheelDown => 65,
        }
    }

    fn is_wheel(self) -> bool {
        matches!(self, Button::WheelUp | Button::WheelDown)
    }
}

/// Whether the application asked for this kind of event.
pub fn wants(mode: TermMode, action: Action, button: Button) -> bool {
    match action {
        Action::Press | Action::Release => mode.intersects(TermMode::MOUSE_MODE),
        Action::Motion if button == Button::None => mode.contains(TermMode::MOUSE_MOTION),
        Action::Motion => mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG),
    }
}

/// Encodes a mouse event at a zero-based grid cell. Returns `None` when the
/// event cannot be represented (e.g. coordinates too large for X10).
pub fn report(
    col: usize,
    line: usize,
    button: Button,
    action: Action,
    modifiers: &Modifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if !wants(mode, action, button) {
        return None;
    }
    let mut code = button.code();
    if action == Action::Motion {
        code += 32;
    }
    if modifiers.shift {
        code += 4;
    }
    if modifiers.alt {
        code += 8;
    }
    if modifiers.control {
        code += 16;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if action == Action::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{};{}{suffix}", col + 1, line + 1).into_bytes());
    }

    // Legacy encodings can't say which button was released, and don't report
    // wheel releases at all.
    if action == Action::Release {
        if button.is_wheel() {
            return None;
        }
        code = 3 + (code & !3);
    }

    let mut out = b"\x1b[M".to_vec();
    out.push(32 + code);
    if mode.contains(TermMode::UTF8_MOUSE) {
        for v in [col, line] {
            let ch = char::from_u32(32 + 1 + v as u32)?;
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    } else {
        for v in [col, line] {
            let byte = 32 + 1 + v;
            if byte > 255 {
                return None;
            }
            out.push(byte as u8);
        }
    }
    Some(out)
}

/// Focus in/out report (mode 1004).
pub fn focus_report(focused: bool, mode: TermMode) -> Option<&'static [u8]> {
    mode.contains(TermMode::FOCUS_IN_OUT).then_some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}

/// Arrow keys sent for wheel scrolling in the alternate screen when the app
/// didn't request mouse reporting (mode 1007, on by default) — makes `less`
/// and `man` scroll.
pub fn alternate_scroll(lines: i32, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
        || mode.intersects(TermMode::MOUSE_MODE)
        || lines == 0
    {
        return None;
    }
    let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
        (true, true) => b"\x1bOA",
        (true, false) => b"\x1b[A",
        (false, true) => b"\x1bOB",
        (false, false) => b"\x1b[B",
    };
    Some(seq.repeat(lines.unsigned_abs() as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        control: false,
        alt: false,
        shift: false,
        platform: false,
        function: false,
    };

    #[test]
    fn nothing_without_mouse_mode() {
        assert_eq!(report(0, 0, Button::Left, Action::Press, &NONE, TermMode::NONE), None);
    }

    #[test]
    fn sgr_press_release_and_drag() {
        let mode = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        assert_eq!(
            report(4, 9, Button::Left, Action::Press, &NONE, mode).unwrap(),
            b"\x1b[<0;5;10M"
        );
        assert_eq!(
            report(4, 9, Button::Left, Action::Release, &NONE, mode).unwrap(),
            b"\x1b[<0;5;10m"
        );
        assert_eq!(
            report(5, 9, Button::Left, Action::Motion, &NONE, mode).unwrap(),
            b"\x1b[<32;6;10M"
        );
        // Plain motion needs any-event mode (1003).
        assert_eq!(report(5, 9, Button::None, Action::Motion, &NONE, mode), None);
        let ctrl = Modifiers { control: true, ..NONE };
        assert_eq!(
            report(0, 0, Button::WheelUp, Action::Press, &ctrl, mode).unwrap(),
            b"\x1b[<80;1;1M"
        );
    }

    #[test]
    fn sgr_supports_large_coordinates() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            report(300, 400, Button::Right, Action::Press, &NONE, mode).unwrap(),
            b"\x1b[<2;301;401M"
        );
    }

    #[test]
    fn legacy_encoding() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            report(0, 0, Button::Left, Action::Press, &NONE, mode).unwrap(),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            report(0, 0, Button::Middle, Action::Release, &NONE, mode).unwrap(),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
        assert_eq!(report(300, 0, Button::Left, Action::Press, &NONE, mode), None);
        let utf8 = mode | TermMode::UTF8_MOUSE;
        let out = report(300, 0, Button::Left, Action::Press, &NONE, utf8).unwrap();
        assert_eq!(&out[..4], &[0x1b, b'[', b'M', 32]);
        assert_eq!(std::str::from_utf8(&out[4..]).unwrap().chars().next(), char::from_u32(333));
    }

    #[test]
    fn alternate_scroll_in_less() {
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        assert_eq!(alternate_scroll(2, mode).unwrap(), b"\x1b[A\x1b[A");
        assert_eq!(alternate_scroll(-1, mode | TermMode::APP_CURSOR).unwrap(), b"\x1bOB");
        assert_eq!(alternate_scroll(1, mode | TermMode::MOUSE_REPORT_CLICK), None);
        assert_eq!(alternate_scroll(1, TermMode::ALTERNATE_SCROLL), None);
    }

    #[test]
    fn focus_reports() {
        assert_eq!(focus_report(true, TermMode::FOCUS_IN_OUT), Some(&b"\x1b[I"[..]));
        assert_eq!(focus_report(false, TermMode::NONE), None);
    }
}
