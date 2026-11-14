//! The server pipes and the peer check: named pipe instances whose clients
//! must run as the server's user, at its integrity level or above.
//!
//! A server listens at a pipe name (`\\.\pipe\…`, from
//! `brindle_remote_protocol::server_dir`), which is what this module's
//! paths are. What differs from a listening socket:
//!
//! - **Instances:** each connection is its own instance of the pipe, and
//!   the instance a client connects to becomes that connection's handle.
//!   The listener always holds one instance for the next client: an accept
//!   creates the next one before it hands out the connected one, so the
//!   name is never without an instance of this server. A client that finds
//!   the one instance taken ([`connect`], between a connection and its
//!   accept) waits for the next.
//! - **The name:** the first instance is created with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a name someone already holds fails
//!   [`Listener::bind`] (`AddrInUse`; Windows reports error 5 or 231).
//!   Every instance gets the user-and-SYSTEM DACL with the no-write-up
//!   label ([`win::Security::for_pipe`]), rejects remote clients and is read
//!   as bytes. The name is free again when the listener and every
//!   connection it accepted are closed.
//! - **Readiness:** the listener keeps one overlapped `ConnectNamedPipe`
//!   in flight on the waiting instance, due a packet on its loop's port, and
//!   is readable once that has ended. So it accepts only while registered
//!   with a loop; clients can connect before that, and are accepted after.
//!   The streams it accepts belong to the same loop (their handle is
//!   associated with its port).
//! - **The peer check** comes before anything is read: the client's process
//!   (`GetNamedPipeClientProcessId`), then that process's token user and
//!   integrity. A client that runs as another user or below the server's
//!   integrity, or whose token can't be read, is closed unread.
//!   (Impersonating the client isn't possible before a read.)
//! - **Clients** check the server the same way before they write
//!   ([`win::check_server`]).

use std::io;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use brindle_remote_protocol::win::{self, Principal, Side};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    WaitNamedPipeW,
};
use windows::core::PCWSTR;

use super::conn::Stream;
use super::event_loop::{Port, Source, Watched, millis, os_error, win32};
use crate::server::event_loop::{Event, Interest, Token};

/// What every pipe name starts with.
const PIPE_PREFIX: &str = r"\\.\pipe\";

/// The most characters a pipe name has, prefix included.
const MAX_NAME: usize = 256;

/// Each instance's own buffers, each way.
const PIPE_BUFFER: u32 = 64 * 1024;

/// How long [`connect`] waits for a server whose instances are all taken.
const BUSY_LIMIT: Duration = Duration::from_secs(5);

const NO_ERROR: u32 = 0;

/// Who a server runs as, which its peers must match: the token's user, and
/// the integrity level its clients must at least have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity(Result<Principal, String>);

impl Identity {
    /// This process's own. If it can't be read, [`Listener::bind`] says so.
    pub fn own() -> Self {
        Self(win::own_principal().map_err(|error| error.to_string()))
    }

    /// Someone else's, for a server that runs, as far as its peer check is
    /// concerned, as another user.
    #[cfg(test)]
    pub fn other(self) -> Self {
        Self(self.0.map(|principal| Principal {
            sid: format!("{}-1", principal.sid),
            ..principal
        }))
    }
}

/// The name in `path`, checked: `\\.\pipe\` and a name without a backslash,
/// at most 256 characters in all.
fn pipe_name(path: &Path) -> io::Result<Vec<u16>> {
    let invalid = |problem: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a pipe name: {problem}", path.display()),
        )
    };
    let Some(name) = path
        .to_str()
        .and_then(|text| text.strip_prefix(PIPE_PREFIX))
    else {
        return Err(invalid(r"it doesn't start with \\.\pipe\"));
    };
    if name.is_empty() || name.contains(['\\', '\0']) {
        return Err(invalid("the name after the prefix is empty or has a backslash"));
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    if wide.len() - 1 > MAX_NAME {
        return Err(invalid("it is longer than 256 characters"));
    }
    Ok(wide)
}

/// Fails if a server can't listen at `path`, which must be a pipe name.
pub fn check_path(path: &Path) -> io::Result<()> {
    pipe_name(path).map(drop)
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// Connects to the server listening at the pipe name `path`, blocking, and
/// checks who runs it before anything is written: a server that runs as
/// another user or at another integrity level is `PermissionDenied`. With
/// no server there the error is `NotFound`.
pub fn connect(path: &Path) -> io::Result<Stream> {
    let name = pipe_name(path)?;
    let deadline = Instant::now() + BUSY_LIMIT;
    let handle = loop {
        // Overlapped, since the stream's reads and writes don't block. The
        // server may identify this client, and no more.
        // SAFETY: the name is NUL-terminated and outlives the call.
        let opened = unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match opened {
            // SAFETY: the handle is new and owned by nothing else.
            Ok(handle) => break unsafe { OwnedHandle::from_raw_handle(handle.0) },
            // Every instance has a client: the server makes the next one
            // when it accepts.
            Err(error) if win32(&error) == ERROR_PIPE_BUSY.0 => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("the server at {} accepts no connections", path.display()),
                    ));
                }
                // Returns when an instance is free, or failed: the next
                // open tells which.
                // SAFETY: as above.
                let _ = unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), millis(remaining).max(1)) };
            }
            Err(error) => return Err(os_error(win32(&error))),
        }
    };
    win::check_server(&handle)?;
    Ok(Stream::new(handle))
}

/// Creates an instance of the pipe `name` (NUL-terminated) for one client.
/// The `first` one fails with `AddrInUse` when the name is taken.
fn create_instance(name: &[u16], first: bool) -> io::Result<OwnedHandle> {
    let security = win::Security::for_pipe()?;
    let mut open = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: the name is NUL-terminated, and it and the security
    // attributes outlive the call.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            Some(security.attributes()),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        let code = error.raw_os_error().map(|code| code as u32);
        // Both mean that someone holds the name already: its creator's
        // DACL refuses this process, or allows no further instance.
        let taken = [Some(ERROR_ACCESS_DENIED.0), Some(ERROR_PIPE_BUSY.0)].contains(&code);
        let name = String::from_utf16_lossy(&name[..name.len() - 1]);
        return Err(if first && taken {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("the pipe name {name} is taken by another process ({error})"),
            )
        } else {
            io::Error::new(error.kind(), format!("can't listen on {name}: {error}"))
        });
    }
    // SAFETY: the handle is new and owned by nothing else.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

/// The pid of the process at the client end of a pipe instance.
fn client_pid(pipe: &OwnedHandle) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: the handle is open and `pid` is writable.
    unsafe { GetNamedPipeClientProcessId(raw(pipe), &mut pid) }
        .map_err(|error| os_error(win32(&error)))?;
    Ok(pid)
}

/// Accepts a client only when it runs as `own`'s user and not below its
/// integrity level. `client` is who it runs as, or why that can't be told,
/// which refuses it too. Every refusal is `PermissionDenied`.
fn check(own: &Principal, client: io::Result<Principal>) -> io::Result<()> {
    let refused = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
    let client = client.map_err(|error| {
        refused(format!(
            "refused a connection whose user can't be told: {error}"
        ))
    })?;
    win::peer_allowed(own, &client, Side::Server)
        .map_err(|refusal| refused(format!("refused a connection: {refusal}")))
}

/// What Windows is given for the connect in flight. Its packet names it by
/// the address of its `OVERLAPPED`, which is this block's address.
#[repr(C)]
struct Block {
    overlapped: OVERLAPPED,
    /// The reference to the listener's state (`Arc::into_raw`) the connect
    /// holds while its packet is due.
    owner: *const Shared,
}

/// How the waiting instance's `ConnectNamedPipe` stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connect {
    /// Not issued: no loop yet, or its loop is gone.
    Idle,
    /// In flight, due a packet.
    Pending,
    /// Ended with this error; with none, a client is connected.
    Done(u32),
}

struct State {
    /// The pipe's name, NUL-terminated.
    name: Vec<u16>,
    /// The instance the next client connects to, or has connected to and
    /// isn't accepted yet. Closed (`None`) once the owner is gone.
    instance: Option<OwnedHandle>,
    connect: Connect,
    /// Never moved, and freed with the state: a connect in flight holds a
    /// reference to the state. Reached only through raw pointers, since
    /// Windows writes to it meanwhile.
    block: NonNull<Block>,
    /// The port the instances are associated with, from the first
    /// registration on.
    port: Option<Arc<Port>>,
    /// A loop reports this listener.
    registered: bool,
}

// SAFETY: `block` is this value's own allocation, and nothing in it is tied
// to a thread.
unsafe impl Send for State {}

impl Drop for State {
    fn drop(&mut self) {
        // No connect is in flight: it would hold a reference to this.
        // SAFETY: allocated in `Listener::bind`, and freed once, here.
        drop(unsafe { Box::from_raw(self.block.as_ptr()) });
    }
}

impl State {
    fn overlapped(&self) -> *mut OVERLAPPED {
        // `Block` is `repr(C)` with the `OVERLAPPED` first.
        self.block.as_ptr().cast()
    }

    /// Issues the connect on the waiting instance when one is due: none in
    /// flight or ended, and a loop to take its packet.
    fn listen(&mut self, me: &Arc<Shared>) {
        if self.connect != Connect::Idle {
            return;
        }
        let Some(instance) = self.instance.as_ref().map(raw) else {
            return;
        };
        let Some(port) = self.port.as_ref().filter(|port| port.is_alive()) else {
            return;
        };
        let block = self.block.as_ptr();
        let overlapped = self.overlapped();
        // SAFETY: nothing is in flight, so the block is ours alone.
        unsafe {
            block.write(Block {
                overlapped: OVERLAPPED::default(),
                owner: Arc::into_raw(Arc::clone(me)),
            });
        }
        // Before the call: the packet may be taken as soon as the connect
        // is issued.
        port.expect(overlapped, connected);
        // SAFETY: the instance is open, and the block lives until the
        // connect has ended and its packet is taken.
        let error = match unsafe { ConnectNamedPipe(instance, Some(overlapped)) } {
            Ok(()) => NO_ERROR,
            Err(error) => win32(&error),
        };
        self.connect = if error == NO_ERROR || error == ERROR_IO_PENDING.0 {
            Connect::Pending
        } else {
            // It ended outright: no packet, so the reference comes back.
            port.forget(overlapped);
            // SAFETY: the reference made above, which nothing else took.
            unsafe {
                drop(Arc::from_raw((*block).owner));
                (*block).owner = ptr::null();
            }
            // A client was there already. One that has left again
            // (`ERROR_NO_DATA`) is a connection too, at its end.
            if error == ERROR_PIPE_CONNECTED.0 || error == ERROR_NO_DATA.0 {
                Connect::Done(NO_ERROR)
            } else {
                Connect::Done(error)
            }
        };
    }

    /// Cancels the connect in flight. Its packet still comes.
    fn cancel(&self) {
        if self.connect == Connect::Pending
            && let Some(instance) = self.instance.as_ref()
        {
            // SAFETY: the instance is open and the block alive.
            let _ = unsafe { CancelIoEx(raw(instance), Some(self.overlapped())) };
        }
    }
}

/// What a listener, the loop that reports it and its connect in flight
/// share.
struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Records the end of the connect a packet names: its
/// [`Completion`](super::event_loop::Completion).
///
/// # Safety
///
/// `overlapped` must come from the packet of a connect [`State::listen`]
/// expected with this function, and each packet must be handed in once.
unsafe fn connected(overlapped: *mut OVERLAPPED, _bytes: u32, error: u32) {
    let block = overlapped.cast::<Block>();
    // SAFETY: the connect's reference kept the state, and so the block,
    // alive until now, and nothing else touches the block meanwhile.
    let shared = unsafe { Arc::from_raw(mem::replace(&mut (*block).owner, ptr::null())) };
    let mut state = shared.lock();
    state.connect = match error {
        // Cancelled: the listener or its loop is going.
        error if error == ERROR_OPERATION_ABORTED.0 => Connect::Idle,
        error => Connect::Done(error),
    };
    drop(state);
    // With the listener gone, this may be the last reference.
}

impl Watched for Shared {
    fn attach(self: Arc<Self>, port: &Arc<Port>) -> io::Result<()> {
        let mut state = self.lock();
        if state.registered {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the source is already registered",
            ));
        }
        match &state.port {
            Some(bound) if Arc::ptr_eq(bound, port) => {}
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the source belongs to another event loop",
                ));
            }
            None => {
                if let Some(instance) = state.instance.as_ref() {
                    port.associate(raw(instance))?;
                }
                state.port = Some(Arc::clone(port));
                port.adopt(&self);
            }
        }
        state.registered = true;
        state.listen(&self);
        Ok(())
    }

    fn detach(&self) {
        // The connect stays in flight: its packet is taken with the port's
        // others, and a client that connects meanwhile is accepted later.
        self.lock().registered = false;
    }

    fn cancel_packets(&self) {
        self.lock().cancel();
    }

    fn readiness(
        self: Arc<Self>,
        token: Token,
        interest: Interest,
        _polling: &mut bool,
    ) -> Option<Event> {
        let mut state = self.lock();
        state.listen(&self);
        let ready = matches!(state.connect, Connect::Done(_)) && state.instance.is_some();
        (interest.readable && ready).then_some(Event {
            token,
            readable: true,
            writable: false,
            closed: false,
        })
    }
}

/// A non-blocking listener: the pipe name's waiting instance.
pub struct Listener {
    shared: Arc<Shared>,
    identity: Principal,
}

impl Listener {
    /// Listens at the pipe name `path` for peers that run as `identity`.
    /// A name that someone holds already is `AddrInUse`.
    pub fn bind(path: &Path, identity: Identity) -> io::Result<Self> {
        let name = pipe_name(path)?;
        let identity = identity.0.map_err(|error| {
            io::Error::other(format!("can't tell who this server runs as: {error}"))
        })?;
        let instance = create_instance(&name, true)?;
        let block = Box::new(Block {
            overlapped: OVERLAPPED::default(),
            owner: ptr::null(),
        });
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    name,
                    instance: Some(instance),
                    connect: Connect::Idle,
                    block: NonNull::from(Box::leak(block)),
                    port: None,
                    registered: false,
                }),
            }),
            identity,
        })
    }

    /// What an event loop waits on: readable when a connection is waiting.
    pub fn source(&self) -> Source {
        Source::of(Arc::clone(&self.shared) as Arc<dyn Watched>)
    }

    /// Accepts the next connection waiting, as the core and the holders
    /// do. `Ok(None)` means none is waiting (always, for a listener no loop
    /// has registered). A peer that runs as another user or below this
    /// server's integrity level (or who can't be told) is closed before
    /// anything is read from it, and comes back as a `PermissionDenied`
    /// error for the caller to log before it calls again.
    pub fn accept(&self) -> io::Result<Option<Stream>> {
        let mut pumped = false;
        let (connected, port, error) = loop {
            let mut state = self.shared.lock();
            state.listen(&self.shared);
            match state.connect {
                Connect::Idle => return Ok(None),
                Connect::Pending => {
                    // Its packet may be waiting for the loop's next wait.
                    let Some(port) = state.port.clone().filter(|_| !pumped) else {
                        return Ok(None);
                    };
                    drop(state);
                    pumped = true;
                    port.pump(Duration::ZERO)?;
                }
                Connect::Done(error) => {
                    // The next client's instance comes first, so that the
                    // name always has one. If it can't be made, the
                    // connection waits for the next accept.
                    let next = create_instance(&state.name, false)?;
                    if let Some(port) = &state.port {
                        port.associate(raw(&next))?;
                    }
                    let connected = state.instance.replace(next);
                    state.connect = Connect::Idle;
                    state.listen(&self.shared);
                    let Some(connected) = connected else {
                        return Ok(None);
                    };
                    break (connected, state.port.clone(), error);
                }
            }
        };
        if error != NO_ERROR {
            return Err(io::Error::new(
                os_error(error).kind(),
                format!("a client's connection failed: {}", os_error(error)),
            ));
        }
        // Dropping `connected` closes the instance, unread.
        check(
            &self.identity,
            client_pid(&connected).and_then(win::principal_of),
        )?;
        Ok(Some(match port {
            Some(port) => Stream::on(connected, &port),
            None => Stream::new(connected),
        }))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        // A connect in flight keeps the state until its packet is taken.
        state.cancel();
        // Closing the instance gives up the name, and a client that
        // connected and wasn't accepted reads the end.
        state.instance = None;
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::Weak;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    use brindle_remote_protocol::Frame;
    use brindle_remote_protocol::win::{Dacl, Integrity};

    use super::super::conn::tests::{check as check_pattern, pattern, send_pattern};
    use super::*;
    use crate::server::conn::{self, Conn};
    use crate::server::event_loop::{EventLoop, Loop};

    const SOON: Option<Duration> = Some(Duration::from_secs(10));

    /// A pipe name no other test or process uses.
    fn name(tag: &str) -> std::path::PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\brindle-test-listen-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
        .into()
    }

    /// A listener at a fresh name, registered with a new loop as token 0.
    fn listening(tag: &str, identity: Identity) -> (Loop, Listener, std::path::PathBuf) {
        let path = name(tag);
        let mut ev = Loop::new().unwrap();
        let listener = Listener::bind(&path, identity).unwrap();
        ev.register(&listener.source(), 0, Interest::READABLE)
            .unwrap();
        (ev, listener, path)
    }

    /// Waits for the listener to be readable, and accepts.
    fn accept(ev: &mut Loop, listener: &Listener) -> io::Result<Option<Stream>> {
        let mut events = Vec::new();
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(
            events,
            [Event {
                token: 0,
                readable: true,
                writable: false,
                closed: false
            }]
        );
        listener.accept()
    }

    fn principal(sid: &str, integrity: Integrity) -> Principal {
        Principal {
            sid: sid.into(),
            integrity,
        }
    }

    #[test]
    fn paths_are_pipe_names() {
        check_path(Path::new(r"\\.\pipe\brindle-server-0123456789abcdef-core")).unwrap();
        for bad in [
            r"C:\Users\me\run\core.sock",
            r"\\.\pipe\",
            r"\\.\pipe\a\b",
            r"\\server\pipe\name",
        ] {
            let error = check_path(Path::new(bad)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad}");
        }
        let longest = format!(r"\\.\pipe\{}", "n".repeat(MAX_NAME - PIPE_PREFIX.len()));
        check_path(Path::new(&longest)).unwrap();
        assert!(check_path(Path::new(&format!("{longest}n"))).is_err());
        assert!(connect(Path::new(r"C:\nowhere\core.sock")).is_err());
        // No server at a good name is `NotFound`, which callers read as
        // "not running".
        let error = connect(&name("nobody")).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
    }

    #[test]
    fn the_client_must_be_the_same_user_at_no_lower_integrity() {
        let own = principal("S-1-5-21-1-2-3-1001", Integrity::MEDIUM);
        check(&own, Ok(own.clone())).unwrap();
        // A client above the server is the server's to accept; the client
        // itself refuses a server at another level.
        check(&own, Ok(principal(&own.sid, Integrity::HIGH))).unwrap();

        let other_user = check(&own, Ok(principal("S-1-5-21-1-2-3-1002", Integrity::MEDIUM)))
            .unwrap_err();
        assert_eq!(other_user.kind(), io::ErrorKind::PermissionDenied);
        assert!(other_user.to_string().contains("another user"), "{other_user}");

        let lower = check(&own, Ok(principal(&own.sid, Integrity::LOW))).unwrap_err();
        assert_eq!(lower.kind(), io::ErrorKind::PermissionDenied);
        assert!(lower.to_string().contains("low integrity"), "{lower}");
        let elevated = principal(&own.sid, Integrity::HIGH);
        assert!(check(&elevated, Ok(own.clone())).is_err());

        // A token that can't be opened is refused, whatever the reason.
        let unreadable = check(&own, Err(io::Error::from_raw_os_error(5))).unwrap_err();
        assert_eq!(unreadable.kind(), io::ErrorKind::PermissionDenied);
        assert!(unreadable.to_string().contains("can't be told"), "{unreadable}");
        let gone = check(&own, Err(io::ErrorKind::NotFound.into())).unwrap_err();
        assert_eq!(gone.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn a_same_user_client_connects_through_the_loop() {
        let (mut ev, listener, path) = listening("accept", Identity::own());
        let mut events = Vec::new();
        ev.wait(&mut events, Some(Duration::from_millis(30)))
            .unwrap();
        assert!(events.is_empty(), "nobody has connected");
        assert!(listener.accept().unwrap().is_none());

        let mut client = connect(&path).unwrap();
        let mut server = accept(&mut ev, &listener).unwrap().unwrap();
        // One connection is one event.
        ev.wait(&mut events, Some(Duration::from_millis(30)))
            .unwrap();
        assert!(events.is_empty());
        assert!(listener.accept().unwrap().is_none());

        // The accepted stream works on the listener's loop, both ways.
        server.set_nonblocking().unwrap();
        ev.register(&server.source(), 1, Interest::READABLE)
            .unwrap();
        client.write_all(b"hello").unwrap();
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].token == 1 && events[0].readable);
        let mut buffer = [0u8; 16];
        assert_eq!(server.read(&mut buffer).unwrap(), 5);
        assert_eq!(&buffer[..5], b"hello");
        assert_eq!(server.write(b"welcome").unwrap(), 7);
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        client.read_exact(&mut buffer[..7]).unwrap();
        assert_eq!(&buffer[..7], b"welcome");

        // And only there.
        let mut second = Loop::new().unwrap();
        ev.deregister(1).unwrap();
        assert!(
            second
                .register(&server.source(), 1, Interest::READABLE)
                .is_err()
        );
    }

    #[test]
    fn an_accept_without_a_wait_takes_the_connection() {
        // As the core's tests do: the client connects, and the server
        // accepts without waiting on its loop first.
        let (_ev, listener, path) = listening("direct", Identity::own());
        let mut client = connect(&path).unwrap();
        client.write_all(&[2, 0, 0, 0, 2, b'{', b'}']).unwrap();
        let mut server = listener.accept().unwrap().expect("a connection");
        let mut message = [0u8; 7];
        server.read_exact(&mut message).unwrap();
        assert_eq!(message[0], 2);

        // A client that connected before the listener was registered is
        // accepted once it is.
        let path = name("early");
        let listener = Listener::bind(&path, Identity::own()).unwrap();
        let _client = connect(&path).unwrap();
        assert!(listener.accept().unwrap().is_none(), "no loop yet");
        let mut ev = Loop::new().unwrap();
        ev.register(&listener.source(), 0, Interest::READABLE)
            .unwrap();
        assert!(accept(&mut ev, &listener).unwrap().is_some());
    }

    #[test]
    fn a_client_of_another_user_or_lower_integrity_is_closed_unread() {
        let own = win::own_principal().unwrap();
        let above = Identity(Ok(Principal {
            integrity: Integrity(own.integrity.0 + 0x1000),
            ..own.clone()
        }));
        for (tag, identity) in [
            ("foreign", Identity::own().other()),
            // As if this server ran one integrity level higher.
            ("above", above),
        ] {
            let (mut ev, listener, path) = listening(tag, identity);
            let mut client = connect(&path).unwrap();
            client.write_all(&[2, 0, 0, 0, 2, b'{', b'}']).unwrap();
            let error = accept(&mut ev, &listener).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{tag}");
            assert!(error.to_string().contains("refused"), "{error}");
            // The client reads the end, and nothing else.
            client
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut byte = [0u8; 1];
            assert_eq!(client.read(&mut byte).unwrap(), 0, "{tag}");
            assert!(listener.accept().unwrap().is_none());

            // The listener goes on: the next client gets the same answer.
            let _client = connect(&path).unwrap();
            let error = accept(&mut ev, &listener).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{tag}");
        }
        // A server whose own user can't be read doesn't listen.
        let unknown = Identity(Err("no token".into()));
        assert!(Listener::bind(&name("unknown"), unknown).is_err());
    }

    #[test]
    fn a_taken_name_fails_the_bind() {
        let path = name("taken");
        let first = Listener::bind(&path, Identity::own()).unwrap();
        let error = Listener::bind(&path, Identity::own()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse, "{error}");
        assert!(error.to_string().contains("taken"), "{error}");

        // Also while the listener's only instances are connections.
        let mut ev = Loop::new().unwrap();
        ev.register(&first.source(), 0, Interest::READABLE).unwrap();
        let _client = connect(&path).unwrap();
        let server = accept(&mut ev, &first).unwrap().unwrap();
        drop(first);
        let error = Listener::bind(&path, Identity::own()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse, "{error}");
        // The name is free once they are closed too.
        drop(server);
        drop(Listener::bind(&path, Identity::own()).unwrap());

        // A pipe someone made with room for one instance only (error 231,
        // where the listener's own pipes give 5).
        let path = name("squatted");
        let wide = pipe_name(&path).unwrap();
        // SAFETY: the name is NUL-terminated and outlives the call.
        let squatter = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE,
                1,
                0,
                0,
                0,
                None,
            )
        };
        assert_ne!(squatter, INVALID_HANDLE_VALUE);
        // SAFETY: the handle is new and owned by nothing else.
        let squatter = unsafe { OwnedHandle::from_raw_handle(squatter.0) };
        let error = Listener::bind(&path, Identity::own()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse, "{error}");
        // A client refuses nothing here (the squatter is this process),
        // but never reaches a listener.
        drop(squatter);
        drop(Listener::bind(&path, Identity::own()).unwrap());
    }

    #[test]
    fn instances_are_for_this_user_and_system_only() {
        let (mut ev, listener, path) = listening("dacl", Identity::own());
        let own = win::own_sid().unwrap();
        let dacl_of = |listener: &Listener| {
            let state = listener.shared.lock();
            Dacl::of_handle(state.instance.as_ref().unwrap().as_raw_handle()).unwrap()
        };
        assert!(dacl_of(&listener).allows_only(&[&own, "S-1-5-18"]));
        // The instances that follow the first one, too.
        let _client = connect(&path).unwrap();
        let _server = accept(&mut ev, &listener).unwrap().unwrap();
        assert!(dacl_of(&listener).allows_only(&[&own, "S-1-5-18"]));
    }

    #[test]
    fn clients_wait_for_the_next_instance() {
        let (mut ev, listener, path) = listening("busy", Identity::own());
        // The first takes the one instance. The others find the pipe busy
        // until an accept makes the next.
        let first = connect(&path).unwrap();
        let waiting: Vec<_> = (0..3)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || connect(&path).unwrap())
            })
            .collect();
        thread::sleep(Duration::from_millis(50));
        let mut servers = Vec::new();
        for _ in 0..4 {
            servers.push(accept(&mut ev, &listener).unwrap().unwrap());
        }
        let clients: Vec<Stream> = waiting
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        drop((first, clients, servers));
    }

    #[test]
    fn floods_arrive_in_order_through_accepted_connections() {
        const TOTAL: u64 = 16 * 1024 * 1024;
        const CLIENTS: usize = 3;
        let (mut ev, listener, path) = listening("flood", Identity::own());
        let writers: Vec<_> = (0..CLIENTS)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || {
                    let mut client = connect(&path).unwrap();
                    send_pattern(&mut client, &pattern(), TOTAL);
                    // Dropped here: everything was taken by the pipe.
                })
            })
            .collect();
        let pattern = pattern();
        let mut streams: Vec<(Stream, u64, bool)> = Vec::new();
        let mut events = Vec::new();
        let mut buffer = vec![0u8; 48 * 1024];
        while streams.len() < CLIENTS || streams.iter().any(|(_, _, ended)| !ended) {
            ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
            assert!(!events.is_empty(), "the flood stalled");
            for event in events.clone() {
                if event.token == 0 {
                    while let Some(stream) = listener.accept().unwrap() {
                        stream.set_nonblocking().unwrap();
                        ev.register(&stream.source(), streams.len() + 1, Interest::READABLE)
                            .unwrap();
                        streams.push((stream, 0, false));
                    }
                    continue;
                }
                let (stream, received, ended) = &mut streams[event.token - 1];
                // A fair share per turn, as a handler takes.
                for _ in 0..8 {
                    match stream.read(&mut buffer) {
                        Ok(0) => {
                            *ended = true;
                            ev.deregister(event.token).unwrap();
                            break;
                        }
                        Ok(count) => {
                            check_pattern(&pattern, *received, &buffer[..count]);
                            *received += count as u64;
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) => panic!("read failed at byte {received}: {error}"),
                    }
                }
            }
        }
        for (_, received, _) in &streams {
            assert_eq!(*received, TOTAL);
        }
        for writer in writers {
            writer.join().unwrap();
        }
    }

    #[test]
    fn a_graceful_close_delivers_the_queue_and_then_the_end() {
        // The server's side of an accepted connection, registered with its
        // loop, says its last words and closes; the loop isn't waiting
        // meanwhile.
        let (mut ev, listener, path) = listening("close", Identity::own());
        let client = connect(&path).unwrap();
        let server = accept(&mut ev, &listener).unwrap().unwrap();
        let mut server = Conn::new(server).unwrap();
        ev.register(&server.source(), 1, Interest::READABLE)
            .unwrap();

        // More than the pipe holds, and what the client sent is never read.
        let frames = [
            Frame::new(9, vec![3; 400_000]).unwrap(),
            Frame::new(10, b"goodbye".to_vec()).unwrap(),
        ];
        let mut client = Conn::new(client).unwrap();
        client.queue(&Frame::new(1, b"never read".to_vec()).unwrap());
        client.flush().unwrap();
        for frame in &frames {
            server.queue(frame);
        }
        let reader = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut got = Vec::new();
            loop {
                // Slowly: the close has to wait for this side.
                thread::sleep(Duration::from_millis(1));
                let eof = client.read_available().unwrap();
                while let Some(frame) = client.next_frame().unwrap() {
                    got.push(frame);
                }
                if eof {
                    return got;
                }
                assert!(Instant::now() < deadline, "no end after {} frames", got.len());
                conn::wait(&[&client], Duration::from_millis(100));
            }
        });
        server.close_for(Duration::from_secs(10)).unwrap();
        // Everything had left by the time the close returned.
        drop(server);
        assert_eq!(reader.join().unwrap(), frames);
    }

    #[test]
    fn a_refusal_flushed_before_the_drop_arrives_whole() {
        // What the core does with a client it turns away: queue the error,
        // flush for a moment, drop the connection.
        let (mut ev, listener, path) = listening("refusal", Identity::own());
        let client = connect(&path).unwrap();
        let mut server = Conn::new(accept(&mut ev, &listener).unwrap().unwrap()).unwrap();
        ev.register(&server.source(), 1, Interest::READABLE)
            .unwrap();
        let frame = Frame::new(4, vec![8; 300_000]).unwrap();
        server.queue(&frame);
        let reader = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            let mut client = client;
            client
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut got = Vec::new();
            client.read_to_end(&mut got).unwrap();
            got.len()
        });
        server.flush_for(Duration::from_secs(10)).unwrap();
        drop(server);
        assert_eq!(reader.join().unwrap(), 5 + 300_000);
    }

    /// Has `ev` take what has completed until `done`.
    fn turn_until(ev: &mut Loop, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            ev.wait(&mut events, Some(Duration::ZERO)).unwrap();
            thread::yield_now();
        }
    }

    #[test]
    fn closing_with_a_connect_in_flight_frees_the_state_after_its_packet() {
        let mut ev = Loop::new().unwrap();
        for round in 0..300 {
            let path = name("drop");
            let listener = Listener::bind(&path, Identity::own()).unwrap();
            let state: Weak<Shared> = Arc::downgrade(&listener.shared);
            ev.register(&listener.source(), round, Interest::READABLE)
                .unwrap();
            let mut client = None;
            match round % 4 {
                // The owner goes first, then the registration.
                0 => {
                    drop(listener);
                    ev.deregister(round).unwrap();
                }
                1 => {
                    ev.deregister(round).unwrap();
                    drop(listener);
                }
                // A client connects just before, and is never accepted.
                2 => {
                    client = Some(connect(&path).unwrap());
                    drop(listener);
                    ev.deregister(round).unwrap();
                }
                // One connects from another thread while the owner goes.
                _ => {
                    let connecting = thread::spawn(move || connect(&path).ok());
                    thread::yield_now();
                    drop(listener);
                    ev.deregister(round).unwrap();
                    client = connecting.join().unwrap();
                }
            }
            // The connect's packet frees the state when it is taken.
            turn_until(&mut ev, || state.upgrade().is_none());
            // A client that got in reads the end.
            if let Some(mut client) = client {
                client
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut byte = [0u8; 1];
                assert_eq!(client.read(&mut byte).unwrap(), 0, "round {round}");
            }
        }
    }

    #[test]
    fn dropping_the_loop_cancels_a_listeners_connect() {
        for round in 0..50 {
            let (ev, listener, path) = listening("loop", Identity::own());
            let state: Weak<Shared> = Arc::downgrade(&listener.shared);
            if round % 2 == 0 {
                drop(listener);
                assert!(state.upgrade().is_some(), "its packet is untaken");
                drop(ev);
                assert!(state.upgrade().is_none(), "drained by the drop");
            } else {
                drop(ev);
                // Without its loop the listener accepts nothing, and still
                // holds the name.
                assert!(listener.accept().unwrap().is_none());
                assert!(Listener::bind(&path, Identity::own()).is_err());
                drop(listener);
                assert!(state.upgrade().is_none());
                drop(Listener::bind(&path, Identity::own()).unwrap());
            }
        }
    }
}
