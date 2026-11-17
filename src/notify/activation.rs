//! Raising the window when the user clicks one of its notifications, or
//! when a later launch hands its request to this Brindle.
//!
//! On Wayland a window may only take focus with an xdg-activation token the
//! compositor issued for a user action. GPUI's `activate_window` asks for a
//! token of its own, which compositors refuse when the window isn't focused
//! (GNOME then shows "Brindle is ready" instead). The notification server
//! hands us a valid token with the click (`ActivationToken`), and a launcher
//! hands one to the program it starts (`XDG_ACTIVATION_TOKEN`), so we use it
//! on GPUI's own surface: we attach to GPUI's `wl_display` with a separate
//! event queue, bind `xdg_activation_v1`, and activate the surface.
//! Anything else (X11, no token, a failure) falls back to `activate_window`.
//!
//! Binding needs a registry round trip, which waits on the compositor, so
//! the attachment is made once, on the background executor, as soon as the
//! window has opened ([`prepare`]); the UI thread never waits on it
//! (`process R13`). A raise with a token that comes while it is being made
//! waits for it in a task, then raises with the token, or plainly if the
//! attachment failed ([`policy::raise_plan`]).

use std::cell::RefCell;
use std::ffi::c_void;

use gpui::{AnyWindowHandle, App, Window};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};

use super::policy::{self, AttachState, RaisePlan};

/// Starts attaching to GPUI's Wayland display for `window`, on the
/// background executor, so later raises can carry a token. Does nothing
/// off Wayland, or when an attachment was already started. Call it once
/// the window has opened.
pub fn prepare(window: &Window, cx: &mut App) {
    let display = match wayland_pointers(window) {
        Ok(Some((display, _))) => display as usize,
        Ok(None) => return,
        Err(err) => {
            settle(Err(format!("{err:#}")), cx);
            return;
        }
    };
    let starts = ATTACHMENT.with_borrow_mut(|attachment| match attachment {
        Attachment::Unused => {
            *attachment = Attachment::Pending { waiting: Vec::new() };
            true
        }
        Attachment::Pending { .. } | Attachment::Ready(_) | Attachment::Failed(_) => false,
    });
    if !starts {
        return;
    }
    let attaching = cx.background_executor().spawn(async move {
        // SAFETY: `display` is GPUI's `wl_display`, which lives as long as
        // the app; a foreign backend never closes it.
        unsafe { attach(display) }.map_err(|err| format!("{err:#}"))
    });
    cx.spawn(async move |cx| {
        let attached = attaching.await;
        cx.update(|cx| settle(attached, cx)).ok();
    })
    .detach();
}

/// Brings `window` to the front, using `token` when there is one and the
/// attachment is made; while it is being made, the raise waits for it.
/// `cx` keeps one signature on every platform.
pub fn raise(window: &mut Window, token: Option<&str>, _cx: &mut App) {
    ATTACHMENT.with_borrow_mut(|attachment| match (policy::raise_plan(attachment.state(), token.is_some()), token) {
        (RaisePlan::WithToken, Some(token)) => raise_with_token(attachment, window, token),
        (RaisePlan::AfterAttach, Some(token)) => raise_after_attach(attachment, window, token),
        (RaisePlan::Plain, Some(_)) => {
            if let Attachment::Failed(why) = attachment {
                log::debug!("raising the window without its activation token: {why}");
            }
            window.activate_window();
        }
        // A plan with a token always has one.
        (RaisePlan::WithToken | RaisePlan::AfterAttach | RaisePlan::Plain, None) => window.activate_window(),
    });
}

/// Activates `window`'s surface with `token` through the attachment, or
/// raises it plainly when that can't be done.
fn raise_with_token(attachment: &Attachment, window: &mut Window, token: &str) {
    let raised = match attachment {
        Attachment::Ready(attached) => activate_with_token(attached, window, token),
        // Not reached: the plan uses the token only once attached.
        Attachment::Unused | Attachment::Pending { .. } | Attachment::Failed(_) => Ok(false),
    };
    match raised {
        Ok(true) => {
            log::debug!("raised the window with an activation token");
            return;
        }
        Ok(false) => log::debug!("not a Wayland window; raising it without the token"),
        Err(err) => log::warn!("raising the window with an activation token failed: {err:#}"),
    }
    window.activate_window();
}

/// Keeps `window` and `token` until the attachment is made or has failed
/// ([`settle`] raises them then).
fn raise_after_attach(attachment: &mut Attachment, window: &mut Window, token: &str) {
    match attachment {
        Attachment::Pending { waiting } => {
            log::debug!("raising the window once the activation attachment is made");
            // GPUI's handle, not raw-window-handle's.
            waiting.push((Window::window_handle(window), token.to_string()));
        }
        // Not reached: the plan waits only while the attachment is pending.
        Attachment::Unused | Attachment::Ready(_) | Attachment::Failed(_) => window.activate_window(),
    }
}

/// Stores the attachment's outcome (a failure is logged here, once) and
/// raises the windows that waited for it.
fn settle(attached: Result<Attached, String>, cx: &mut App) {
    let outcome = match attached {
        Ok(attached) => {
            log::debug!("attached to the Wayland display for activation tokens");
            Attachment::Ready(attached)
        }
        Err(why) => {
            log::warn!("windows will be raised without activation tokens: {why}");
            Attachment::Failed(why)
        }
    };
    let waiting = ATTACHMENT.with_borrow_mut(|attachment| match attachment {
        Attachment::Unused => {
            *attachment = outcome;
            Vec::new()
        }
        Attachment::Pending { waiting } => {
            let waiting = std::mem::take(waiting);
            *attachment = outcome;
            waiting
        }
        // Settled already (not reached: one attachment is started).
        Attachment::Ready(_) | Attachment::Failed(_) => Vec::new(),
    });
    for (window, token) in waiting {
        window.update(cx, |_, window, cx| raise(window, Some(&token), cx)).ok();
    }
}

/// GPUI's `wl_display` and the window's `wl_surface`, on Wayland.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enums: raw_window_handle's RawDisplayHandle and RawWindowHandle, of which only Wayland's counts"
)]
fn wayland_pointers(window: &Window) -> anyhow::Result<Option<(*mut c_void, *mut c_void)>> {
    // GPUI's X11 windows panic when asked for their raw handles
    // (`unimplemented!`), so only ask on the backend GPUI picked for Wayland.
    if gpui::guess_compositor() != "Wayland" {
        return Ok(None);
    }
    let display = match HasDisplayHandle::display_handle(window).map_err(|e| anyhow::anyhow!("{e:?}"))?.as_raw() {
        RawDisplayHandle::Wayland(handle) => handle.display.as_ptr(),
        _ => return Ok(None),
    };
    let surface = match HasWindowHandle::window_handle(window).map_err(|e| anyhow::anyhow!("{e:?}"))?.as_raw() {
        RawWindowHandle::Wayland(handle) => handle.surface.as_ptr(),
        _ => return Ok(None),
    };
    Ok(Some((display, surface)))
}

/// Returns `Ok(false)` when the window isn't a Wayland window.
fn activate_with_token(attached: &Attached, window: &Window, token: &str) -> anyhow::Result<bool> {
    let Some((display, surface)) = wayland_pointers(window)? else { return Ok(false) };
    anyhow::ensure!(display as usize == attached.display, "the window isn't on the display Brindle attached to");
    // SAFETY: `surface` comes from GPUI's live window, which outlives this
    // call (we're inside its update), and is only used here.
    let surface = unsafe { surface_proxy(attached, surface)? };
    attached.activation.activate(token.to_string(), &surface);
    // Doesn't wait: it reports would-block instead (`wl_display_flush`).
    attached.connection.flush()?;
    Ok(true)
}

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(_: &mut Self, _: &XdgActivationV1, _: xdg_activation_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// Our attachment to GPUI's display, made once: binding needs a registry
/// round trip, and the registry can't be destroyed again. It is made on the
/// background executor, so it is `Send`: the display is kept as an address.
struct Attached {
    /// The `wl_display` it was made for.
    display: usize,
    connection: Connection,
    activation: XdgActivationV1,
}

/// How far the attachment has got, on the UI thread.
enum Attachment {
    /// Not Wayland, or [`prepare`] hasn't run.
    Unused,
    /// Being made on the background executor; the windows to raise with
    /// their tokens once it is made (or has failed).
    Pending { waiting: Vec<(AnyWindowHandle, String)> },
    Ready(Attached),
    /// Making it failed (logged once, in [`settle`]).
    Failed(String),
}

impl Attachment {
    fn state(&self) -> AttachState {
        match self {
            Attachment::Unused => AttachState::Unused,
            Attachment::Pending { .. } => AttachState::Pending,
            Attachment::Ready(_) => AttachState::Ready,
            Attachment::Failed(_) => AttachState::Failed,
        }
    }
}

thread_local! {
    static ATTACHMENT: RefCell<Attachment> = const { RefCell::new(Attachment::Unused) };
}

/// Attaches to the `wl_display` at `display`: wraps it, does the registry
/// round trip (which waits on the compositor) and binds `xdg_activation_v1`.
///
/// # Safety
///
/// `display` must be the address of a live `wl_display` that outlives the
/// attachment.
unsafe fn attach(display: usize) -> anyhow::Result<Attached> {
    // SAFETY: the caller's; a foreign backend never closes the display.
    let backend = unsafe { Backend::from_foreign_display((display as *mut c_void).cast()) };
    let connection = Connection::from_backend(backend);
    let (globals, queue) = registry_queue_init::<State>(&connection)?;
    let activation: XdgActivationV1 = globals.bind(&queue.handle(), 1..=1, ())?;
    Ok(Attached { display, connection, activation })
}

/// `surface`'s proxy on our connection.
unsafe fn surface_proxy(a: &Attached, surface: *mut c_void) -> anyhow::Result<WlSurface> {
    // SAFETY: `surface` is the window's live `wl_surface` proxy.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }?;
    Ok(WlSurface::from_id(&a.connection, id)?)
}
