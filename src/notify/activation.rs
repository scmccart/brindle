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

use std::cell::RefCell;
use std::ffi::c_void;

use gpui::Window;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};

/// Brings `window` to the front, using `token` when there is one.
pub fn raise(window: &mut Window, token: Option<&str>) {
    if let Some(token) = token {
        match activate_with_token(window, token) {
            Ok(true) => {
                log::debug!("raised the window with an activation token");
                return;
            }
            Ok(false) => log::debug!("not a Wayland window; raising it without the token"),
            Err(err) => log::warn!("raising the window with an activation token failed: {err:#}"),
        }
    }
    window.activate_window();
}

/// GPUI's `wl_display` and the window's `wl_surface`, on Wayland.
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
fn activate_with_token(window: &Window, token: &str) -> anyhow::Result<bool> {
    let Some((display, surface)) = wayland_pointers(window)? else { return Ok(false) };
    // SAFETY: both pointers come from GPUI's live window, which outlives
    // this call (we're inside its update), and are only used here.
    unsafe { activate(display, surface, token)? };
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
/// round trip, and the registry can't be destroyed again.
struct Attached {
    display: *mut c_void,
    connection: Connection,
    activation: XdgActivationV1,
}

thread_local! {
    static ATTACHED: RefCell<Option<Attached>> = const { RefCell::new(None) };
}

/// Runs `f` with our attachment to `display`, making it first if needed.
unsafe fn with_attached<R>(display: *mut c_void, f: impl FnOnce(&mut Attached) -> anyhow::Result<R>) -> anyhow::Result<R> {
    ATTACHED.with_borrow_mut(|attached| {
        if attached.as_ref().is_none_or(|a| a.display != display) {
            // SAFETY: `display` is GPUI's live `wl_display`; a foreign
            // backend never closes it.
            let backend = unsafe { Backend::from_foreign_display(display.cast()) };
            let connection = Connection::from_backend(backend);
            let (globals, queue) = registry_queue_init::<State>(&connection)?;
            let activation: XdgActivationV1 = globals.bind(&queue.handle(), 1..=1, ())?;
            *attached = Some(Attached { display, connection, activation });
        }
        f(attached.as_mut().expect("attached above"))
    })
}

/// `surface`'s proxy on our connection.
unsafe fn surface_proxy(a: &Attached, surface: *mut c_void) -> anyhow::Result<WlSurface> {
    // SAFETY: `surface` is the window's live `wl_surface` proxy.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }?;
    Ok(WlSurface::from_id(&a.connection, id)?)
}

unsafe fn activate(display: *mut c_void, surface: *mut c_void, token: &str) -> anyhow::Result<()> {
    unsafe {
        with_attached(display, |a| {
            let surface = surface_proxy(a, surface)?;
            a.activation.activate(token.to_string(), &surface);
            a.connection.flush()?;
            Ok(())
        })
    }
}
