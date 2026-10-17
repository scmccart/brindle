//! The primary selection: what finishing a selection copies and a middle
//! click pastes.
//!
//! On Linux and FreeBSD it is the desktop's (X11 PRIMARY, or Wayland's
//! primary selection), through GPUI. Other platforms have no such thing, so
//! Brindle keeps its own, shared by all of its windows and invisible to
//! other applications.

use gpui::{App, ClipboardItem};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn write_primary(cx: &mut App, item: ClipboardItem) {
    cx.write_to_primary(item);
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn read_primary(cx: &App) -> Option<ClipboardItem> {
    cx.read_from_primary()
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn write_primary(cx: &mut App, item: ClipboardItem) {
    cx.default_global::<Shared>().0.write(item);
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn read_primary(cx: &App) -> Option<ClipboardItem> {
    cx.try_global::<Shared>().and_then(|shared| shared.0.read())
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
#[derive(Default)]
struct Shared(PrimaryBuffer);

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
impl gpui::Global for Shared {}

/// Brindle's own primary selection, for platforms without one.
#[derive(Default)]
#[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
struct PrimaryBuffer(Option<ClipboardItem>);

#[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
impl PrimaryBuffer {
    fn write(&mut self, item: ClipboardItem) {
        self.0 = Some(item);
    }

    fn read(&self) -> Option<ClipboardItem> {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buffer: &PrimaryBuffer) -> Option<String> {
        buffer.read().and_then(|item| item.text())
    }

    #[test]
    fn starts_empty() {
        assert_eq!(text(&PrimaryBuffer::default()), None);
    }

    #[test]
    fn latest_write_wins() {
        let mut buffer = PrimaryBuffer::default();
        buffer.write(ClipboardItem::new_string("one".into()));
        buffer.write(ClipboardItem::new_string("two".into()));
        assert_eq!(text(&buffer).as_deref(), Some("two"));
        assert_eq!(text(&buffer).as_deref(), Some("two"), "reading doesn't consume it");
    }
}
