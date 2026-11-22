//! The platform pointer: showing the one a frame sets at once.
//!
//! GPUI sets the pointer as each frame ends (`Window::set_cursor_style`
//! during paint, applied by the draw's `reset_cursor_style`). On Wayland and
//! X11 that shows it then. On Windows GPUI only posts it to the window
//! (`WM_USER + 1`), which records it and calls `SetCursor` only when the
//! pointer is hidden or unhidden; the window shows a recorded pointer at its
//! next `WM_SETCURSOR`. Windows sends that when the mouse moves, and never
//! while the mouse is captured, which GPUI does on every press until the
//! release. GPUI offers no way to show it sooner (`Window::refresh` only
//! draws again, which records it again), so [`show_now`] sends the window a
//! `WM_SETCURSOR` of its own. When to call it is
//! `terminal::pointer::Shown::frame`'s decision.

use gpui::{App, Window};

/// Has the platform show the pointer the frame being painted sets, rather
/// than at the mouse's next move. Call it during paint.
#[cfg(windows)]
pub fn show_now(window: &Window, cx: &mut App) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{HTCLIENT, PostMessageW, WM_MOUSEMOVE, WM_SETCURSOR};

    let Some(hwnd) = crate::win::hwnd(window) else { return };
    // Deferred to the end of the frame's update, so it is posted after the
    // pointer GPUI posts as the draw ends; the window handles its posted
    // messages in order. `HTCLIENT` (the client area) in the low word is
    // what makes GPUI's handler call `SetCursor` with the recorded pointer.
    cx.defer(move |_| {
        let lparam = LPARAM(((WM_MOUSEMOVE << 16) | HTCLIENT) as isize);
        // SAFETY: posts a message to this process's window; nothing is
        // borrowed, and a window already gone only fails the call.
        if let Err(error) = unsafe { PostMessageW(Some(hwnd), WM_SETCURSOR, WPARAM(hwnd.0 as usize), lparam) } {
            log::debug!("showing the pointer: {error}");
        }
    });
}

/// Has the platform show the pointer the frame being painted sets: GPUI's
/// Wayland and X11 platforms show it as the frame ends, so nothing is left
/// to do.
#[cfg(not(windows))]
pub fn show_now(_window: &Window, _cx: &mut App) {}
