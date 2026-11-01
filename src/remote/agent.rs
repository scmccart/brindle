//! Headless remote-session agent used by the `brindle-remote` binary.
//!
//! The agent owns exactly one session per process. Standard input and output
//! carry framed protocol values; stderr is reserved for startup diagnostics
//! from the binary and is never mixed into the framed stream.

use std::io;

use super::protocol::SpawnRequest;

/// Serves one already-described session over the process's framed stdio.
///
/// On success, the first frame written is always a `SpawnResponse`. A failed
/// spawn is also reported with a `SpawnResponse`, after which this returns
/// successfully because the protocol-level request was handled.
pub fn serve_stdio(request: SpawnRequest) -> io::Result<()> {
    platform::serve_stdio(request)
}

/// Compatibility entry point for a client that sends its `SpawnRequest` as
/// the first control-channel frame instead of passing JSON on the command
/// line.
pub fn serve_server_stdio() -> io::Result<()> {
    platform::serve_server_stdio()
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use std::io;

    use super::SpawnRequest;

    const UNSUPPORTED: &str = "brindle-remote sessions are supported only on Linux";

    pub fn serve_stdio(_request: SpawnRequest) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED))
    }

    pub fn serve_server_stdio() -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::path::PathBuf;
    use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use super::super::protocol::{
        CHANNEL_CONTROL, ExitMessage, FRAME_STREAM_MARKER, Frame, MSG_DATA, MSG_EXIT, MSG_PING, MSG_PONG,
        MSG_QUERY_PROCESS, MSG_RESIZE, MSG_SPAWN, ProcessInfo, ResizeRequest, SessionKind,
        SpawnRequest, SpawnResponse,
    };

    const OUTPUT_CHUNK_SIZE: usize = 32 * 1024;
    const EVENT_INTERVAL: Duration = Duration::from_millis(50);
    const GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(1);
    const FORCED_SHUTDOWN: Duration = Duration::from_secs(1);
    const OUTPUT_DRAIN_IDLE: Duration = Duration::from_millis(250);
    const OUTPUT_DRAIN_LIMIT: Duration = Duration::from_secs(2);
    /// How long the client may stop draining output before the session ends.
    /// Long enough to ride out a network stall or a laptop wake; a dead
    /// transport is detected separately (stdout write error, stdin EOF, ssh
    /// ServerAlive), so this only bounds a live but unresponsive client.
    const STDOUT_STALL_LIMIT: Duration = Duration::from_secs(120);
    const SESSION_INPUT_WRITE_LIMIT: Duration = Duration::from_secs(5);

    pub fn serve_server_stdio() -> io::Result<()> {
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
            if frame.channel_id != CHANNEL_CONTROL || frame.msg_type != MSG_SPAWN {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the first server frame must be a Spawn request on channel 0",
                ));
            }
            frame.decode_json::<SpawnRequest>()?
        };
        serve_stdio(request)
    }

    pub fn serve_stdio(request: SpawnRequest) -> io::Result<()> {
        if let Err(error) = validate_request(&request) {
            write_spawn_response(&SpawnResponse {
                channel_id: request.channel_id,
                success: false,
                error: Some(error.to_string()),
                pid: None,
            })?;
            return Ok(());
        }

        let mut session = match spawn_session(&request) {
            Ok(session) => session,
            Err(error) => {
                write_spawn_response(&SpawnResponse {
                    channel_id: request.channel_id,
                    success: false,
                    error: Some(error.to_string()),
                    pid: None,
                })?;
                return Ok(());
            }
        };

        if let Err(error) = write_spawn_response(&SpawnResponse {
            channel_id: request.channel_id,
            success: true,
            error: None,
            pid: Some(session.pid as u32),
        }) {
            signal_process_group(session.pid, libc::SIGKILL);
            let _ = session.child.wait();
            return Err(error);
        }

        run_session(request, session)
    }

    fn validate_request(request: &SpawnRequest) -> io::Result<()> {
        if request.channel_id == CHANNEL_CONTROL {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a session channel must be non-zero",
            ));
        }
        if request.command.as_deref().is_some_and(str::is_empty) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command must not be empty",
            ));
        }
        if request.kind == SessionKind::Pty && (request.cols == 0 || request.rows == 0) {
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
        input: SessionInput,
        output: SessionOutput,
        stderr: Option<ChildStderr>,
        kind: SessionKind,
    }

    enum SessionInput {
        Pty(Arc<OwnedFd>),
        Pipe(Option<ChildStdin>),
    }

    impl SessionInput {
        fn write_all(&mut self, bytes: &[u8], child_exited: &AtomicBool) -> io::Result<()> {
            match self {
                Self::Pty(master) => write_all_fd(master.as_raw_fd(), bytes, child_exited),
                Self::Pipe(Some(stdin)) => write_all_fd(stdin.as_raw_fd(), bytes, child_exited),
                Self::Pipe(None) => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "session input is closed",
                )),
            }
        }

        fn close(&mut self) {
            if let Self::Pipe(stdin) = self {
                stdin.take();
            }
        }

        fn pty_fd(&self) -> Option<RawFd> {
            match self {
                Self::Pty(master) => Some(master.as_raw_fd()),
                Self::Pipe(_) => None,
            }
        }
    }

    enum SessionOutput {
        Pty(Arc<OwnedFd>),
        Pipe(ChildStdout),
    }

    fn spawn_session(request: &SpawnRequest) -> io::Result<SpawnedSession> {
        match request.kind {
            SessionKind::Pty => spawn_pty(request),
            SessionKind::Tmux => spawn_tmux(request),
        }
    }

    fn spawn_pty(request: &SpawnRequest) -> io::Result<SpawnedSession> {
        let winsize = make_winsize(
            request.cols,
            request.rows,
            request.cell_width,
            request.cell_height,
        );
        let (master, slave) = open_pty(&winsize)?;
        set_nonblocking(master.as_raw_fd())?;

        let slave_stdin = duplicate_fd(slave.as_raw_fd())?;
        let slave_stdout = duplicate_fd(slave.as_raw_fd())?;
        let shell = default_shell(request);
        let mut command = command_for(request, shell.clone());
        if request.command.is_none() {
            // The default session is a login shell, as a terminal emulator or
            // sshd would start it.
            if let Some(name) = std::path::Path::new(&shell).file_name() {
                let mut login = OsString::from("-");
                login.push(name);
                command.arg0(login);
            }
        }
        if !request.env.contains_key("TERM") {
            command.env("TERM", "xterm-256color");
        }
        command
            .stdin(Stdio::from(File::from(slave_stdin)))
            .stdout(Stdio::from(File::from(slave_stdout)))
            .stderr(Stdio::from(File::from(slave)));

        // The child must become a session leader and acquire the slave as its
        // controlling terminal for shell job control and tcgetpgrp to work.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command.spawn()?;
        let pid = child.id() as libc::pid_t;
        let master = Arc::new(master);
        Ok(SpawnedSession {
            pid,
            child,
            input: SessionInput::Pty(master.clone()),
            output: SessionOutput::Pty(master),
            stderr: None,
            kind: SessionKind::Pty,
        })
    }

    fn spawn_tmux(request: &SpawnRequest) -> io::Result<SpawnedSession> {
        let mut command = command_for(request, OsString::from("tmux"));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn()?;
        let pid = child.id() as libc::pid_t;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => return cleanup_incomplete_child(child, "tmux stdin was not piped"),
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return cleanup_incomplete_child(child, "tmux stdout was not piped"),
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => return cleanup_incomplete_child(child, "tmux stderr was not piped"),
        };
        if let Err(error) = set_nonblocking(stdin.as_raw_fd()) {
            signal_process_group(pid, libc::SIGKILL);
            let _ = child.wait();
            return Err(error);
        }

        Ok(SpawnedSession {
            pid,
            child,
            input: SessionInput::Pipe(Some(stdin)),
            output: SessionOutput::Pipe(stdout),
            stderr: Some(stderr),
            kind: SessionKind::Tmux,
        })
    }

    fn cleanup_incomplete_child<T>(mut child: Child, message: &'static str) -> io::Result<T> {
        signal_process_group(child.id() as libc::pid_t, libc::SIGKILL);
        let _ = child.wait();
        Err(io::Error::other(message))
    }

    fn command_for(request: &SpawnRequest, default_program: OsString) -> Command {
        let program = request
            .command
            .as_ref()
            .map(|program| OsString::from(program.as_str()))
            .unwrap_or(default_program);
        let mut command = Command::new(program);
        command.args(&request.args).envs(&request.env);
        if let Some(cwd) = &request.cwd {
            // A directory that does not exist here (a path inherited from a
            // different host, or since deleted) starts the session in $HOME.
            let wanted = expand_remote_cwd(cwd, &request.env);
            if wanted.is_dir() {
                command.current_dir(wanted);
            } else if let Some(home) = home_dir(&request.env).filter(|home| home.is_dir()) {
                command.current_dir(home);
            }
        }
        command
    }

    fn home_dir(env: &std::collections::BTreeMap<String, String>) -> Option<PathBuf> {
        env.get("HOME")
            .filter(|home| !home.is_empty())
            .map(OsStr::new)
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|home| !home.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| passwd_entry().map(|(_, home)| home))
    }

    /// The login shell and home directory of the current user from the
    /// passwd database. `docker exec` sets no `SHELL`.
    fn passwd_entry() -> Option<(OsString, PathBuf)> {
        use std::ffi::CStr;
        use std::os::unix::ffi::OsStringExt;

        let mut buffer = vec![0u8; 4096];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::zeroed();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if status != 0 || found.is_null() {
            return None;
        }
        let entry = unsafe { entry.assume_init() };
        if entry.pw_shell.is_null() || entry.pw_dir.is_null() {
            return None;
        }
        let shell = unsafe { CStr::from_ptr(entry.pw_shell) }.to_bytes().to_vec();
        let dir = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes().to_vec();
        Some((
            OsString::from_vec(shell),
            PathBuf::from(OsString::from_vec(dir)),
        ))
    }

    fn expand_remote_cwd(cwd: &str, env: &std::collections::BTreeMap<String, String>) -> PathBuf {
        let home = || home_dir(env);
        if cwd == "~" {
            return home().unwrap_or_else(|| PathBuf::from(cwd));
        }
        if let Some(rest) = cwd.strip_prefix("~/")
            && let Some(home) = home()
        {
            return home.join(rest);
        }
        PathBuf::from(cwd)
    }

    fn default_shell(request: &SpawnRequest) -> OsString {
        request
            .env
            .get("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(|shell| OsString::from(shell.as_str()))
            .or_else(|| std::env::var_os("SHELL").filter(|shell| !shell.is_empty()))
            .or_else(|| {
                passwd_entry()
                    .map(|(shell, _)| shell)
                    .filter(|shell| !shell.is_empty() && PathBuf::from(shell).exists())
            })
            .unwrap_or_else(|| OsString::from("/bin/sh"))
    }

    fn open_pty(winsize: &libc::winsize) -> io::Result<(OwnedFd, OwnedFd)> {
        let mut master = -1;
        let mut slave = -1;
        let result = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                winsize,
            )
        };
        if result == -1 {
            let error = io::Error::last_os_error();
            if master >= 0 {
                unsafe { libc::close(master) };
            }
            if slave >= 0 {
                unsafe { libc::close(slave) };
            }
            return Err(error);
        }
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        set_close_on_exec(master.as_raw_fd())?;
        set_close_on_exec(slave.as_raw_fd())?;
        Ok((master, slave))
    }

    fn set_close_on_exec(fd: RawFd) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn duplicate_fd(fd: RawFd) -> io::Result<OwnedFd> {
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
    }

    fn set_nonblocking(fd: RawFd) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[derive(Debug)]
    enum Event {
        Incoming(Frame),
        InputEof,
        InputFailed(io::Error),
        Output(Vec<u8>),
        OutputClosed,
        OutputFailed(io::Error),
        Diagnostic(String),
        ChildExited(io::Result<Option<i32>>),
        WriterFailed(io::Error),
    }

    fn run_session(request: SpawnRequest, mut session: SpawnedSession) -> io::Result<()> {
        let child_exited = Arc::new(AtomicBool::new(false));
        let output_enabled = Arc::new(AtomicBool::new(true));
        let (event_tx, event_rx) = mpsc::sync_channel(32);
        let (outgoing_tx, writer) = spawn_frame_writer(event_tx.clone());

        spawn_input_reader(event_tx.clone());
        spawn_output_reader(
            session.output,
            event_tx.clone(),
            child_exited.clone(),
            output_enabled.clone(),
        );
        if let Some(stderr) = session.stderr.take() {
            spawn_stderr_reader(stderr, event_tx.clone(), output_enabled.clone());
        }
        spawn_child_waiter(session.child, event_tx.clone(), child_exited.clone());
        drop(event_tx);

        let mut child_waited = false;
        let mut exit_code = None;
        let mut output_closed = false;
        let mut drain_idle_deadline = None;
        let mut drain_hard_deadline = None;
        let mut shutdown_started = None;
        let mut force_killed_at = None;
        let mut terminal_error = None;
        let mut writer_failed = false;

        loop {
            match event_rx.recv_timeout(EVENT_INTERVAL) {
                Ok(Event::Incoming(frame)) => {
                    match handle_frame(
                        frame,
                        &request,
                        &mut session.input,
                        session.pid,
                        &child_exited,
                        &outgoing_tx,
                    ) {
                        Ok(FrameAction::Continue) => {}
                        Ok(FrameAction::Shutdown) => begin_shutdown(
                            &mut session.input,
                            session.pid,
                            session.kind,
                            &mut shutdown_started,
                            child_waited || child_exited.load(Ordering::Acquire),
                        ),
                        Err(error) => {
                            let _ = send_protocol_error(
                                &outgoing_tx,
                                request.channel_id,
                                error.to_string(),
                            );
                            remember_error(&mut terminal_error, error);
                            begin_shutdown(
                                &mut session.input,
                                session.pid,
                                session.kind,
                                &mut shutdown_started,
                                child_waited || child_exited.load(Ordering::Acquire),
                            );
                        }
                    }
                }
                Ok(Event::InputEof) => begin_shutdown(
                    &mut session.input,
                    session.pid,
                    session.kind,
                    &mut shutdown_started,
                    child_waited || child_exited.load(Ordering::Acquire),
                ),
                Ok(Event::InputFailed(error)) => {
                    let _ = send_protocol_error(
                        &outgoing_tx,
                        request.channel_id,
                        format!("failed to read a client frame: {error}"),
                    );
                    remember_error(&mut terminal_error, error);
                    begin_shutdown(
                        &mut session.input,
                        session.pid,
                        session.kind,
                        &mut shutdown_started,
                        child_waited || child_exited.load(Ordering::Acquire),
                    );
                }
                Ok(Event::Output(bytes)) => {
                    if output_enabled.load(Ordering::Acquire) {
                        if let Err(error) = send_output(
                            &outgoing_tx,
                            Frame::new(request.channel_id, MSG_DATA, bytes),
                        ) {
                            remember_error(&mut terminal_error, error);
                            begin_shutdown(
                                &mut session.input,
                                session.pid,
                                session.kind,
                                &mut shutdown_started,
                                child_waited || child_exited.load(Ordering::Acquire),
                            );
                        }
                        if child_waited {
                            drain_idle_deadline = Some(Instant::now() + OUTPUT_DRAIN_IDLE);
                        }
                    }
                }
                Ok(Event::OutputClosed) => output_closed = true,
                Ok(Event::OutputFailed(error)) => {
                    let _ = send_protocol_error(
                        &outgoing_tx,
                        request.channel_id,
                        format!("failed to read session output: {error}"),
                    );
                    remember_error(&mut terminal_error, error);
                    output_closed = true;
                    begin_shutdown(
                        &mut session.input,
                        session.pid,
                        session.kind,
                        &mut shutdown_started,
                        child_waited || child_exited.load(Ordering::Acquire),
                    );
                }
                Ok(Event::Diagnostic(message)) => {
                    if let Err(error) =
                        send_protocol_error(&outgoing_tx, request.channel_id, message)
                    {
                        remember_error(&mut terminal_error, error);
                    }
                }
                Ok(Event::ChildExited(result)) => {
                    child_waited = true;
                    match result {
                        Ok(code) => exit_code = code,
                        Err(error) => remember_error(&mut terminal_error, error),
                    }
                    let now = Instant::now();
                    drain_idle_deadline = Some(now + OUTPUT_DRAIN_IDLE);
                    drain_hard_deadline = Some(now + OUTPUT_DRAIN_LIMIT);
                }
                Ok(Event::WriterFailed(error)) => {
                    writer_failed = true;
                    remember_error(&mut terminal_error, error);
                    session.input.close();
                    if !child_waited && !child_exited.load(Ordering::Acquire) {
                        signal_process_group(session.pid, libc::SIGKILL);
                        force_killed_at.get_or_insert_with(Instant::now);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if !child_waited {
                        remember_error(
                            &mut terminal_error,
                            io::Error::other("all remote-agent workers stopped unexpectedly"),
                        );
                    }
                    break;
                }
            }

            let now = Instant::now();
            if let Some(started) = shutdown_started {
                if !child_waited
                    && !child_exited.load(Ordering::Acquire)
                    && force_killed_at.is_none()
                    && now.duration_since(started) >= GRACEFUL_SHUTDOWN
                {
                    signal_process_group(session.pid, libc::SIGKILL);
                    force_killed_at = Some(now);
                }
            }
            if let Some(killed_at) = force_killed_at {
                if !child_waited && now.duration_since(killed_at) >= FORCED_SHUTDOWN {
                    remember_error(
                        &mut terminal_error,
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "session did not exit after SIGKILL",
                        ),
                    );
                    break;
                }
            }

            if child_waited {
                let idle_expired = drain_idle_deadline.is_some_and(|deadline| now >= deadline);
                let limit_expired = drain_hard_deadline.is_some_and(|deadline| now >= deadline);
                if output_closed || idle_expired || limit_expired {
                    output_enabled.store(false, Ordering::Release);
                    if !writer_failed {
                        if let Err(error) =
                            send_outgoing(&outgoing_tx, Frame::exit(request.channel_id, exit_code))
                        {
                            remember_error(&mut terminal_error, error);
                        }
                    }
                    break;
                }
            }
        }

        output_enabled.store(false, Ordering::Release);
        session.input.close();
        if !child_waited && !child_exited.load(Ordering::Acquire) {
            signal_process_group(session.pid, libc::SIGKILL);
        }
        drop(outgoing_tx);

        let writer_result = writer
            .join()
            .map_err(|_| io::Error::other("remote frame writer thread panicked"))?;
        if let Err(error) = writer_result {
            remember_error(&mut terminal_error, error);
        }

        match terminal_error {
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
        request: &SpawnRequest,
        input: &mut SessionInput,
        child_pid: libc::pid_t,
        child_exited: &AtomicBool,
        outgoing: &SyncSender<Frame>,
    ) -> io::Result<FrameAction> {
        if frame.msg_type == MSG_PING {
            send_outgoing(
                outgoing,
                Frame::new(frame.channel_id, MSG_PONG, frame.payload),
            )?;
            return Ok(FrameAction::Continue);
        }
        if frame.channel_id != request.channel_id {
            send_protocol_error(
                outgoing,
                frame.channel_id,
                format!(
                    "frame is for channel {}, but this agent serves channel {}",
                    frame.channel_id, request.channel_id
                ),
            )?;
            return Ok(FrameAction::Continue);
        }

        match frame.msg_type {
            MSG_DATA => {
                input.write_all(&frame.payload, child_exited)?;
                Ok(FrameAction::Continue)
            }
            MSG_RESIZE => {
                let resize = match frame.decode_json::<ResizeRequest>() {
                    Ok(resize) if resize.cols != 0 && resize.rows != 0 => resize,
                    Ok(_) => {
                        send_protocol_error(
                            outgoing,
                            frame.channel_id,
                            "PTY rows and columns must be non-zero",
                        )?;
                        return Ok(FrameAction::Continue);
                    }
                    Err(error) => {
                        send_protocol_error(
                            outgoing,
                            frame.channel_id,
                            format!("invalid Resize payload: {error}"),
                        )?;
                        return Ok(FrameAction::Continue);
                    }
                };
                match input.pty_fd() {
                    Some(fd) => resize_pty(fd, &resize)?,
                    None => send_protocol_error(
                        outgoing,
                        frame.channel_id,
                        "Resize is valid only for PTY sessions",
                    )?,
                }
                Ok(FrameAction::Continue)
            }
            MSG_QUERY_PROCESS => {
                if !frame.payload.is_empty() {
                    send_protocol_error(
                        outgoing,
                        frame.channel_id,
                        "QueryProcess payload must be empty",
                    )?;
                    return Ok(FrameAction::Continue);
                }
                let info = inspect_process(input.pty_fd(), child_pid, request.channel_id);
                send_outgoing(outgoing, Frame::process_info(&info))?;
                Ok(FrameAction::Continue)
            }
            MSG_EXIT => {
                if let Err(error) = frame.decode_json::<ExitMessage>() {
                    send_protocol_error(
                        outgoing,
                        frame.channel_id,
                        format!("invalid Exit payload: {error}"),
                    )?;
                    return Ok(FrameAction::Continue);
                }
                Ok(FrameAction::Shutdown)
            }
            other => {
                send_protocol_error(
                    outgoing,
                    frame.channel_id,
                    format!("unsupported session message type {other}"),
                )?;
                Ok(FrameAction::Continue)
            }
        }
    }

    fn begin_shutdown(
        input: &mut SessionInput,
        pid: libc::pid_t,
        kind: SessionKind,
        started: &mut Option<Instant>,
        child_waited: bool,
    ) {
        input.close();
        if started.is_none() {
            *started = Some(Instant::now());
            if !child_waited {
                let signal = match kind {
                    SessionKind::Pty => libc::SIGHUP,
                    SessionKind::Tmux => libc::SIGTERM,
                };
                signal_process_group(pid, signal);
            }
        }
    }

    fn remember_error(slot: &mut Option<io::Error>, error: io::Error) {
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    fn send_protocol_error(
        outgoing: &SyncSender<Frame>,
        channel_id: u32,
        message: impl Into<String>,
    ) -> io::Result<()> {
        send_outgoing(outgoing, Frame::error(channel_id, message))
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
    /// bounded queues are the backpressure. Only a writer that makes no room
    /// for `STDOUT_STALL_LIMIT` is an error.
    fn send_output(outgoing: &SyncSender<Frame>, frame: io::Result<Frame>) -> io::Result<()> {
        let mut frame = frame?;
        let deadline = Instant::now() + STDOUT_STALL_LIMIT;
        loop {
            match outgoing.try_send(frame) {
                Ok(()) => return Ok(()),
                Err(mpsc::TrySendError::Full(returned)) => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "client is not draining remote output",
                        ));
                    }
                    frame = returned;
                    thread::sleep(Duration::from_millis(1));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "remote frame writer has stopped",
                    ));
                }
            }
        }
    }

    fn spawn_input_reader(events: SyncSender<Event>) {
        thread::spawn(move || {
            let stdin = io::stdin();
            let mut reader = stdin.lock();
            loop {
                match Frame::read_from_or_eof(&mut reader) {
                    Ok(Some(frame)) => {
                        if events.send(Event::Incoming(frame)).is_err() {
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
        output: SessionOutput,
        events: SyncSender<Event>,
        child_exited: Arc<AtomicBool>,
        enabled: Arc<AtomicBool>,
    ) {
        thread::spawn(move || match output {
            SessionOutput::Pty(master) => read_pty_output(master, &events, &child_exited, &enabled),
            SessionOutput::Pipe(stdout) => read_pipe_output(stdout, &events, &enabled),
        });
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

    fn read_pipe_output(mut output: ChildStdout, events: &SyncSender<Event>, enabled: &AtomicBool) {
        let mut buffer = vec![0; OUTPUT_CHUNK_SIZE];
        loop {
            match output.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    if !enabled.load(Ordering::Acquire) {
                        break;
                    }
                    if events
                        .send(Event::Output(buffer[..length].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = events.send(Event::OutputFailed(error));
                    break;
                }
            }
        }
        let _ = events.send(Event::OutputClosed);
    }

    fn spawn_stderr_reader(
        mut stderr: ChildStderr,
        events: SyncSender<Event>,
        enabled: Arc<AtomicBool>,
    ) {
        thread::spawn(move || {
            let mut buffer = vec![0; 4096];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(length) => {
                        if !enabled.load(Ordering::Acquire) {
                            return;
                        }
                        let message = String::from_utf8_lossy(&buffer[..length]).into_owned();
                        if events.send(Event::Diagnostic(message)).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = events.send(Event::Diagnostic(format!(
                            "failed to read session stderr: {error}"
                        )));
                        return;
                    }
                }
            }
        });
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

    fn make_winsize(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> libc::winsize {
        libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: cols.saturating_mul(cell_width),
            ws_ypixel: rows.saturating_mul(cell_height),
        }
    }

    fn resize_pty(fd: RawFd, request: &ResizeRequest) -> io::Result<()> {
        let winsize = make_winsize(
            request.cols,
            request.rows,
            request.cell_width,
            request.cell_height,
        );
        if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &winsize) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn inspect_process(
        pty_fd: Option<RawFd>,
        child_pid: libc::pid_t,
        channel_id: u32,
    ) -> ProcessInfo {
        let foreground_group = pty_fd
            .map(|fd| unsafe { libc::tcgetpgrp(fd) })
            .filter(|pid| *pid > 0);
        let pid = foreground_group
            .and_then(find_process_in_group)
            .unwrap_or(child_pid);
        let proc_dir = format!("/proc/{pid}");
        let foreground_name = fs::read_to_string(format!("{proc_dir}/comm"))
            .ok()
            .map(|name| {
                name.trim_end_matches(|character| character == '\r' || character == '\n')
                    .to_owned()
            })
            .filter(|name| !name.is_empty());
        let cwd = fs::read_link(format!("{proc_dir}/cwd"))
            .or_else(|_| fs::read_link(format!("/proc/{child_pid}/cwd")))
            .ok()
            .map(|path| path.to_string_lossy().into_owned());
        ProcessInfo {
            channel_id,
            pid: pid.max(0) as u32,
            foreground_name,
            cwd,
        }
    }

    fn find_process_in_group(group: libc::pid_t) -> Option<libc::pid_t> {
        if process_group(group) == Some(group) {
            return Some(group);
        }
        let entries = fs::read_dir("/proc").ok()?;
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<libc::pid_t>().ok())
            else {
                continue;
            };
            if process_group(pid) == Some(group) {
                return Some(pid);
            }
        }
        None
    }

    fn process_group(pid: libc::pid_t) -> Option<libc::pid_t> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields = stat.get(stat.rfind(')')? + 1..)?;
        // Fields after the command are: state (3), ppid (4), pgrp (5).
        fields.split_whitespace().nth(2)?.parse().ok()
    }

    fn signal_process_group(pid: libc::pid_t, signal: libc::c_int) {
        if pid > 0 {
            unsafe {
                libc::kill(-pid, signal);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn cwd_tilde_uses_remote_environment_home() {
            let env = [("HOME".to_string(), "/home/remote".to_string())].into();
            assert_eq!(expand_remote_cwd("~", &env), PathBuf::from("/home/remote"));
            assert_eq!(
                expand_remote_cwd("~/src tree", &env),
                PathBuf::from("/home/remote/src tree")
            );
            assert_eq!(
                expand_remote_cwd("/srv/app", &env),
                PathBuf::from("/srv/app")
            );
        }
    }
}
