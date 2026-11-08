//! Headless remote-session agent used by the `brindle-remote` binary.
//!
//! The agent owns exactly one session per process. Standard input and output
//! carry framed protocol values; stderr is reserved for startup diagnostics
//! from the binary and is never mixed into the framed stream.

/// Serves one session over the process's framed stdio: writes the frame
/// marker, reads the client's `SpawnRequest` as its first frame, then answers
/// with a `SpawnResponse`. A failed spawn is also reported with a
/// `SpawnResponse`, after which this returns successfully because the
/// protocol-level request was handled.
pub use platform::serve_stdio;

#[cfg(not(target_os = "linux"))]
mod platform {
    use std::io;

    pub fn serve_stdio() -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "brindle-remote sessions are supported only on Linux",
        ))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::io::{self, Write};
    use std::os::fd::{AsRawFd, OwnedFd, RawFd};
    use std::os::unix::process::ExitStatusExt;
    use std::process::Child;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use brindle_remote_protocol::{
        ExitMessage, FRAME_STREAM_MARKER, Frame, MSG_DATA, MSG_EXIT, MSG_HEARTBEAT,
        MSG_QUERY_PROCESS, MSG_RESIZE, MSG_SPAWN, ResizeRequest, SpawnRequest, SpawnResponse,
        validate_idle_limit,
    };

    use crate::pty::{
        self, FORCED_SHUTDOWN, GRACEFUL_SHUTDOWN, OUTPUT_CHUNK_SIZE, OUTPUT_DRAIN_IDLE,
        OUTPUT_DRAIN_LIMIT, PtyCommand, inspect_process, set_nonblocking, signal_process_group,
    };

    const EVENT_INTERVAL: Duration = Duration::from_millis(50);
    /// How long the client may stop draining output before the session ends.
    /// Long enough to ride out a network stall or a laptop wake; a dead
    /// transport is detected separately (stdout write error, stdin EOF, ssh
    /// ServerAlive), so this only bounds a live but unresponsive client.
    const STDOUT_STALL_LIMIT: Duration = Duration::from_secs(120);
    const SESSION_INPUT_WRITE_LIMIT: Duration = Duration::from_secs(5);

    pub fn serve_stdio() -> io::Result<()> {
        // Shell startup files can print before the helper runs; the marker
        // lets the client find where the frame stream begins.
        {
            let stdout = io::stdout();
            let mut writer = stdout.lock();
            writer.write_all(FRAME_STREAM_MARKER)?;
            writer.flush()?;
        }
        let request = {
            let stdin = io::stdin();
            let mut reader = stdin.lock();
            let frame = Frame::read_from(&mut reader)?;
            if frame.msg_type != MSG_SPAWN {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the first frame must be a Spawn request",
                ));
            }
            frame.decode_json::<SpawnRequest>()?
        };

        if let Err(error) = validate_request(&request) {
            write_spawn_response(&SpawnResponse {
                success: false,
                error: Some(error.to_string()),
                pid: None,
            })?;
            return Ok(());
        }

        let mut session = match spawn_pty(&request) {
            Ok(session) => session,
            Err(error) => {
                write_spawn_response(&SpawnResponse {
                    success: false,
                    error: Some(error.to_string()),
                    pid: None,
                })?;
                return Ok(());
            }
        };

        if let Err(error) = write_spawn_response(&SpawnResponse {
            success: true,
            error: None,
            pid: Some(session.pid as u32),
        }) {
            signal_process_group(session.pid, libc::SIGKILL);
            let _ = session.child.wait();
            return Err(error);
        }

        let idle_limit = Duration::from_secs(u64::from(request.idle_limit_secs));
        run_session(session, idle_limit)
    }

    fn validate_request(request: &SpawnRequest) -> io::Result<()> {
        validate_idle_limit(request.idle_limit_secs)?;
        if request.command.as_deref().is_some_and(str::is_empty) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command must not be empty",
            ));
        }
        if request.cols == 0 || request.rows == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "PTY rows and columns must be non-zero",
            ));
        }
        Ok(())
    }

    fn write_spawn_response(response: &SpawnResponse) -> io::Result<()> {
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        Frame::spawn_response(response)?.write_to(&mut writer)
    }

    struct SpawnedSession {
        pid: libc::pid_t,
        child: Child,
        /// The PTY master: session input goes in, output and the process
        /// queries come from it.
        master: Arc<OwnedFd>,
    }

    fn spawn_pty(request: &SpawnRequest) -> io::Result<SpawnedSession> {
        let spawned = pty::spawn(&PtyCommand {
            command: request.command.as_deref(),
            args: &request.args,
            cwd: request.cwd.as_deref(),
            env: &request.env,
            cols: request.cols,
            rows: request.rows,
            cell_width: request.cell_width,
            cell_height: request.cell_height,
            exact_env: false,
        })?;
        Ok(SpawnedSession {
            pid: spawned.pid,
            child: spawned.child,
            master: Arc::new(spawned.master),
        })
    }

    #[derive(Debug)]
    enum Event {
        Incoming(Frame),
        InputEof,
        InputFailed(io::Error),
        Output(Vec<u8>),
        OutputClosed,
        OutputFailed(io::Error),
        ChildExited(io::Result<Option<i32>>),
        WriterFailed(io::Error),
    }

    /// How the session is being ended: the signals sent to its process group
    /// and the first error, which the agent reports when it exits.
    struct Shutdown {
        pid: libc::pid_t,
        child_exited: Arc<AtomicBool>,
        started: Option<Instant>,
        force_killed_at: Option<Instant>,
        error: Option<io::Error>,
    }

    impl Shutdown {
        fn new(pid: libc::pid_t, child_exited: Arc<AtomicBool>) -> Self {
            Self {
                pid,
                child_exited,
                started: None,
                force_killed_at: None,
                error: None,
            }
        }

        /// The child has been waited for or is a zombie, so signals are moot.
        fn child_gone(&self, child_waited: bool) -> bool {
            child_waited || self.child_exited.load(Ordering::Acquire)
        }

        fn remember(&mut self, error: io::Error) {
            self.error.get_or_insert(error);
        }

        /// Hangs the session up once; later calls only keep the clock.
        fn begin(&mut self, child_waited: bool) {
            if self.started.is_none() {
                self.started = Some(Instant::now());
                if !self.child_gone(child_waited) {
                    signal_process_group(self.pid, libc::SIGHUP);
                }
            }
        }

        /// Tells the client what went wrong, remembers it and ends the session.
        fn fail(
            &mut self,
            outgoing: &SyncSender<Frame>,
            message: impl Into<String>,
            error: io::Error,
            child_waited: bool,
        ) {
            let _ = send_protocol_error(outgoing, message);
            self.remember(error);
            self.begin(child_waited);
        }

        /// Kills the process group unless the child is gone; says whether it did.
        fn kill(&self, child_waited: bool) -> bool {
            let alive = !self.child_gone(child_waited);
            if alive {
                signal_process_group(self.pid, libc::SIGKILL);
            }
            alive
        }
    }

    /// When the client last sent a frame, as measured by the stdin reader
    /// thread. The session loop can block for a long time on a slow client's
    /// output, so only the reader, which keeps taking frames off stdin, can
    /// tell silence from a backlog.
    ///
    /// The reader stamps every frame it reads, including heartbeats, which it
    /// then drops. A real frame is handed to the loop through the event queue,
    /// which can be full; while the reader waits for room, `busy` says the
    /// client has just sent something, and finishing the hand-over stamps
    /// again. The loop reads `busy` before the stamp (both Acquire, paired
    /// with Release stores in the reader's order stamp, busy, stamp, not-busy),
    /// so it never pairs an old stamp with a finished hand-over. Nothing here
    /// is a lock, so a blocked reader holds nothing the loop needs.
    struct Liveness {
        start: Instant,
        /// Milliseconds from `start` to the latest stamp.
        last_frame_ms: AtomicU64,
        busy: AtomicBool,
    }

    impl Liveness {
        /// The clock starts at `start`, so a client that sends nothing after
        /// its spawn request is still bounded.
        fn new(start: Instant) -> Self {
            Self {
                start,
                last_frame_ms: AtomicU64::new(0),
                busy: AtomicBool::new(false),
            }
        }

        fn stamp(&self, now: Instant) {
            let ms = now.saturating_duration_since(self.start).as_millis() as u64;
            self.last_frame_ms.fetch_max(ms, Ordering::Release);
        }

        fn set_busy(&self, busy: bool) {
            self.busy.store(busy, Ordering::Release);
        }

        /// The reader has passed a real frame on.
        fn finish_handover(&self, now: Instant) {
            self.stamp(now);
            self.set_busy(false);
        }

        /// Whether the client has sent nothing for the whole `limit`.
        fn expired(&self, now: Instant, limit: Duration) -> bool {
            if self.busy.load(Ordering::Acquire) {
                return false;
            }
            let last = Duration::from_millis(self.last_frame_ms.load(Ordering::Acquire));
            now.saturating_duration_since(self.start)
                .saturating_sub(last)
                >= limit
        }
    }

    fn run_session(session: SpawnedSession, idle_limit: Duration) -> io::Result<()> {
        let child_exited = Arc::new(AtomicBool::new(false));
        let output_enabled = Arc::new(AtomicBool::new(true));
        let (event_tx, event_rx) = mpsc::sync_channel(32);
        let (outgoing_tx, writer) = spawn_frame_writer(event_tx.clone());

        let liveness = Arc::new(Liveness::new(Instant::now()));
        spawn_input_reader(event_tx.clone(), liveness.clone());
        spawn_output_reader(
            session.master.clone(),
            event_tx.clone(),
            child_exited.clone(),
            output_enabled.clone(),
        );
        spawn_child_waiter(session.child, event_tx.clone(), child_exited.clone());
        drop(event_tx);

        let master_fd = session.master.as_raw_fd();
        let mut shutdown = Shutdown::new(session.pid, child_exited.clone());
        let mut child_waited = false;
        let mut exit_code = None;
        let mut output_closed = false;
        let mut drain_idle_deadline = None;
        let mut drain_hard_deadline = None;
        let mut writer_failed = false;

        loop {
            match event_rx.recv_timeout(EVENT_INTERVAL) {
                Ok(Event::Incoming(frame)) => {
                    match handle_frame(frame, master_fd, session.pid, &child_exited, &outgoing_tx) {
                        Ok(FrameAction::Continue) => {}
                        Ok(FrameAction::Shutdown) => shutdown.begin(child_waited),
                        Err(error) => {
                            let message = error.to_string();
                            shutdown.fail(&outgoing_tx, message, error, child_waited);
                        }
                    }
                }
                Ok(Event::InputEof) => shutdown.begin(child_waited),
                Ok(Event::InputFailed(error)) => {
                    let message = format!("failed to read a client frame: {error}");
                    shutdown.fail(&outgoing_tx, message, error, child_waited);
                }
                Ok(Event::Output(bytes)) => {
                    if output_enabled.load(Ordering::Acquire) {
                        if let Err(error) = send_output(&outgoing_tx, Frame::data(bytes)) {
                            shutdown.remember(error);
                            shutdown.begin(child_waited);
                        }
                        if child_waited {
                            drain_idle_deadline = Some(Instant::now() + OUTPUT_DRAIN_IDLE);
                        }
                    }
                }
                Ok(Event::OutputClosed) => output_closed = true,
                Ok(Event::OutputFailed(error)) => {
                    let message = format!("failed to read session output: {error}");
                    output_closed = true;
                    shutdown.fail(&outgoing_tx, message, error, child_waited);
                }
                Ok(Event::ChildExited(result)) => {
                    child_waited = true;
                    match result {
                        Ok(code) => exit_code = code,
                        Err(error) => shutdown.remember(error),
                    }
                    let now = Instant::now();
                    drain_idle_deadline = Some(now + OUTPUT_DRAIN_IDLE);
                    drain_hard_deadline = Some(now + OUTPUT_DRAIN_LIMIT);
                }
                Ok(Event::WriterFailed(error)) => {
                    writer_failed = true;
                    shutdown.remember(error);
                    if shutdown.kill(child_waited) {
                        shutdown.force_killed_at.get_or_insert_with(Instant::now);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if !child_waited {
                        shutdown.remember(io::Error::other(
                            "all remote-agent workers stopped unexpectedly",
                        ));
                    }
                    break;
                }
            }

            let now = Instant::now();
            // On every pass, events and timeouts alike, so steady output
            // can't starve it. The reader's clock is independent of how far
            // behind this loop is.
            if shutdown.started.is_none() && !child_waited && liveness.expired(now, idle_limit) {
                let message = format!(
                    "no message from the client for {} seconds; ending the session",
                    idle_limit.as_secs()
                );
                let error = io::Error::new(io::ErrorKind::TimedOut, message.clone());
                shutdown.fail(&outgoing_tx, message, error, child_waited);
            }
            if let Some(started) = shutdown.started {
                if shutdown.force_killed_at.is_none()
                    && now.duration_since(started) >= GRACEFUL_SHUTDOWN
                    && shutdown.kill(child_waited)
                {
                    shutdown.force_killed_at = Some(now);
                }
            }
            if let Some(killed_at) = shutdown.force_killed_at {
                if !child_waited && now.duration_since(killed_at) >= FORCED_SHUTDOWN {
                    shutdown.remember(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "session did not exit after SIGKILL",
                    ));
                    break;
                }
            }

            if child_waited {
                let idle_expired = drain_idle_deadline.is_some_and(|deadline| now >= deadline);
                let limit_expired = drain_hard_deadline.is_some_and(|deadline| now >= deadline);
                if output_closed || idle_expired || limit_expired {
                    output_enabled.store(false, Ordering::Release);
                    if !writer_failed {
                        if let Err(error) = send_outgoing(&outgoing_tx, Frame::exit(exit_code)) {
                            shutdown.remember(error);
                        }
                    }
                    break;
                }
            }
        }

        output_enabled.store(false, Ordering::Release);
        let _ = shutdown.kill(child_waited);
        drop(outgoing_tx);

        let writer_result = writer
            .join()
            .map_err(|_| io::Error::other("remote frame writer thread panicked"))?;
        if let Err(error) = writer_result {
            shutdown.remember(error);
        }

        match shutdown.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FrameAction {
        Continue,
        Shutdown,
    }

    fn handle_frame(
        frame: Frame,
        master_fd: RawFd,
        child_pid: libc::pid_t,
        child_exited: &AtomicBool,
        outgoing: &SyncSender<Frame>,
    ) -> io::Result<FrameAction> {
        match frame.msg_type {
            MSG_DATA => {
                write_all_fd(master_fd, &frame.payload, child_exited)?;
                Ok(FrameAction::Continue)
            }
            MSG_RESIZE => {
                let resize = match frame.decode_json::<ResizeRequest>() {
                    Ok(resize) if resize.cols != 0 && resize.rows != 0 => resize,
                    Ok(_) => {
                        send_protocol_error(outgoing, "PTY rows and columns must be non-zero")?;
                        return Ok(FrameAction::Continue);
                    }
                    Err(error) => {
                        send_protocol_error(outgoing, format!("invalid Resize payload: {error}"))?;
                        return Ok(FrameAction::Continue);
                    }
                };
                resize_pty(master_fd, &resize)?;
                Ok(FrameAction::Continue)
            }
            MSG_QUERY_PROCESS => {
                if !frame.payload.is_empty() {
                    send_protocol_error(outgoing, "QueryProcess payload must be empty")?;
                    return Ok(FrameAction::Continue);
                }
                let info = inspect_process(master_fd, child_pid);
                send_outgoing(outgoing, Frame::process_info(&info))?;
                Ok(FrameAction::Continue)
            }
            MSG_EXIT => {
                if let Err(error) = frame.decode_json::<ExitMessage>() {
                    send_protocol_error(outgoing, format!("invalid Exit payload: {error}"))?;
                    return Ok(FrameAction::Continue);
                }
                Ok(FrameAction::Shutdown)
            }
            other => {
                send_protocol_error(
                    outgoing,
                    format!("unsupported session message type {other}"),
                )?;
                Ok(FrameAction::Continue)
            }
        }
    }

    fn send_protocol_error(
        outgoing: &SyncSender<Frame>,
        message: impl Into<String>,
    ) -> io::Result<()> {
        send_outgoing(outgoing, Frame::error(message))
    }

    fn send_outgoing(outgoing: &SyncSender<Frame>, frame: io::Result<Frame>) -> io::Result<()> {
        outgoing.try_send(frame?).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => io::Error::new(
                io::ErrorKind::TimedOut,
                "client is not draining remote output",
            ),
            mpsc::TrySendError::Disconnected(_) => {
                io::Error::new(io::ErrorKind::BrokenPipe, "remote frame writer has stopped")
            }
        })
    }

    /// Session output waits for room in the writer queue instead of failing:
    /// a PTY can produce data far faster than a slow link drains it, and the
    /// bounded queues are the backpressure. A writer that makes no progress
    /// for `STDOUT_STALL_LIMIT` fails (`TimedWriter`) and exits, which drops
    /// the queue's receiver and ends this wait with an error.
    fn send_output(outgoing: &SyncSender<Frame>, frame: io::Result<Frame>) -> io::Result<()> {
        outgoing.send(frame?).map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "remote frame writer has stopped")
        })
    }

    fn spawn_input_reader(events: SyncSender<Event>, liveness: Arc<Liveness>) {
        thread::spawn(move || {
            let stdin = io::stdin();
            let mut reader = stdin.lock();
            loop {
                match Frame::read_from_or_eof(&mut reader) {
                    Ok(Some(frame)) => {
                        liveness.stamp(Instant::now());
                        // Proof of life only: it never waits in the queue
                        // behind output.
                        if frame.msg_type == MSG_HEARTBEAT {
                            continue;
                        }
                        liveness.set_busy(true);
                        let sent = events.send(Event::Incoming(frame));
                        liveness.finish_handover(Instant::now());
                        if sent.is_err() {
                            return;
                        }
                    }
                    Ok(None) => {
                        let _ = events.send(Event::InputEof);
                        return;
                    }
                    Err(error) => {
                        let _ = events.send(Event::InputFailed(error));
                        return;
                    }
                }
            }
        });
    }

    fn spawn_output_reader(
        master: Arc<OwnedFd>,
        events: SyncSender<Event>,
        child_exited: Arc<AtomicBool>,
        enabled: Arc<AtomicBool>,
    ) {
        thread::spawn(move || read_pty_output(master, &events, &child_exited, &enabled));
    }

    fn read_pty_output(
        master: Arc<OwnedFd>,
        events: &SyncSender<Event>,
        child_exited: &AtomicBool,
        enabled: &AtomicBool,
    ) {
        let fd = master.as_raw_fd();
        let mut buffer = vec![0; OUTPUT_CHUNK_SIZE];
        'outer: loop {
            let mut poll_fd = libc::pollfd {
                fd,
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll_fd, 1, 100) };
            if ready == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                let _ = events.send(Event::OutputFailed(error));
                break;
            }
            if ready == 0 {
                if child_exited.load(Ordering::Acquire) || !enabled.load(Ordering::Acquire) {
                    break;
                }
                continue;
            }
            if poll_fd.revents & libc::POLLNVAL != 0 {
                let _ = events.send(Event::OutputFailed(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "PTY master file descriptor became invalid",
                )));
                break;
            }

            loop {
                let read = unsafe {
                    libc::read(fd, buffer.as_mut_ptr().cast::<libc::c_void>(), buffer.len())
                };
                if read > 0 {
                    if !enabled.load(Ordering::Acquire) {
                        break 'outer;
                    }
                    if events
                        .send(Event::Output(buffer[..read as usize].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                if read == 0 {
                    break 'outer;
                }

                let error = io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EAGAIN) => break,
                    Some(libc::EIO) => break 'outer,
                    _ if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => {
                        let _ = events.send(Event::OutputFailed(error));
                        break 'outer;
                    }
                }
            }

            if child_exited.load(Ordering::Acquire) || !enabled.load(Ordering::Acquire) {
                break;
            }
        }
        let _ = events.send(Event::OutputClosed);
    }

    fn spawn_child_waiter(
        mut child: Child,
        events: SyncSender<Event>,
        child_exited: Arc<AtomicBool>,
    ) {
        thread::spawn(move || {
            // Observe exit without reaping first. While the child is a zombie
            // its PID cannot be reused; publish that fact before `wait` reaps it.
            let observed = loop {
                let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        child.id() as libc::id_t,
                        info.as_mut_ptr(),
                        libc::WEXITED | libc::WNOWAIT,
                    )
                };
                if result == 0 {
                    break true;
                }
                if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break false;
                }
            };
            if observed {
                child_exited.store(true, Ordering::Release);
            }
            let result = child.wait().map(|status| {
                status
                    .code()
                    .or_else(|| status.signal().map(|signal| -signal))
            });
            child_exited.store(true, Ordering::Release);
            let _ = events.send(Event::ChildExited(result));
        });
    }

    fn spawn_frame_writer(
        events: SyncSender<Event>,
    ) -> (SyncSender<Frame>, JoinHandle<io::Result<()>>) {
        let (frames, receiver) = mpsc::sync_channel(32);
        let handle = thread::spawn(move || write_frames(receiver, events));
        (frames, handle)
    }

    fn write_frames(frames: Receiver<Frame>, events: SyncSender<Event>) -> io::Result<()> {
        if let Err(error) = set_nonblocking(libc::STDOUT_FILENO) {
            let report = io::Error::new(error.kind(), error.to_string());
            let _ = events.try_send(Event::WriterFailed(report));
            return Err(error);
        }
        let mut writer = TimedWriter {
            fd: libc::STDOUT_FILENO,
        };
        while let Ok(frame) = frames.recv() {
            if let Err(error) = frame.write_to(&mut writer) {
                let report = io::Error::new(error.kind(), error.to_string());
                let _ = events.try_send(Event::WriterFailed(report));
                return Err(error);
            }
        }
        Ok(())
    }

    struct TimedWriter {
        fd: RawFd,
    }

    impl Write for TimedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let deadline = Instant::now() + STDOUT_STALL_LIMIT;
            loop {
                let written = unsafe {
                    libc::write(
                        self.fd,
                        buffer.as_ptr().cast::<libc::c_void>(),
                        buffer.len(),
                    )
                };
                if written >= 0 {
                    return Ok(written as usize);
                }

                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.kind() == io::ErrorKind::WouldBlock {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "remote client stopped reading framed output",
                        ));
                    }
                    let remaining = deadline.duration_since(now);
                    poll_writable(self.fd, remaining.min(Duration::from_millis(100)))?;
                    continue;
                }
                return Err(error);
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn poll_writable(fd: RawFd, timeout: Duration) -> io::Result<()> {
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        let result = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(());
            }
            return Err(error);
        }
        if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "remote client output pipe closed",
            ));
        }
        Ok(())
    }

    fn write_all_fd(fd: RawFd, bytes: &[u8], child_exited: &AtomicBool) -> io::Result<()> {
        let deadline = Instant::now() + SESSION_INPUT_WRITE_LIMIT;
        let mut written = 0;
        while written < bytes.len() {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "session stopped consuming input",
                ));
            }
            let result = unsafe {
                libc::write(
                    fd,
                    bytes[written..].as_ptr().cast::<libc::c_void>(),
                    bytes.len() - written,
                )
            };
            if result > 0 {
                written += result as usize;
                continue;
            }
            if result == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "session input write returned zero",
                ));
            }

            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.kind() == io::ErrorKind::WouldBlock {
                if child_exited.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "session exited while input was pending",
                    ));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                poll_writable(fd, remaining.min(Duration::from_millis(100)))?;
                continue;
            }
            return Err(error);
        }
        Ok(())
    }

    fn resize_pty(fd: RawFd, request: &ResizeRequest) -> io::Result<()> {
        pty::resize(
            fd,
            request.cols,
            request.rows,
            request.cell_width,
            request.cell_height,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn request(idle_limit_secs: u32) -> SpawnRequest {
            SpawnRequest {
                command: None,
                args: Vec::new(),
                cwd: None,
                env: Default::default(),
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
                idle_limit_secs,
            }
        }

        #[test]
        fn requests_need_a_sensible_idle_limit() {
            assert!(validate_request(&request(60)).is_ok());
            assert!(validate_request(&request(0)).is_err());
            assert!(validate_request(&request(u32::MAX)).is_err());
        }

        #[test]
        fn the_client_is_silent_only_after_the_whole_limit() {
            let start = Instant::now();
            let limit = Duration::from_secs(60);
            let live = Liveness::new(start);
            assert!(!live.expired(start, limit));
            assert!(!live.expired(start + Duration::from_secs(59), limit));
            assert!(live.expired(start + limit, limit));
            // A clock reading before the stamp is not silence.
            live.stamp(start + limit);
            assert!(!live.expired(start, limit));
        }

        #[test]
        fn a_stamp_clears_the_silence() {
            let start = Instant::now();
            let limit = Duration::from_secs(60);
            let live = Liveness::new(start);
            assert!(live.expired(start + limit, limit));
            live.stamp(start + limit);
            assert!(!live.expired(start + limit + Duration::from_secs(59), limit));
            assert!(live.expired(start + limit + limit, limit));
        }

        #[test]
        fn a_frame_being_handed_over_is_never_silence() {
            let start = Instant::now();
            let limit = Duration::from_secs(60);
            let live = Liveness::new(start);
            live.set_busy(true);
            assert!(!live.expired(start + limit * 10, limit));
            // Finishing the hand-over counts as the client's latest sign of life.
            live.finish_handover(start + limit * 10);
            assert!(!live.expired(start + limit * 10 + Duration::from_secs(1), limit));
            assert!(live.expired(start + limit * 11, limit));
        }
    }
}
