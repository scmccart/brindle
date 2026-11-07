//! The single-threaded event loop the core and the holders run on.
//!
//! [`EventLoop`] is the small platform seam: register, modify and deregister
//! a source, wait with a timeout, and be woken from another thread or a
//! signal through a [`Waker`]. Linux uses [`PollLoop`], over `poll(2)`;
//! Windows (phase 3) will provide IOCP behind the same calls.
//!
//! Readiness is level-triggered: a source that is still readable is reported
//! again on the next wait, so a handler may stop after a fair share of work.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

/// Names a registered source in the events a wait returns.
pub type Token = usize;

/// What a source is waited for. With neither, it stays registered but is
/// never reported (not even for hang-ups), which is how a reader pauses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Interest {
    pub readable: bool,
    pub writable: bool,
}

impl Interest {
    #[allow(dead_code, reason = "for the core (task group 4)")]
    pub const NONE: Self = Self {
        readable: false,
        writable: false,
    };
    pub const READABLE: Self = Self {
        readable: true,
        writable: false,
    };

    pub fn is_none(self) -> bool {
        !self.readable && !self.writable
    }
}

/// One ready source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub token: Token,
    pub readable: bool,
    pub writable: bool,
    /// The peer hung up or the source failed. A read then returns EOF or
    /// the error, so handlers treat it as readable.
    pub closed: bool,
}

pub trait EventLoop {
    /// Starts reporting `fd` as `token`. Tokens must be unique.
    fn register(&mut self, fd: RawFd, token: Token, interest: Interest) -> io::Result<()>;
    fn modify(&mut self, token: Token, interest: Interest) -> io::Result<()>;
    /// Stops reporting `token`. The caller still owns and closes the fd.
    fn deregister(&mut self, token: Token) -> io::Result<()>;
    /// Waits until a source is ready, the loop is woken, or `timeout`
    /// passes (`None` waits forever). Fills `events` (cleared first) and
    /// returns whether a [`Waker`] fired.
    fn wait(&mut self, events: &mut Vec<Event>, timeout: Option<Duration>) -> io::Result<bool>;
    fn waker(&self) -> Waker;
}

/// Wakes a loop's wait from another thread or from a signal handler.
#[derive(Debug, Clone)]
pub struct Waker {
    fd: Arc<OwnedFd>,
}

impl Waker {
    #[allow(dead_code, reason = "for the core's worker threads (task group 4)")]
    pub fn wake(&self) -> io::Result<()> {
        write_wake_byte(self.fd.as_raw_fd())
    }

    /// Wakes the loop whenever this process receives `signal`. The handler
    /// only writes a byte; the loop then does the work (for `SIGCHLD`,
    /// reaping). One waker serves every signal registered this way, and a
    /// later call replaces it.
    pub fn wake_on_signal(&self, signal: libc::c_int) -> io::Result<()> {
        // A duplicate that is never closed, so the handler's fd can't be
        // reused for something else while a signal is in flight.
        let fd = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        SIGNAL_WAKE_FD.store(fd, Ordering::Release);
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = wake_from_signal as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

static SIGNAL_WAKE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn wake_from_signal(_signal: libc::c_int) {
    let fd = SIGNAL_WAKE_FD.load(Ordering::Acquire);
    if fd >= 0 {
        // write(2) is async-signal-safe; keep errno as the interrupted code
        // left it.
        unsafe {
            let errno = *libc::__errno_location();
            let byte = 1u8;
            libc::write(fd, (&byte as *const u8).cast(), 1);
            *libc::__errno_location() = errno;
        }
    }
}

fn write_wake_byte(fd: RawFd) -> io::Result<()> {
    let byte = 1u8;
    let written = unsafe { libc::write(fd, (&byte as *const u8).cast(), 1) };
    if written == -1 {
        let error = io::Error::last_os_error();
        // A full pipe already holds a wake-up.
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
    }
    Ok(())
}

/// A non-blocking, close-on-exec pipe: (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

struct Entry {
    token: Token,
    fd: RawFd,
    interest: Interest,
}

/// [`EventLoop`] over `poll(2)`, with a self-pipe for wake-ups.
pub struct PollLoop {
    entries: Vec<Entry>,
    wake_read: OwnedFd,
    waker: Waker,
    pollfds: Vec<libc::pollfd>,
}

impl PollLoop {
    pub fn new() -> io::Result<Self> {
        let (wake_read, wake_write) = pipe()?;
        Ok(Self {
            entries: Vec::new(),
            wake_read,
            waker: Waker {
                fd: Arc::new(wake_write),
            },
            pollfds: Vec::new(),
        })
    }

    fn entry(&mut self, token: Token) -> io::Result<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.token == token)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no token {token}")))
    }

    fn drain_wakes(&self) {
        let mut buffer = [0u8; 64];
        loop {
            let read = unsafe {
                libc::read(
                    self.wake_read.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if read <= 0 {
                break;
            }
        }
    }
}

fn poll_events(interest: Interest) -> libc::c_short {
    let mut events = 0;
    if interest.readable {
        events |= libc::POLLIN;
    }
    if interest.writable {
        events |= libc::POLLOUT;
    }
    events
}

impl EventLoop for PollLoop {
    fn register(&mut self, fd: RawFd, token: Token, interest: Interest) -> io::Result<()> {
        if self.entries.iter().any(|entry| entry.token == token) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("token {token} is already registered"),
            ));
        }
        self.entries.push(Entry {
            token,
            fd,
            interest,
        });
        Ok(())
    }

    fn modify(&mut self, token: Token, interest: Interest) -> io::Result<()> {
        self.entry(token)?.interest = interest;
        Ok(())
    }

    fn deregister(&mut self, token: Token) -> io::Result<()> {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.token != token);
        if self.entries.len() == before {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no token {token}"),
            ));
        }
        Ok(())
    }

    fn wait(&mut self, events: &mut Vec<Event>, timeout: Option<Duration>) -> io::Result<bool> {
        events.clear();
        self.pollfds.clear();
        self.pollfds.push(libc::pollfd {
            fd: self.wake_read.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        for entry in &self.entries {
            self.pollfds.push(libc::pollfd {
                // poll(2) skips negative fds, so a paused source can't
                // report a hang-up over and over.
                fd: if entry.interest.is_none() {
                    -1
                } else {
                    entry.fd
                },
                events: poll_events(entry.interest),
                revents: 0,
            });
        }
        let timeout_ms = match timeout {
            // Round up, so a short timeout doesn't spin at 0 ms.
            Some(timeout) => timeout
                .as_nanos()
                .div_ceil(1_000_000)
                .min(libc::c_int::MAX as u128) as libc::c_int,
            None => -1,
        };
        let ready = unsafe {
            libc::poll(
                self.pollfds.as_mut_ptr(),
                self.pollfds.len() as libc::nfds_t,
                timeout_ms,
            )
        };
        if ready == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(error);
        }
        let woken = self.pollfds[0].revents != 0;
        if woken {
            self.drain_wakes();
        }
        for (pollfd, entry) in self.pollfds[1..].iter().zip(&self.entries) {
            let revents = pollfd.revents;
            if revents == 0 {
                continue;
            }
            events.push(Event {
                token: entry.token,
                readable: revents & libc::POLLIN != 0,
                writable: revents & libc::POLLOUT != 0,
                closed: revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0,
            });
        }
        Ok(woken)
    }

    fn waker(&self) -> Waker {
        self.waker.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn write_byte(fd: &OwnedFd) {
        assert_eq!(
            unsafe { libc::write(fd.as_raw_fd(), b"x".as_ptr().cast(), 1) },
            1
        );
    }

    #[test]
    fn reports_registered_sources_by_token() {
        let mut ev = PollLoop::new().unwrap();
        let (a_read, a_write) = pipe().unwrap();
        let (b_read, _b_write) = pipe().unwrap();
        ev.register(a_read.as_raw_fd(), 7, Interest::READABLE)
            .unwrap();
        ev.register(b_read.as_raw_fd(), 8, Interest::READABLE)
            .unwrap();
        assert!(
            ev.register(b_read.as_raw_fd(), 8, Interest::READABLE)
                .is_err()
        );

        let mut events = Vec::new();
        let start = Instant::now();
        assert!(
            !ev.wait(&mut events, Some(Duration::from_millis(30)))
                .unwrap()
        );
        assert!(events.is_empty());
        assert!(start.elapsed() >= Duration::from_millis(25));

        write_byte(&a_write);
        assert!(!ev.wait(&mut events, Some(Duration::from_secs(5))).unwrap());
        assert_eq!(
            events,
            [Event {
                token: 7,
                readable: true,
                writable: false,
                closed: false
            }]
        );
        // Level-triggered: still readable until read.
        ev.wait(&mut events, Some(Duration::from_secs(5))).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn modify_pauses_and_writable_reports() {
        let mut ev = PollLoop::new().unwrap();
        let (read, write) = pipe().unwrap();
        ev.register(read.as_raw_fd(), 1, Interest::READABLE)
            .unwrap();
        ev.register(write.as_raw_fd(), 2, Interest::NONE).unwrap();
        write_byte(&write);
        drop(write);
        ev.deregister(2).unwrap();
        assert!(ev.deregister(2).is_err());

        let mut events = Vec::new();
        ev.modify(1, Interest::NONE).unwrap();
        ev.wait(&mut events, Some(Duration::from_millis(20)))
            .unwrap();
        assert!(
            events.is_empty(),
            "a paused source reports nothing, not even its hang-up"
        );
        ev.modify(1, Interest::READABLE).unwrap();
        ev.wait(&mut events, Some(Duration::from_secs(5))).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].readable && events[0].closed);

        let (_read, write) = pipe().unwrap();
        ev.register(
            write.as_raw_fd(),
            3,
            Interest {
                readable: false,
                writable: true,
            },
        )
        .unwrap();
        ev.deregister(1).unwrap();
        ev.wait(&mut events, Some(Duration::from_secs(5))).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].token == 3 && events[0].writable);
        assert!(ev.modify(1, Interest::NONE).is_err());
    }

    #[test]
    fn a_waker_wakes_a_wait_from_another_thread() {
        let mut ev = PollLoop::new().unwrap();
        let waker = ev.waker();
        let start = Instant::now();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            waker.wake().unwrap();
        });
        let mut events = Vec::new();
        assert!(ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap());
        assert!(start.elapsed() < Duration::from_secs(20));
        thread.join().unwrap();
        // The wake-up was consumed.
        assert!(
            !ev.wait(&mut events, Some(Duration::from_millis(10)))
                .unwrap()
        );

        // Many wakes before a wait are one wake-up, and never block.
        let waker = ev.waker();
        for _ in 0..100_000 {
            waker.wake().unwrap();
        }
        assert!(ev.wait(&mut events, Some(Duration::from_secs(5))).unwrap());
    }

    #[test]
    fn a_waker_wakes_a_wait_from_a_signal() {
        let mut ev = PollLoop::new().unwrap();
        ev.waker().wake_on_signal(libc::SIGUSR2).unwrap();
        let thread = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            unsafe { libc::kill(libc::getpid(), libc::SIGUSR2) };
        });
        let mut events = Vec::new();
        assert!(ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap());
        thread.join().unwrap();
    }
}
