//! Raising a window when the user clicks one of its notifications, or
//! picks something another window shows.
//!
//! On Wayland a window may only take focus with an xdg-activation token the
//! compositor issued for a user action. GPUI's `activate_window` asks for a
//! token of its own, which compositors refuse when the window isn't focused
//! (GNOME then shows "Brindle is ready" instead). The notification server
//! hands us a valid token with the click (`ActivationToken`), so we use it
//! on GPUI's own surface: we attach to GPUI's `wl_display` with a separate
//! event queue, bind `xdg_activation_v1`, and activate the surface.
//! Anything else (X11, no token, a failure) falls back to `activate_window`.
//!
//! For a choice made in one Brindle window that brings up another, the
//! token is requested on behalf of the window the user is in, which has
//! focus ([`request_token`]). GNOME only honours such a request when it
//! names the serial of the latest key or button press, which GPUI
//! keeps to itself. So once a window opens ([`track_input`]) we keep our
//! own `wl_keyboard` and `wl_pointer` on the same connection: the
//! compositor sends every input event to all of them, with the same serial.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::Duration;

use gpui::{App, Window};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{self, XdgActivationTokenV1};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};

/// Brings `window` to the front, using `token` when there is one.
pub fn raise(window: &mut Window, token: Option<&str>) {
    if let Some(token) = token {
        match activate_with_token(window, token) {
            Ok(true) => {
                log::debug!("raised the window with the notification's activation token");
                return;
            }
            Ok(false) => log::debug!("not a Wayland window; raising it without the token"),
            Err(err) => log::warn!("raising the window with the notification's token failed: {err:#}"),
        }
    }
    window.activate_window();
}

/// Asks the compositor for an activation token on behalf of `window`, the
/// focused window the user just acted in. `None` off Wayland or on failure.
pub fn request_token(window: &Window) -> Option<String> {
    let result = wayland_pointers(window).and_then(|pointers| match pointers {
        // SAFETY: as in `activate_with_token`.
        Some((display, surface)) => unsafe { token_for(display, surface) }.map(Some),
        None => Ok(None),
    });
    match result {
        Ok(Some(token)) => {
            log::debug!("got an activation token from the focused window");
            Some(token)
        }
        Ok(None) => None,
        Err(err) => {
            log::debug!("no activation token from the focused window: {err:#}");
            None
        }
    }
}

/// Starts following input serials for [`request_token`], once a Wayland
/// window exists. Called for every window; only the first does anything.
pub fn track_input(window: &Window, cx: &mut App) {
    let Ok(Some((display, _))) = wayland_pointers(window) else { return };
    // SAFETY: `display` is GPUI's live `wl_display`.
    let fresh = unsafe { with_attached(display, |a| Ok(!std::mem::replace(&mut a.draining, true))) };
    match fresh {
        Ok(true) => {
            // Our queue fills with every key, button and pointer motion
            // event until it's dispatched; empty it now and then.
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    drain();
                }
            })
            .detach();
        }
        Ok(false) => {}
        Err(err) => log::debug!("not following input serials: {err:#}"),
    }
}

fn drain() {
    ATTACHED.with_borrow_mut(|attached| {
        if let Some(a) = attached
            && let Err(err) = a.queue.dispatch_pending(&mut a.state)
        {
            log::debug!("dispatching our Wayland queue failed: {err}");
        }
    });
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

#[derive(Default)]
struct State {
    /// The token the compositor issued for our last request.
    token: Option<String>,
    /// The serial of the latest key or button press: what the
    /// compositor checks a token request's serial against.
    serial: Option<u32>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event else { return };
        if caps.contains(wl_seat::Capability::Keyboard) {
            state.keyboard.get_or_insert_with(|| seat.get_keyboard(qh, ()));
        } else if let Some(keyboard) = state.keyboard.take()
            && keyboard.version() >= 3
        {
            keyboard.release();
        }
        if caps.contains(wl_seat::Capability::Pointer) {
            state.pointer.get_or_insert_with(|| seat.get_pointer(qh, ()));
        } else if let Some(pointer) = state.pointer.take()
            && pointer.version() >= 3
        {
            pointer.release();
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(state: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // Presses only: a release read along with its press isn't what the
        // compositor checks. The keymap's fd is closed by dropping the event.
        if let wl_keyboard::Event::Key { serial, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. } = event {
            state.serial = Some(serial);
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(this: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_pointer::Event::Button { serial, state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } = event {
            this.serial = Some(serial);
        }
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(_: &mut Self, _: &XdgActivationV1, _: xdg_activation_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

/// Our attachment to GPUI's display, made once: binding needs a registry
/// round trip, and the registry can't be destroyed again.
struct Attached {
    display: *mut c_void,
    connection: Connection,
    activation: XdgActivationV1,
    seat: Option<WlSeat>,
    queue: EventQueue<State>,
    state: State,
    /// Whether `track_input` has started draining `queue`.
    draining: bool,
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
            let mut queue = queue;
            let activation: XdgActivationV1 = globals.bind(&queue.handle(), 1..=1, ())?;
            let seat: Option<WlSeat> = globals.bind(&queue.handle(), 1..=5, ()).ok();
            let mut state = State::default();
            // Gets the seat's capabilities, creating our keyboard and pointer.
            queue.roundtrip(&mut state)?;
            *attached = Some(Attached { display, connection, activation, seat, queue, state, draining: false });
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

unsafe fn token_for(display: *mut c_void, surface: *mut c_void) -> anyhow::Result<String> {
    unsafe {
        with_attached(display, |a| {
            let surface = surface_proxy(a, surface)?;
            // Catch up with input up to now: the key or click that got us
            // here is the serial the compositor wants.
            a.queue.roundtrip(&mut a.state)?;
            let request = a.activation.get_activation_token(&a.queue.handle(), ());
            request.set_app_id("brindle".into());
            match (a.state.serial, &a.seat) {
                (Some(serial), Some(seat)) => request.set_serial(serial, seat),
                _ => log::debug!("no input serial for the activation token request"),
            }
            request.set_surface(&surface);
            request.commit();
            a.state.token = None;
            // The compositor answers `done` right away; a couple of round
            // trips is plenty.
            for _ in 0..3 {
                a.queue.roundtrip(&mut a.state)?;
                if a.state.token.is_some() {
                    break;
                }
            }
            request.destroy();
            a.state.token.take().ok_or_else(|| anyhow::anyhow!("the compositor sent no token"))
        })
    }
}
