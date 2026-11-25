//! `brindle-remote hold`, driven by a test core over H1 (tasks 3.2–3.4).
//!
//! Most tests run on both platforms with the platform's own programs
//! ([`programs`]). Those that need a Unix shell, exact PTY bytes, signals
//! or `/proc` are Linux's, and Windows has its own beside them for the same
//! behaviour over a pseudoconsole, whose output carries its own escape
//! sequences and isn't compared byte for byte.

#![cfg(any(target_os = "linux", windows))]

mod support;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use brindle_remote_protocol::Frame;
use brindle_remote_protocol::endpoint::Endpoints;
use brindle_remote_protocol::holder::{self as h1, Drain, Goodbye, Previous, ToCore, ToHolder};

use support::*;

/// The sessions' programs, as a Unix shell runs them.
#[cfg(unix)]
mod programs {
    use std::path::Path;

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::holder::Spawn;

    use super::support::shell;

    /// What ends a line of input.
    pub const ENTER: &[u8] = b"\n";
    /// How many bytes [`flood`] prints.
    const FLOOD: usize = 6 * 1024 * 1024;

    /// Exits with `code`.
    pub fn exits(session: SessionId, code: i32) -> Spawn {
        shell(session, &format!("exit {code}"))
    }

    /// Waits for a line of input, then exits with `code`.
    pub fn exits_after_a_line(session: SessionId, code: i32) -> Spawn {
        shell(session, &format!("read x; exit {code}"))
    }

    /// Prints `one`, then `two:<line>` for the line it reads, then waits
    /// until [`release`] lets it go through `go` (a FIFO, made here) and
    /// exits with 0. Releasing it needs no core.
    ///
    /// It opens `go` (read-write, which doesn't wait for a writer) before
    /// it prints `one`, and keeps it open until it reads: a byte written to
    /// a FIFO that no one else has open is lost when the writer closes it,
    /// so a program that opened `go` only when it got to its last `read`
    /// could miss a [`release`] that came first, and never exit.
    pub fn echoes_a_line(session: SessionId, go: &Path) -> Spawn {
        make_fifo(go);
        let script = format!(
            "exec 3<>'{}'; echo one; read x; echo \"two:$x\"; read y <&3",
            go.display()
        );
        shell(session, &script)
    }

    /// Lets an [`echoes_a_line`] program waiting on `go` go.
    pub fn release(go: &Path) {
        use std::io::Write as _;

        // Opened for reading too, so the open doesn't wait for the program's.
        let mut fifo = std::fs::OpenOptions::new().read(true).write(true).open(go).unwrap();
        fifo.write_all(b"
").unwrap();
    }

    /// Makes a FIFO at `path`. A session's program that reads it waits until
    /// the test writes to it.
    pub fn make_fifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;

        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c_path` is a NUL-terminated string that outlives the call.
        let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo: {}", std::io::Error::last_os_error());
    }

    /// Prints a flood and exits with 0. No newlines, so the PTY passes the
    /// bytes through unchanged.
    pub fn flood(session: SessionId, _dir: &Path) -> Spawn {
        let script = format!("head -c {FLOOD} /dev/zero | tr '\\0' x; exit 0");
        shell(session, &script)
    }

    /// Fails unless `output` is all of [`flood`]'s.
    pub fn check_flood(output: &[u8]) {
        assert!(output.iter().all(|&byte| byte == b'x'));
        assert_eq!(output.len(), FLOOD, "every byte of output");
    }

    /// Prints more than a tail holds, then `END`, and waits for a line.
    pub fn overflows_the_tail(session: SessionId, _dir: &Path) -> Spawn {
        let script = "head -c 1500000 /dev/zero | tr '\\0' x; echo; echo END; read z";
        shell(session, script)
    }
}

/// The sessions' programs, as `cmd.exe` runs them.
#[cfg(windows)]
mod programs {
    use std::path::Path;

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::holder::Spawn;

    use super::support::shell;
    pub use super::support::{check_lines, numbered_lines};

    /// What ends a line of input.
    pub const ENTER: &[u8] = b"\r";
    /// How many lines [`flood`] prints: about 6 MB.
    const FLOOD_LINES: usize = 85_000;

    /// Exits with `code`.
    pub fn exits(session: SessionId, code: i32) -> Spawn {
        shell(session, &format!("exit {code}"))
    }

    /// Waits for a line of input, then exits with `code`.
    pub fn exits_after_a_line(session: SessionId, code: i32) -> Spawn {
        shell(session, &format!("set /p x=& exit {code}"))
    }

    /// Prints `one`, then `two:<line>` for the line it reads, then waits
    /// until [`release`] creates `go` and exits with 0. Releasing it needs
    /// no core. The path goes unquoted (the launch line would escape the
    /// quotes, which `cmd` doesn't undo), so it has no space.
    pub fn echoes_a_line(session: SessionId, go: &Path) -> Spawn {
        assert!(!go.to_str().unwrap().contains(' '), "{go:?}");
        let wait = format!(
            "for /l %i in (1,1,600) do @(if exist {} (exit 0) else ping -n 2 127.0.0.1 >nul)",
            go.display()
        );
        shell(session, &format!("echo one& set /p x=& echo two:!x!& {wait}"))
    }

    /// Lets an [`echoes_a_line`] program waiting on `go` go.
    pub fn release(go: &Path) {
        std::fs::write(go, b"").unwrap();
    }

    /// Prints a flood and exits with 0.
    pub fn flood(session: SessionId, dir: &Path) -> Spawn {
        let file = numbered_lines(dir, FLOOD_LINES);
        shell(session, &format!("type {}& exit 0", file.display()))
    }

    /// Fails unless `output` is all of [`flood`]'s.
    pub fn check_flood(output: &[u8]) {
        check_lines(output, FLOOD_LINES);
    }

    /// Prints more than a tail holds, then `END`, and waits for a line.
    pub fn overflows_the_tail(session: SessionId, dir: &Path) -> Spawn {
        let file = numbered_lines(dir, 21_000);
        shell(session, &format!("type {}& echo END& set /p z=", file.display()))
    }
}

/// A holder in a fresh directory, with a connected test core.
fn holder(tag: &str) -> (TempDir, TestCore, u32) {
    let mut dir = TempDir::new(tag);
    start_holder(&dir.path);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(1);
    let pid = adopted.welcome().pid;
    dir.track(pid);
    (dir, core, pid)
}

fn holder_exits(dir: &TempDir, pid: u32) -> bool {
    eventually(TIMEOUT, || !alive(pid) && dir.holder_endpoints().is_empty())
}

#[cfg(unix)]
/// Runs `printf R >&3` with a new pipe's write end passed as fd 3, and checks
/// that the byte arrives. The pipe is first put on fd 3 itself (`on_target`),
/// close-on-exec as `pipe2` can return it, or kept off it. That is done in
/// the forked child, whose fds are its own copy, so no other test's fd 3 is
/// touched.
fn assert_passed_as_fd_3(on_target: bool) {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let write_fd = write.as_raw_fd();
    let mut command = Command::new("sh");
    command.args(["-c", "printf R >&3"]).stdin(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            let fd = match (on_target, write_fd == 3) {
                (true, false) => libc::dup3(write_fd, 3, libc::O_CLOEXEC),
                (false, true) => libc::fcntl(write_fd, libc::F_DUPFD_CLOEXEC, 4),
                _ => write_fd,
            };
            if fd == -1 {
                return Err(std::io::Error::last_os_error());
            }
            pass_fd_as(fd, 3)
        });
    }
    let output = command.output().unwrap();
    drop(write);
    let mut got = String::new();
    fs::File::from(read).read_to_string(&mut got).unwrap();
    assert_eq!(
        got,
        "R",
        "on_target {on_target}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn a_ready_pipe_is_passed_as_fd_3_whether_or_not_it_is_there_already() {
    // `dup2(3, 3)` does nothing, so the server's exec would close it and
    // `--ready-fd 3` would name a closed fd.
    assert_passed_as_fd_3(true);
    assert_passed_as_fd_3(false);
}

/// `Command` reports a failed exec through a close-on-exec pipe in the
/// child. When that pipe's write end is fd 3, passing the ready pipe as fd 3
/// replaces it, and the error's bytes arrive in the ready pipe, where the
/// first of them once read as "ready".
#[cfg(unix)]
#[test]
fn a_failed_exec_isnt_ready_though_its_error_went_down_the_ready_pipe() {
    let dir = TempDir::new("exec-error");
    let missing = Path::new("/nonexistent/brindle-remote");
    let error = start_with(missing, "hold", &dir.path, &[], send_exec_error_down_fd_3)
        .expect_err("a program that isn't there started");
    let error = error.to_string();
    assert!(error.starts_with("hold wasn't ready"), "{error}");
    // The error did go down the ready pipe: 8 bytes, where a ready server
    // writes one. (Had it gone to `Command`, the message would name the
    // missing program instead.)
    assert!(error.contains("its ready pipe held 8 bytes"), "{error}");
}

#[test]
fn a_new_holder_is_detached_and_welcomes_its_first_core() {
    let mut dir = TempDir::new("welcome");
    start_holder(&dir.path);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(1);
    let welcome = adopted.welcome();
    dir.track(welcome.pid);
    assert_eq!(welcome.protocol, h1::PROTOCOLS.max);
    assert_eq!(welcome.previous, Previous::None);
    assert!(welcome.sessions.is_empty());
    assert!(!welcome.build.is_empty());
    assert_ne!(parent(welcome.pid), Some(std::process::id()));
    // It is the first holder of its build, and listens where that one does.
    let place = Endpoints::of(&dir.path).unwrap().holder(&welcome.build, 0);
    assert_eq!(dir.holder_endpoints(), std::slice::from_ref(&place.endpoint));
    assert!(place.lock.exists() && place.log.exists());
    #[cfg(unix)]
    {
        let name = place.endpoint.file_name().unwrap().to_string_lossy();
        assert_eq!(name, format!("holder-{}-0.sock", welcome.build));
        let mode = fs::metadata(&dir.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    core.send(ToHolder::Reparent(h1::Reparent {
        session: 9,
        hwnd: 1,
    }));
    // REPARENT is advisory, and does something only on Windows: no answer
    // either way (what it came to is the holder's log's), so the next
    // message is the next request's.
    core.spawn(programs::exits(1, 0));
    assert_eq!(core.exit_code(1), Some(0));
}

#[cfg(unix)]
#[test]
fn a_session_gets_exactly_the_requests_environment() {
    let mut dir = TempDir::new("env");
    // Set on the holder itself: the program must not see it.
    start(
        Path::new(BIN),
        "hold",
        &dir.path,
        &[("BRINDLE_HOLDER_OWN", "leak")],
    )
    .unwrap();
    let mut core = TestCore::connect_to(&dir);
    dir.track(core.hello(1).welcome().pid);

    let mut request = spawn(1, "env", &[]);
    request.env.insert("FOO".into(), "2".into());
    request
        .env
        .insert("HOME".into(), "/nonexistent-home".into());
    request.cwd = Some("/".into());
    let expected: Vec<String> = request
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    core.spawn(request);
    let (output, code) = core.output_and_exit(1);
    assert_eq!(code, Some(0));
    let mut lines: Vec<String> = String::from_utf8_lossy(&output)
        .split("\r\n")
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    lines.sort();
    assert_eq!(lines, expected);
}

#[test]
fn a_missing_command_fails_naming_the_program() {
    #[cfg(unix)]
    let (program, reason) = ("/nonexistent/brindle-no-such-program", "No such file");
    #[cfg(windows)]
    let (program, reason) = (r"C:\nonexistent\brindle-no-such-program.exe", "cannot find");
    let (_dir, mut core, _) = holder("missing");
    core.send(ToHolder::Spawn(spawn(4, program, &[])));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 4);
            assert!(
                failed.message.contains(program) && failed.message.contains(reason),
                "{}",
                failed.message
            );
        }
        other => panic!("{other:?}"),
    }
    // The holder still works.
    core.spawn(programs::exits(5, 0));
    assert_eq!(core.exit_code(5), Some(0));
}

/// A holder whose only use was a program it couldn't start stays while its
/// core is connected, and exits once the core has gone, without waiting out
/// its 10 s `UNUSED_LIMIT` (so `brindle-remote stop` leaves nothing running
/// after an agent profile whose command doesn't exist).
#[test]
fn a_holder_whose_spawn_failed_exits_once_its_core_goes() {
    #[cfg(unix)]
    let program = "/nonexistent/brindle-no-such-program";
    #[cfg(windows)]
    let program = r"C:\nonexistent\brindle-no-such-program.exe";
    let (dir, mut core, pid) = holder("failed-gone");
    core.send(ToHolder::Spawn(spawn(4, program, &[])));
    assert!(matches!(core.recv(), ToCore::SpawnFailed(_)));
    assert!(core.quiet_for(Duration::from_millis(200)) && alive(pid), "kept while its core is connected");
    drop(core);
    // Well inside the 10 s limit an unused holder waits.
    assert!(eventually(Duration::from_secs(5), || !alive(pid) && dir.holder_endpoints().is_empty()));
}

#[test]
fn exit_statuses_are_reported_and_an_empty_holder_exits() {
    let (dir, mut core, pid) = holder("exit");
    core.spawn(programs::exits_after_a_line(1, 3));
    #[cfg(unix)]
    {
        core.spawn(shell(2, "kill -TERM $$"));
        assert_eq!(core.exit_code(2), None, "killed by a signal");
    }
    // Windows has no signals: a terminated program has an exit code too,
    // the one it was terminated with.
    #[cfg(windows)]
    {
        let program = core.spawn(programs::exits_after_a_line(2, 0));
        kill(program);
        assert_eq!(core.exit_code(2), Some(1), "terminated");
    }
    assert!(alive(pid), "one session is left");
    core.input(1, programs::ENTER);
    assert_eq!(core.exit_code(1), Some(3));
    assert!(core.closes());
    assert!(holder_exits(&dir, pid));
    assert!(
        !dir.path
            .read_dir()
            .unwrap()
            .flatten()
            .any(|entry| { entry.file_name().to_string_lossy().ends_with(".lock") })
    );
}

/// Reads at most `chunk` bytes per call, pausing before each: a core
/// slower than the program's output.
struct Throttled<R> {
    inner: R,
    chunk: usize,
    pause: Duration,
}

impl<R: std::io::Read> std::io::Read for Throttled<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        throttle(self.pause);
        let len = buffer.len().min(self.chunk);
        self.inner.read(&mut buffer[..len])
    }
}

/// A flood followed by the program's exit, read by a core much slower than
/// the holder's queue fills: the holder must deliver every byte and the
/// `EXITED` before it exits, however long that takes (within its bound),
/// and leave without resetting the connection while the core still writes.
#[test]
fn an_exiting_holder_delivers_a_flood_and_its_exit_to_a_slow_core() {
    let mut dir = TempDir::new("flood");
    start_holder(&dir.path);
    let endpoint = dir.holder_endpoints().remove(0);
    let mut stream = connect(&endpoint).unwrap();
    let send = |stream: &mut Stream, message: ToHolder| {
        message.encode().unwrap().write_to(stream).unwrap()
    };
    send(
        &mut stream,
        ToHolder::Hello(h1::Hello {
            min: h1::PROTOCOLS.min,
            max: h1::PROTOCOLS.max,
            core_build: "test-core".into(),
            instance: 1,
        }),
    );
    send(&mut stream, ToHolder::Spawn(programs::flood(1, &dir.path)));
    // About 1.3 MB/s: the flood takes about 5 s to read, longer than the
    // holder used to wait.
    let mut reader = std::io::BufReader::with_capacity(
        32 * 1024,
        Throttled {
            inner: stream.try_clone().unwrap(),
            chunk: 32 * 1024,
            pause: Duration::from_millis(25),
        },
    );
    let mut output = Vec::new();
    let mut code = None;
    let mut last_poke = std::time::Instant::now();
    loop {
        let frame = match Frame::read_from_or_eof(&mut reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => panic!("after {} bytes of output: {error}", output.len()),
        };
        match ToCore::decode(&frame).unwrap() {
            ToCore::Welcome(welcome) => dir.track(welcome.pid),
            ToCore::Spawned(_) => {}
            ToCore::Output(data) => {
                assert_eq!(code, None, "output after EXITED");
                output.extend(data.bytes);
            }
            ToCore::Exited(exited) => {
                assert_eq!(exited.session, 1);
                code = Some(exited.code);
            }
            other => panic!("{other:?}"),
        }
        // A core keeps talking to its holder (pauses, resumes,
        // checkpoints); an unknown session isn't an error.
        if last_poke.elapsed() >= Duration::from_millis(100) {
            last_poke = std::time::Instant::now();
            let _ = ToHolder::Resume(h1::SessionRef { session: 99 })
                .encode()
                .unwrap()
                .write_to(&mut stream);
        }
    }
    programs::check_flood(&output);
    assert_eq!(code, Some(Some(0)), "the exit, with its status");
    // The holder closed its side; once the core closes too, it is gone.
    drop(reader);
    drop(stream);
    assert!(
        eventually(TIMEOUT, || dir.holder_endpoints().is_empty()
            && server_processes(&dir.path, "hold").is_empty()),
        "the holder didn't exit"
    );
    let logs = dir.logs();
    assert!(!logs.contains("didn't take"), "{logs}");
}

#[cfg(unix)]
#[test]
fn input_resize_and_output_reach_the_program() {
    let (_dir, mut core, _) = holder("io");
    core.spawn(shell(1, "read line; stty size; echo \"got $line\"; exit 0"));
    core.send(ToHolder::Resize(h1::Resize {
        session: 1,
        cols: 100,
        rows: 30,
        cell_w: 8,
        cell_h: 16,
    }));
    core.input(1, b"hello\n");
    let output = core.output_until(1, b"got hello");
    assert!(
        contains(&output, b"30 100"),
        "{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(core.exit_code(1), Some(0));
}

#[cfg(unix)]
#[test]
fn ptys_are_utf8() {
    let (_dir, mut core, _) = holder("iutf8");
    core.spawn(shell(1, "stty -a; echo done.; exit 0"));
    let output = String::from_utf8_lossy(&core.output_until(1, b"done.")).into_owned();
    assert!(
        output.split_whitespace().any(|flag| flag == "iutf8"),
        "{output}"
    );
    assert_eq!(core.exit_code(1), Some(0));
}

#[cfg(unix)]
#[test]
fn hangup_kills_the_process_group() {
    let (_dir, mut core, holder_pid) = holder("hangup");
    // Started first, so the holder isn't left empty (and exits) between the two.
    let plain = core.spawn(shell(2, "sleep 1000"));
    // Ignores SIGHUP, so it takes the SIGKILL after the grace period.
    let pid = core.spawn(shell(1, "trap '' HUP; sleep 1000 & echo \"bg=$!.\"; wait"));
    let output = String::from_utf8_lossy(&core.output_until(1, b".")).into_owned();
    let background: u32 = output
        .split("bg=")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no pid in {output:?}"));
    assert_eq!(
        parent(pid),
        Some(holder_pid),
        "the holder is the program's parent"
    );
    assert!(alive(background));

    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
    assert!(eventually(TIMEOUT, || !alive(pid) && !alive(background)));

    // A program that doesn't ignore it goes at once.
    core.hang_up(2);
    assert_eq!(core.exit_code(2), None);
    assert!(!alive(plain));
}

#[cfg(unix)]
#[test]
fn hangup_kills_a_child_that_outlives_its_exited_leader() {
    let (_dir, mut core, _) = holder("hangup-orphan");
    // The leader goes on SIGHUP; its child ignores it and stays in the group.
    // The child reports its own pid, and only once it ignores the signal: a
    // pid the leader printed could be hung up before the child's trap ran.
    let pid = core.spawn(shell(
        1,
        "(trap '' HUP; exec sh -c 'echo \"bg=$$.\"; exec sleep 1000') & wait",
    ));
    let background = reported_pid(&core.output_until(1, b"."), "bg");
    assert!(alive(background));

    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
    assert!(!alive(pid), "the leader is reaped");
    assert!(alive(background), "the child outlived its leader");
    assert!(
        eventually(TIMEOUT, || !alive(background)),
        "the child survived the hang-up"
    );
}

/// A holder that ignores SIGHUP, as one started under `nohup` does, must not
/// pass that on: an exec keeps ignored signals ignored, and a hang-up would
/// then end its programs only by the SIGKILL that follows the grace period.
/// The same goes for a signal the holder was started with blocked.
#[cfg(unix)]
#[test]
fn programs_start_with_default_signals_under_a_holder_that_ignores_sighup() {
    let mut dir = TempDir::new("nohup");
    start_with(
        Path::new(BIN),
        "hold",
        &dir.path,
        &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")],
        || {
            // SAFETY: async-signal-safe calls, on a set that `sigemptyset`
            // fills first.
            unsafe {
                libc::signal(libc::SIGHUP, libc::SIG_IGN);
                let mut blocked = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
                libc::sigemptyset(blocked.as_mut_ptr());
                libc::sigaddset(blocked.as_mut_ptr(), libc::SIGUSR1);
                libc::sigprocmask(libc::SIG_BLOCK, blocked.as_ptr(), std::ptr::null_mut());
            }
            Ok(())
        },
    )
    .unwrap();
    let mut core = TestCore::connect_to(&dir);
    let holder_pid = core.hello(1).welcome().pid;
    dir.track(holder_pid);
    // Whether `pid` ignores SIGHUP, and whether it blocks SIGUSR1.
    let inherited = |pid: u32| {
        let has = |name: &str, signal: libc::c_int| {
            signal_set(pid, name).map(|set| set & (1 << (signal - 1)) != 0)
        };
        (has("SigIgn", libc::SIGHUP), has("SigBlk", libc::SIGUSR1))
    };
    assert_eq!(
        inherited(holder_pid),
        (Some(true), Some(true)),
        "the holder"
    );

    // `cat` waits to open a FIFO that nobody writes to, so only a signal
    // ends it, not the terminal's closing (see
    // `a_program_reading_its_terminal_is_hung_up_by_sighup`).
    let fifo = dir.path.join("never-written");
    programs::make_fifo(&fifo);
    let pid = core.spawn(spawn(1, "cat", &[fifo.to_str().unwrap()]));
    // Should a check below fail: a `cat` that ignores SIGHUP outlives its
    // holder and waits forever.
    dir.track(pid);
    // A forked child is the holder's own image until its exec.
    assert!(eventually(TIMEOUT, || program_of(pid).as_deref() == Some("cat")));
    // Only the two the holder was given: what else a program ignores is its
    // own doing (one built on Rust's runtime ignores SIGPIPE).
    assert_eq!(inherited(pid), (Some(false), Some(false)), "the program");

    // What a client's `CLOSE` makes the core send.
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None, "a signal ended it");
    // SIGHUP did, at once: not the SIGKILL the holder sends a second later.
    let ended_by = format!("session 1: killed by signal Some({})", libc::SIGHUP);
    let logs = dir.logs();
    assert!(logs.contains(&ended_by), "{logs}");
}

/// A hang-up signals before it closes the terminal, so a program blocked
/// reading it dies of `SIGHUP` every time. Closing first, the failed read
/// (EIO) now and then reached `cat` before the signal, and it exited 1:
/// about once in 4,000 hang-ups under a loaded test run, and never in 8,000
/// on an idle machine, so these rounds check the outcome rather than catch
/// the old order.
#[cfg(unix)]
#[test]
fn a_program_reading_its_terminal_is_hung_up_by_sighup() {
    const ROUNDS: u32 = 200;
    let (dir, mut core, _) = holder("sighup-first");
    // Whether `pid` is `cat`, asleep: blocked reading its terminal.
    let blocked = |pid: u32| {
        program_of(pid).as_deref() == Some("cat")
            && std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|stat| stat.rsplit_once(") ").map(|(_, rest)| rest.starts_with('S')))
                .unwrap_or(false)
    };
    for session in 1..=ROUNDS {
        let pid = core.spawn(spawn(session, "cat", &[]));
        assert!(eventually(TIMEOUT, || blocked(pid)), "round {session}: never blocked");
        // SIGHUP, not the SIGKILL that would follow it, ends it: the
        // holder's log says which signal it was, counted below.
        core.hang_up(session);
        assert_eq!(core.exit_code(session), None, "round {session}: not ended by a signal");
    }
    let sighup = format!(": killed by signal Some({})", libc::SIGHUP);
    let logs = dir.logs();
    assert_eq!(logs.matches(&sighup).count(), ROUNDS as usize, "{logs}");
}

/// `SPAWN`'s meta, decoded from JSON as a core would send it.
fn meta() -> h1::Spawn {
    let frame = Frame::new(
        h1::MSG_SPAWN,
        br#"{"session":0,"cols":1,"rows":1,"meta":{"owner":"t1","policy":"kill"}}"#.to_vec(),
    )
    .unwrap();
    match ToHolder::decode(&frame).unwrap() {
        ToHolder::Spawn(spawn) => spawn,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_new_core_gets_the_sessions_checkpoints_and_tails() {
    let (dir, mut first, holder_pid) = holder("adopt");
    let go = dir.path.join("go");
    let mut request = programs::echoes_a_line(1, &go);
    request.meta = meta().meta;
    let pid = first.spawn(request.clone());
    first.output_until(1, b"one\r\n");

    // A newer HELLO takes over while the first core is still connected.
    let mut second = TestCore::connect_to(&dir);
    let adopted = second.hello(2);
    assert!(first.closes(), "the older core is dropped");
    let welcome = adopted.welcome();
    assert_eq!(
        (welcome.previous, welcome.pid),
        (Previous::Crash, holder_pid)
    );
    assert_eq!(welcome.sessions.len(), 1);
    let held = &welcome.sessions[0];
    assert_eq!(
        (held.session, held.pid, held.cols, held.rows),
        (1, pid, 80, 24)
    );
    assert_eq!((held.exit, held.truncated), (None, false));
    assert_eq!(held.meta, request.meta);
    assert_eq!(adopted.checkpoints[&1], b"");
    // The tail is the output so far. A pseudoconsole adds sequences of its
    // own around the program's.
    let tail = &adopted.tails[&1];
    #[cfg(unix)]
    assert_eq!(tail, b"one\r\n");
    assert!(contains(tail, b"one\r\n"), "{tail:?}");

    // A checkpoint replaces the tail it covers ("one\r\n").
    for message in ToHolder::checkpoint(1, b"CP1", tail.len() as u64) {
        second.send(message);
    }
    second.input(1, b"abc");
    second.input(1, programs::ENTER);
    second.output_until(1, b"two:abc\r\n");
    second.send(ToHolder::Goodbye(Goodbye { handover: true }));
    drop(second);

    let mut third = TestCore::connect_to(&dir);
    let adopted = third.hello(3);
    assert_eq!(adopted.welcome().previous, Previous::Handover);
    assert_eq!(adopted.checkpoints[&1], b"CP1");
    let tail = &adopted.tails[&1];
    assert!(
        contains(tail, b"two:abc\r\n") && !contains(tail, b"one"),
        "{tail:?}"
    );

    // An exit while no core is connected is reported in the next WELCOME.
    // The program is let go (`programs::release`) once the core has gone. Input
    // would need a core to carry it, and the exit it causes can reach the
    // holder before that core's disconnect does: the holder then reports
    // the exit to that core and, left without sessions, exits.
    drop(third);
    programs::release(&go);
    assert!(eventually(TIMEOUT, || !alive(pid)));
    let mut fourth = TestCore::connect_to(&dir);
    let adopted = fourth.hello(4);
    match adopted.welcome().sessions[0].exit {
        Some(exit) => assert_eq!(exit.code, Some(0)),
        // Reaped, but its output was still being drained.
        None => assert_eq!(fourth.exit_code(1), Some(0)),
    }
    assert!(fourth.closes(), "the holder had nothing left");
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn set_meta_replaces_what_the_next_core_gets() {
    let (dir, mut first, _) = holder("set-meta");
    const { assert!(h1::PROTOCOLS.max >= 2, "SET_META is H1 version 2") };
    let mut request = programs::exits_after_a_line(1, 0);
    request.meta = meta().meta;
    first.spawn(request);
    let updated = h1::SessionMeta {
        policy: brindle_remote_protocol::client::Policy::Detach,
        ..h1::SessionMeta::default()
    }
    .to_value();
    first.send(ToHolder::SetMeta(h1::SetMeta {
        session: 1,
        meta: updated.clone(),
    }));
    // An unknown session is no error: it may just have ended.
    first.send(ToHolder::SetMeta(h1::SetMeta {
        session: 99,
        meta: updated.clone(),
    }));
    first.input(1, b"");
    drop(first);

    let mut second = TestCore::connect_to(&dir);
    let adopted = second.hello(2);
    let welcome = adopted.welcome();
    assert_eq!(welcome.protocol, h1::PROTOCOLS.max);
    assert_eq!(welcome.sessions[0].meta, updated);
    second.input(1, programs::ENTER);
}

/// How many bytes wait in `stream`, unread.
#[cfg(unix)]
fn unread(stream: &std::os::unix::net::UnixStream) -> libc::c_int {
    use std::os::fd::AsRawFd;

    let mut waiting: libc::c_int = 0;
    // SAFETY: FIONREAD writes one int, and the fd is open.
    let result = unsafe { libc::ioctl(stream.as_raw_fd(), libc::FIONREAD, &mut waiting) };
    assert_eq!(result, 0);
    waiting
}

#[cfg(unix)]
#[test]
fn what_a_core_sent_before_it_went_counts_while_the_holder_had_output_for_it() {
    use std::os::unix::net::UnixStream;

    let (dir, mut first, holder_pid) = holder("write-fail");
    // It prints without pause, so the holder always has output to send.
    first.spawn(shell(1, "stty raw -echo; exec yes"));

    // The next core never reads. Once its socket is full, the holder has
    // output queued for it, which it tries to send at each turn.
    let mut silent = UnixStream::connect(&dir.holder_endpoints()[0]).unwrap();
    let send = |stream: &mut UnixStream, message: ToHolder| {
        message.encode().unwrap().write_to(stream).unwrap()
    };
    send(
        &mut silent,
        ToHolder::Hello(h1::Hello {
            min: h1::PROTOCOLS.min,
            max: h1::PROTOCOLS.max,
            core_build: "silent".into(),
            instance: 2,
        }),
    );
    assert!(first.closes(), "the holder kept its first core");
    let mut before = 0;
    assert!(
        eventually(TIMEOUT, || {
            let now = unread(&silent);
            let full = now >= 64 * 1024 && now == before;
            before = now;
            full
        }),
        "the holder's output never backed up"
    );

    // Its last message, and then it is gone, like a core killed right
    // after telling a client about an attach. The holder doesn't run in
    // between (as on a busy machine), so when it does, the write it tries
    // first fails: the message in its socket must still be read.
    let signal = |signal| unsafe { libc::kill(holder_pid as libc::pid_t, signal) };
    assert_eq!(signal(libc::SIGSTOP), 0);
    assert!(eventually(TIMEOUT, || state(holder_pid) == Some('T')));
    let updated = h1::SessionMeta {
        owner: "b".into(),
        ..h1::SessionMeta::default()
    }
    .to_value();
    send(
        &mut silent,
        ToHolder::SetMeta(h1::SetMeta {
            session: 1,
            meta: updated.clone(),
        }),
    );
    drop(silent);
    assert_eq!(signal(libc::SIGCONT), 0);

    let mut last = TestCore::connect_to(&dir);
    let adopted = last.hello(3);
    assert_eq!(adopted.welcome().sessions[0].meta, updated);
    last.hang_up(1);
}

/// Sets a socket's send buffer to `bytes`, or to as much as the system lets
/// it have, and returns the size it got. Root may pass the system's limit
/// (`net.core.wmem_max`).
#[cfg(unix)]
fn set_send_buffer(stream: &std::os::unix::net::UnixStream, bytes: usize) -> usize {
    use std::os::fd::AsRawFd;

    let fd = stream.as_raw_fd();
    let wanted = bytes as libc::c_int;
    let size = size_of::<libc::c_int>() as libc::socklen_t;
    let mut got: libc::c_int = 0;
    let mut got_size = size;
    // SAFETY: each option takes or gives one int, and the fd is open.
    unsafe {
        let value = (&raw const wanted).cast();
        if libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUFFORCE, value, size) != 0 {
            let set = libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF, value, size);
            assert_eq!(set, 0, "{}", std::io::Error::last_os_error());
        }
        let value = (&raw mut got).cast();
        let read = libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF, value, &mut got_size);
        assert_eq!(read, 0, "{}", std::io::Error::last_os_error());
    }
    // The kernel reports twice what it was asked for, to count its own
    // bookkeeping.
    got as usize / 2
}

/// A core's last message can lie behind more than the holder reads in one
/// turn (1 MiB): everything the core sent before it went is in the holder's
/// socket, up to the socket's send buffer. The buffer is that big only where
/// the system allows it (for root, or with a raised `net.core.wmem_max`).
/// Elsewhere this still checks that a message behind a full buffer counts.
#[cfg(unix)]
#[test]
fn everything_a_core_sent_before_it_went_counts_however_much_it_was() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    /// The send buffer to ask for. The kernel gives twice that, to count its
    /// own bookkeeping, which is still less than the holder reads from a
    /// core that is gone (16 MiB).
    const BUFFER: usize = 6 * 1024 * 1024;
    /// What is kept free of the buffer for the last message.
    const RESERVE: usize = 64 * 1024;

    let (dir, mut first, holder_pid) = holder("last-words");
    // It prints without pause, so the holder always has output to send.
    first.spawn(shell(1, "stty raw -echo; exec yes"));

    // The next core never reads. Once its socket is full, the holder has
    // output queued for it, which it tries to send at each turn.
    let mut silent = UnixStream::connect(&dir.holder_endpoints()[0]).unwrap();
    let bytes = |message: ToHolder| {
        let mut bytes = Vec::new();
        message.encode().unwrap().write_to(&mut bytes).unwrap();
        bytes
    };
    let hello = ToHolder::Hello(h1::Hello {
        min: h1::PROTOCOLS.min,
        max: h1::PROTOCOLS.max,
        core_build: "silent".into(),
        instance: 2,
    });
    silent.write_all(&bytes(hello)).unwrap();
    assert!(first.closes(), "the holder kept its first core");
    let mut before = 0;
    assert!(
        eventually(TIMEOUT, || {
            let now = unread(&silent);
            let full = now >= 64 * 1024 && now == before;
            before = now;
            full
        }),
        "the holder's output never backed up"
    );

    // The holder doesn't run while the core sends all this and goes, as on
    // a busy machine. When it runs again, the write it tries first fails.
    let signal = |signal| unsafe { libc::kill(holder_pid as libc::pid_t, signal) };
    assert_eq!(signal(libc::SIGSTOP), 0);
    assert!(eventually(TIMEOUT, || state(holder_pid) == Some('T')));

    // Messages the holder has no use for (input for a session it doesn't
    // have), until the socket takes no more. One this small is taken whole
    // or not at all.
    let buffer = set_send_buffer(&silent, BUFFER);
    assert!(buffer > 2 * RESERVE, "a send buffer of {buffer} bytes");
    set_send_buffer(&silent, buffer - RESERVE);
    silent.set_nonblocking(true).unwrap();
    let filler = bytes(ToHolder::Input(h1::Data {
        session: 99,
        bytes: vec![b'.'; 8 * 1024],
    }));
    let mut sent = 0;
    loop {
        match silent.write(&filler) {
            Ok(written) => assert_eq!(written, filler.len(), "part of a message"),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("{error}"),
        }
        sent += filler.len();
    }
    assert!(sent >= 64 * 1024, "only {sent} bytes fit");
    // Its last message goes behind them, in the room kept for it.
    let updated = h1::SessionMeta {
        owner: "b".into(),
        ..h1::SessionMeta::default()
    }
    .to_value();
    let last_words = bytes(ToHolder::SetMeta(h1::SetMeta {
        session: 1,
        meta: updated.clone(),
    }));
    set_send_buffer(&silent, buffer);
    assert_eq!(silent.write(&last_words).unwrap(), last_words.len());
    drop(silent);
    assert_eq!(signal(libc::SIGCONT), 0);

    let mut last = TestCore::connect_to(&dir);
    let adopted = last.hello(3);
    assert_eq!(
        adopted.welcome().sessions[0].meta,
        updated,
        "the last of {sent} bytes"
    );
    last.hang_up(1);
}

#[cfg(unix)]
#[test]
fn a_checkpoint_keeps_the_output_it_doesnt_cover() {
    let (dir, mut core, _) = holder("covers");
    core.spawn(spawn(1, "cat", &[]));
    core.input(1, b"a\n");
    core.output_until(1, b"a\r\na\r\n");
    // As if the core had rendered after the echo but before cat's line.
    for message in ToHolder::checkpoint(1, b"CP", 3) {
        core.send(message);
    }
    core.input(1, b"b\n");
    core.output_until(1, b"b\r\nb\r\n");
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.checkpoints[&1], b"CP");
    assert_eq!(adopted.tails[&1], b"a\r\nb\r\nb\r\n");
    assert!(!adopted.welcome().sessions[0].truncated);
    core.hang_up_and_wait(1);
}

#[test]
fn a_tail_past_its_limit_is_truncated() {
    let (dir, mut core, _) = holder("truncate");
    core.spawn(programs::overflows_the_tail(1, &dir.path));
    core.output_until(1, b"END\r\n");
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert!(adopted.welcome().sessions[0].truncated);
    let tail = &adopted.tails[&1];
    assert_eq!(tail.len() as u64, h1::TAIL_LIMIT);
    #[cfg(unix)]
    assert!(tail.ends_with(b"x\r\nEND\r\n") && tail.starts_with(b"xxx"));
    // The newest output is what is kept.
    assert!(contains(&tail[tail.len() - 4096..], b"END\r\n"));

    // A checkpoint clears the flag.
    for message in ToHolder::checkpoint(1, b"CP", h1::TAIL_LIMIT) {
        core.send(message);
    }
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(3);
    assert!(!adopted.welcome().sessions[0].truncated);
    #[cfg(unix)]
    assert_eq!(adopted.tails[&1], b"");
    // At most what the waiting program's console still had to say.
    assert!(adopted.tails[&1].len() < 4096);
    core.hang_up(1);
    core.hung_up(1);
}

/// A core that opens more checkpoints at once than H1 allows
/// (`MAX_OPEN_BLOBS`) gets an `ERROR` for the one past the bound, and
/// nothing else changes: the session runs on, and the next core gets the
/// checkpoint it had before.
#[test]
fn a_core_past_the_open_blob_limit_gets_an_error_and_the_sessions_keep_running() {
    let (dir, mut core, holder_pid) = holder("open-blobs");
    let go = dir.path.join("go");
    let pid = core.spawn(programs::echoes_a_line(1, &go));
    let seen = core.output_until(1, b"one\r\n");
    for message in ToHolder::checkpoint(1, b"CP", seen.len() as u64) {
        core.send(message);
    }

    // Blobs begun for sessions the holder doesn't have and never ended,
    // then one for the session it has, past the bound.
    for session in 100..100 + h1::MAX_OPEN_BLOBS as u32 {
        core.send(ToHolder::CheckpointBegin(h1::BlobBegin { session, len: 4 }));
    }
    core.send(ToHolder::CheckpointBegin(h1::BlobBegin {
        session: 1,
        len: 4,
    }));
    match core.recv_skipping_output() {
        ToCore::Error(error) => {
            assert_eq!(error.session, Some(1), "{}", error.message);
            let limit = format!("the limit is {}", h1::MAX_OPEN_BLOBS);
            assert!(error.message.contains(&limit), "{}", error.message);
        }
        other => panic!("expected ERROR for session 1, got {other:?}"),
    }

    // The session still answers.
    core.input(1, b"abc");
    core.input(1, programs::ENTER);
    core.output_until(1, b"two:abc\r\n");

    // A core that reconnects adopts it with the checkpoint it had.
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    let welcome = adopted.welcome();
    assert_eq!(
        (welcome.pid, welcome.previous),
        (holder_pid, Previous::Crash)
    );
    assert_eq!(welcome.sessions.len(), 1);
    let held = &welcome.sessions[0];
    assert_eq!((held.session, held.pid, held.exit), (1, pid, None));
    assert_eq!(adopted.checkpoints[&1], b"CP");

    programs::release(&go);
    assert_eq!(core.exit_code(1), Some(0));
}

#[cfg(unix)]
#[test]
fn without_a_core_a_program_blocks_instead_of_losing_output() {
    let (dir, mut core, holder_pid) = holder("block");
    let lines = 300_000;
    let pid = core.spawn(shell(
        1,
        &format!("read go; seq 1 {lines}; echo DONE; read z"),
    ));
    // The holder handles the input before it sees the connection end.
    core.input(1, b"go\n");
    drop(core);

    // seq stays blocked on the full PTY once the holder stops reading.
    let mut seq = Vec::new();
    assert!(eventually(TIMEOUT, || {
        seq = children_named(pid, "seq");
        !seq.is_empty()
    }));
    let blocked = stays_blocked(seq[0], holder_pid);
    assert!(alive(seq[0]), "seq finished, so output went somewhere");
    assert!(blocked, "seq never stopped: the holder kept reading");

    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.welcome().previous, Previous::Crash);
    assert!(!adopted.welcome().sessions[0].truncated);
    let mut stream = adopted.tails[&1].clone();
    assert_eq!(stream.len() as u64, h1::TAIL_LIMIT, "the tail filled up");
    stream.extend(core.output_until(1, b"DONE\r\n"));

    let mut expected = b"go\r\n".to_vec();
    for line in 1..=lines {
        expected.extend(format!("{line}\r\n").as_bytes());
    }
    expected.extend(b"DONE\r\n");
    assert!(stream.len() >= expected.len());
    assert!(
        stream[..expected.len()] == expected[..],
        "the output has a gap or is out of order"
    );
    core.hang_up_and_wait(1);
    assert!(holder_exits(&dir, holder_pid));
}

#[cfg(unix)]
#[test]
fn a_paused_session_blocks_until_resumed() {
    let (_dir, mut core, holder_pid) = holder("pause");
    let pid = core.spawn(shell(1, "read go; seq 1 300000; echo DONE; read z"));
    core.send(ToHolder::Pause(h1::SessionRef { session: 1 }));
    core.input(1, b"go\n");
    let mut seq = Vec::new();
    assert!(eventually(TIMEOUT, || {
        seq = children_named(pid, "seq");
        !seq.is_empty()
    }));
    let blocked = stays_blocked(seq[0], holder_pid);
    assert!(alive(seq[0]), "seq finished, so the PTY was read");
    assert!(blocked, "seq never stopped: the PTY was read");

    core.send(ToHolder::Resume(h1::SessionRef { session: 1 }));
    let output = core.output_until(1, b"DONE\r\n");
    assert!(contains(&output, b"\r\n300000\r\n"));
    core.hang_up_and_wait(1);
}

/// The marker of the holder's fault switch in these tests: a `SPAWN` with
/// it in an argument, or `INPUT` with it in its bytes, fails in the holder.
const FAULT: &str = "brindle-test-holder-fault";

/// A holder started with [`FAULT`] as its fault switch's marker, with a
/// connected test core.
fn faulty_holder(tag: &str) -> (TempDir, TestCore, u32) {
    let mut dir = TempDir::new(tag);
    start_holder_with(&dir.path, &[("BRINDLE_TEST_HOLDER_PANIC", FAULT)]);
    let mut core = TestCore::connect_to(&dir);
    let pid = core.hello(1).welcome().pid;
    dir.track(pid);
    (dir, core, pid)
}

/// Sends the line `word` to an [`programs::echoes_a_line`] session and
/// waits for its answer.
fn answers(core: &mut TestCore, session: u32, word: &str) {
    core.input(session, word.as_bytes());
    core.input(session, programs::ENTER);
    core.output_until(session, format!("two:{word}\r\n").as_bytes());
}

#[test]
fn a_holder_fault_while_spawning_fails_that_spawn_only() {
    if skip_without_switch("BRINDLE_TEST_HOLDER_PANIC") {
        return;
    }
    let (dir, mut core, holder_pid) = faulty_holder("fault-spawn");
    let go = dir.path.join("go");
    let pid = core.spawn(programs::echoes_a_line(1, &go));
    core.output_until(1, b"one\r\n");

    let mut faulty = programs::exits(2, 0);
    faulty.args.push(FAULT.into());
    core.send(ToHolder::Spawn(faulty));
    match core.recv_skipping_output() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 2, "{}", failed.message);
            assert!(
                failed.message.contains("BRINDLE_TEST_HOLDER_PANIC matched"),
                "{}",
                failed.message
            );
        }
        other => panic!("expected SPAWN_FAILED for session 2, got {other:?}"),
    }

    // The holder, the other session and the connection carry on, and the
    // failed spawn left nothing behind under its id.
    assert!(alive(holder_pid));
    assert!(alive(pid));
    answers(&mut core, 1, "abc");
    core.spawn(programs::exits(2, 0));
    assert_eq!(core.exit_code(2), Some(0));
    programs::release(&go);
    assert_eq!(core.exit_code(1), Some(0));
}

#[test]
fn a_holder_fault_on_input_reports_an_error_and_both_sessions_keep_running() {
    if skip_without_switch("BRINDLE_TEST_HOLDER_PANIC") {
        return;
    }
    let (dir, mut core, holder_pid) = faulty_holder("fault-input");
    let gos = [dir.path.join("go1"), dir.path.join("go2")];
    // Each session's `one` before the next spawn, which drops output.
    let mut pids = Vec::new();
    for (session, go) in [(1, &gos[0]), (2, &gos[1])] {
        pids.push(core.spawn(programs::echoes_a_line(session, go)));
        core.output_until(session, b"one\r\n");
    }

    core.input(1, FAULT.as_bytes());
    match core.recv_skipping_output() {
        ToCore::Error(error) => {
            assert_eq!(error.session, Some(1), "{}", error.message);
            assert!(error.message.contains("INPUT"), "{}", error.message);
            assert!(
                error.message.contains("BRINDLE_TEST_HOLDER_PANIC matched"),
                "{}",
                error.message
            );
        }
        other => panic!("expected ERROR for session 1, got {other:?}"),
    }

    assert!(alive(holder_pid));
    for pid in pids {
        assert!(alive(pid));
    }
    answers(&mut core, 1, "abc");
    answers(&mut core, 2, "def");
    for (session, go) in [(1, &gos[0]), (2, &gos[1])] {
        programs::release(go);
        assert_eq!(core.exit_code(session), Some(0));
    }
}

#[test]
fn a_core_refused_for_its_first_message_reads_the_error_then_the_end() {
    let (dir, _core, holder_pid) = holder("refuse-first");
    let opening = ToHolder::Pause(h1::SessionRef { session: 1 })
        .encode()
        .unwrap();
    let answer = refused(&dir.holder_endpoints()[0], &opening);
    match ToCore::decode(&answer).unwrap() {
        ToCore::Error(error) => {
            assert_eq!(error.session, None);
            assert!(error.message.contains("HELLO"), "{}", error.message);
        }
        other => panic!("expected ERROR, got {other:?}"),
    }
    assert!(alive(holder_pid));
}

#[test]
fn a_core_refused_for_its_versions_reads_the_error_then_the_end() {
    let (dir, _core, holder_pid) = holder("refuse-versions");
    let opening = ToHolder::Hello(h1::Hello {
        min: 900,
        max: 901,
        core_build: "test-core".into(),
        instance: 2,
    })
    .encode()
    .unwrap();
    let answer = refused(&dir.holder_endpoints()[0], &opening);
    match ToCore::decode(&answer).unwrap() {
        ToCore::Error(error) => {
            assert_eq!(error.session, None);
            assert!(
                error.message.contains("no common protocol version"),
                "{}",
                error.message
            );
        }
        other => panic!("expected ERROR, got {other:?}"),
    }
    assert!(alive(holder_pid));
}

#[test]
fn a_drained_holder_refuses_spawn_and_exits_after_its_last_session() {
    let (dir, mut core, holder_pid) = holder("drain");
    core.spawn(programs::exits_after_a_line(1, 0));
    core.send(ToHolder::Drain(Drain {}));
    core.send(ToHolder::Spawn(programs::exits(2, 0)));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 2);
            assert!(failed.message.contains("draining"), "{}", failed.message);
        }
        other => panic!("{other:?}"),
    }
    assert!(alive(holder_pid));
    core.input(1, programs::ENTER);
    assert_eq!(core.exit_code(1), Some(0));
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn a_holder_without_a_core_starts_one_from_its_own_binary_even_after_it_was_replaced() {
    let mut dir = TempDir::new("supervise");
    let bin_dir = TempDir::new("supervise-bin");
    let name = format!("brindle-remote{}", std::env::consts::EXE_SUFFIX);
    let copy = bin_dir.path.join(name);
    fs::copy(BIN, &copy).unwrap();
    // Instead of `serve`, which comes later, the started "core" prints the
    // version of whichever binary ran, into the holder's log.
    start(
        &copy,
        "hold",
        &dir.path,
        &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")],
    )
    .unwrap();
    let mut core = TestCore::connect_to(&dir);
    let holder_pid = core.hello(1).welcome().pid;
    dir.track(holder_pid);
    core.spawn(programs::exits_after_a_line(1, 0));

    // An install replaces the file. (Not on Windows, where a running image
    // can't be replaced: a holder there runs from a copy made for its
    // build, which no install touches.)
    #[cfg(unix)]
    {
        let replacement = bin_dir.path.join("new");
        fs::write(&replacement, "#!/bin/sh\necho replaced binary ran\n").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(&replacement, &copy).unwrap();
    }

    // The core goes away, as if SIGKILLed.
    drop(core);
    let version = env!("CARGO_PKG_VERSION");
    let starts = |logs: &str| logs.lines().filter(|line| *line == version).count();
    assert!(
        eventually(Duration::from_secs(60), || starts(&dir.logs()) >= 2),
        "no repeated core starts: {}",
        dir.logs()
    );
    let logs = dir.logs();
    assert!(!logs.contains("replaced binary ran"), "{logs}");
    #[cfg(unix)]
    let image = format!("/proc/{holder_pid}/exe");
    #[cfg(windows)]
    let image = copy.display().to_string();
    assert!(logs.contains(&format!("started {image} --version")), "{logs}");

    // A core that connects stops the retries and keeps the session.
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.welcome().sessions.len(), 1);
    core.input(1, programs::ENTER);
    assert_eq!(core.exit_code(1), Some(0));
    assert!(holder_exits(&dir, holder_pid));
}

/// The Windows tests of what the Linux-only ones above cover with a Unix
/// shell: the same behaviour, over a pseudoconsole.
#[cfg(windows)]
mod on_windows {
    use std::time::Instant;

    use brindle_remote_protocol::holder::Spawn;

    use super::*;

    /// How long a hung-up program has before the holder terminates it
    /// (`pty::GRACEFUL_SHUTDOWN`).
    const GRACE: Duration = Duration::from_secs(1);
    /// The exit code of a program the holder terminated.
    const TERMINATED: i32 = 1;

    /// An interactive `cmd` whose prompt is `ready>`.
    fn interactive(session: u32) -> Spawn {
        let mut request = spawn(session, "cmd.exe", &["/d"]);
        request.env = inherited_env(&[("PROMPT", "ready$G")]);
        request
    }

    /// A PowerShell session running `script`.
    fn powershell(session: u32, script: &str) -> Spawn {
        let mut request = spawn(session, "powershell.exe", &["-NoProfile", "-Command", script]);
        request.env = inherited_env(&[]);
        request
    }

    /// A round trip: everything the holder sent before it handled this is
    /// read (and dropped) when this returns.
    fn settle(core: &mut TestCore) {
        core.hang_up(99);
        loop {
            match core.recv() {
                ToCore::Error(error) if error.session == Some(99) => return,
                ToCore::Output(_) => {}
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_session_gets_exactly_the_requests_environment() {
        let mut dir = TempDir::new("env");
        // Set on the holder itself: the program must not see it.
        start(
            Path::new(BIN),
            "hold",
            &dir.path,
            &[("BRINDLE_HOLDER_OWN", "leak")],
        )
        .unwrap();
        let mut core = TestCore::connect_to(&dir);
        dir.track(core.hello(1).welcome().pid);

        // Wide enough for every variable to be one line.
        let mut request = spawn(1, "cmd.exe", &["/d", "/c", "set& cd"]);
        request.cols = 200;
        request.env.insert("FOO".into(), "2".into());
        request.env.insert("HOME".into(), r"C:\nonexistent-home".into());
        let cwd = request.env["SystemRoot"].clone();
        request.cwd = Some(cwd.clone());
        let expected: Vec<String> = request
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        core.spawn(request);
        let (output, code) = core.output_and_exit(1);
        assert_eq!(code, Some(0));
        let mut lines = rows(&replayed(&output, 200, 24, 1000));
        lines.retain(|line| !line.is_empty());
        // As the disk spells it, which needn't be how the variable does.
        let shown = lines.pop().unwrap_or_default();
        assert!(shown.eq_ignore_ascii_case(&cwd), "{shown} isn't {cwd}");
        // `cmd` gives itself these three when it has none.
        let own = ["COMSPEC=", "PATHEXT=", "PROMPT="];
        lines.retain(|line| !own.iter().any(|name| line.starts_with(name)));
        lines.sort();
        assert_eq!(lines, expected);
    }

    #[test]
    fn input_resize_and_output_reach_the_program() {
        const SIZE: &[u8] = b"powershell -NoProfile -Command \"$s = $Host.UI.RawUI.WindowSize; \
                              Write-Host ('size=' + $s.Width + 'x' + $s.Height + ';')\"\r";
        let (_dir, mut core, _) = holder("io");
        core.spawn(interactive(1));
        core.output_until(1, b"ready>");
        core.send(ToHolder::Resize(h1::Resize {
            session: 1,
            cols: 100,
            rows: 30,
            cell_w: 8,
            cell_h: 16,
        }));
        // The console applies a resize a moment later.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            core.input(1, SIZE);
            let output = core.output_until(1, b"ready>");
            if contains(&output, b"size=100x30;") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                String::from_utf8_lossy(&output)
            );
        }
        // The caret goes when `cmd` runs the line, so only the program's
        // own output matches.
        core.input(1, b"echo got^ hello\r");
        core.output_until(1, b"got hello");
        core.input(1, b"exit 0\r");
        assert_eq!(core.exit_code(1), Some(0));
    }

    #[test]
    fn hangup_ends_the_program_and_its_consoles_programs() {
        let (_dir, mut core, holder_pid) = holder("hangup");
        // Started first, so the holder isn't left empty (and exits) between the two.
        let plain = core.spawn(programs::exits_after_a_line(2, 0));
        let pid = core.spawn(spawn(1, "cmd.exe", &["/d", "/c", "ping -t 127.0.0.1"]));
        let mut ping = Vec::new();
        assert!(eventually(TIMEOUT, || {
            ping = children_named(pid, "ping");
            !ping.is_empty()
        }));
        assert_eq!(
            parent(pid),
            Some(holder_pid),
            "the holder is the program's parent"
        );

        // Closing the console ends the programs on it, the shell's child
        // too, and the holder ends any the close missed after the grace
        // (`ping` started just as it went out).
        // The close, not the grace's `TerminateProcess`, ended it: that would
        // have given it the termination's code.
        core.hang_up(1);
        assert_ne!(core.exit_code(1), Some(TERMINATED));
        assert!(eventually(TIMEOUT, || !alive(pid) && !alive(ping[0])));

        core.hang_up(2);
        core.hung_up(2);
        assert!(!alive(plain));
    }

    #[test]
    fn a_hangup_as_a_session_starts_reaches_its_program() {
        let (_dir, mut core, _) = holder("hangup-start");
        // Started first, so the holder isn't left empty (and exits) between
        // the rounds.
        let plain = core.spawn(programs::exits_after_a_line(1, 0));
        for session in 2..5 {
            // Right after `SPAWNED`, before `cmd` is on its console.
            let pid = core.spawn(spawn(session, "cmd.exe", &["/d", "/c", "ping -t 127.0.0.1"]));
            core.hang_up(session);
            let code = core.exit_code(session);
            // The close ends what is on the console at once; anything left
            // by the grace never got it.
            let gone = eventually(2 * GRACE, || children_named(pid, "ping").is_empty());
            for ping in children_named(pid, "ping") {
                kill(ping);
            }
            assert!(gone, "session {session}: ping outlived its console");
            assert_ne!(code, Some(TERMINATED), "session {session}: the close never reached it");
        }
        core.hang_up(1);
        core.hung_up(1);
        assert!(!alive(plain));
    }

    #[test]
    fn hangup_terminates_a_program_that_left_its_console() {
        let (dir, mut core, _) = holder("hangup-left");
        let marker = dir.path.join("left");
        // Closing the console doesn't reach a program that is no longer on
        // it, so it takes the termination after the grace period.
        let script = format!(
            "$k = Add-Type -Name Kernel -Namespace BrindleTest -PassThru -MemberDefinition \
             '[DllImport(\"kernel32.dll\")] public static extern bool FreeConsole();'; \
             [void]$k::FreeConsole(); Set-Content -Path '{}' -Value left; \
             Start-Sleep -Seconds 600",
            marker.display()
        );
        let pid = core.spawn(powershell(1, &script));
        assert!(eventually(TIMEOUT, || marker.exists()), "it never left");

        let hung_up_at = Instant::now();
        core.hang_up(1);
        assert_eq!(core.exit_code(1), Some(TERMINATED));
        assert!(hung_up_at.elapsed() >= GRACE, "it had no grace");
        assert!(!alive(pid));
    }

    #[test]
    fn a_checkpoint_keeps_the_output_it_doesnt_cover() {
        let (dir, mut core, _) = holder("covers");
        core.spawn(interactive(1));
        let mut seen = core.output_until(1, b"ready>");
        core.input(1, b"echo a^1\r");
        seen.extend(core.output_until(1, b"a1"));
        // As a core does: the checkpoint covers what it had read when it
        // rendered, and more output may be on its way already.
        for message in ToHolder::checkpoint(1, b"CP", seen.len() as u64) {
            core.send(message);
        }
        core.input(1, b"echo b^2\r");
        let after = core.output_until(1, b"b2");
        drop(core);
        let mut core = TestCore::connect_to(&dir);
        let adopted = core.hello(2);
        assert_eq!(adopted.checkpoints[&1], b"CP");
        let tail = &adopted.tails[&1];
        assert!(
            tail.starts_with(&after),
            "{:?} doesn't start with {:?}",
            String::from_utf8_lossy(tail),
            String::from_utf8_lossy(&after)
        );
        assert!(!adopted.welcome().sessions[0].truncated);
        core.hang_up(1);
        core.hung_up(1);
    }

    /// About 2.9 MB of output: more than a tail and the pipes hold.
    const LINES: usize = 40_000;

    /// Waits for a line, creates the file it returns, prints [`LINES`]
    /// numbered lines and `DONE`, and waits for another line. Once that
    /// file exists, the program has had its line and is printing.
    fn prints_lines_when_told(session: u32, dir: &TempDir) -> (Spawn, std::path::PathBuf) {
        let file = programs::numbered_lines(&dir.path, LINES);
        let printing = dir.path.join("printing");
        let script = format!(
            "set /p go=& type nul>{}& type {}& echo DONE& set /p z=",
            printing.display(),
            file.display()
        );
        (shell(session, &script), printing)
    }

    #[test]
    fn without_a_core_a_program_blocks_instead_of_losing_output() {
        let (dir, mut core, holder_pid) = holder("block");
        let (request, printing) = prints_lines_when_told(1, &dir);
        let pid = core.spawn(request);
        // The holder handles the input before it sees the connection end.
        core.input(1, b"go\r");
        drop(core);

        // `type` is `cmd`'s own, so `cmd` is the writer. Once it prints, it
        // stays blocked when the holder stops reading at a full tail, and
        // doesn't end: nothing takes the rest of its output.
        assert!(eventually(TIMEOUT, || printing.exists()), "it never had its line");
        let blocked = stays_blocked(pid, holder_pid);
        assert!(alive(pid), "it finished, so its output went somewhere");
        assert!(blocked, "it never stopped: the holder kept reading");

        // The holder read until the tail was full, and no further: nothing
        // was cut off its front.
        let mut core = TestCore::connect_to(&dir);
        let adopted = core.hello(2);
        assert_eq!(adopted.welcome().previous, Previous::Crash);
        assert!(!adopted.welcome().sessions[0].truncated);
        let mut stream = adopted.tails[&1].clone();
        assert_eq!(stream.len() as u64, h1::TAIL_LIMIT, "the tail filled up");
        stream.extend(core.output_until(1, b"DONE\r\n"));
        // No gap, and in order.
        programs::check_lines(&stream, LINES);
        core.hang_up(1);
        core.hung_up(1);
        assert!(holder_exits(&dir, holder_pid));
    }

    #[test]
    fn a_paused_session_blocks_until_resumed() {
        let (dir, mut core, _) = holder("pause");
        let (request, _) = prints_lines_when_told(1, &dir);
        let pid = core.spawn(request);
        core.send(ToHolder::Pause(h1::SessionRef { session: 1 }));
        // What the console said before the pause is out of the way.
        settle(&mut core);
        core.input(1, b"go\r");
        assert!(
            core.quiet_for(Duration::from_secs(1)),
            "the session's output was read"
        );
        assert!(alive(pid));

        core.send(ToHolder::Resume(h1::SessionRef { session: 1 }));
        let output = core.output_until(1, b"DONE\r\n");
        programs::check_lines(&output, LINES);
        core.hang_up(1);
        core.hung_up(1);
    }

    #[test]
    fn reparent_makes_a_window_the_owner_of_the_sessions_console_window() {
        // Only the packaged ConPTY can; a build without it uses Windows' own.
        if !Path::new(BIN).with_file_name("conpty.dll").exists() {
            skip(Capability::PackagedConpty, "no conpty.dll beside the helper");
            return;
        }
        let (_dir, mut core, _) = holder("reparent");
        core.spawn(powershell(1, CONSOLE_WINDOW));
        let console_window = wait_for_console_window(|| core.output_until(1, b";"));
        assert_eq!(window_owner(console_window), 0);

        let window = Window::new();
        core.send(ToHolder::Reparent(h1::Reparent {
            session: 1,
            hwnd: window.id(),
        }));
        // The host applies it a moment later.
        let wanted = window.id();
        until(&format!("the console window's owner is {wanted:#x}"), TIMEOUT, || {
            Window::pump();
            (window_owner(console_window) == wanted).then_some(())
        });
        // It has no answer: the next message is the hang-up's.
        core.hang_up(1);
        core.hung_up(1);
    }

    /// A program named without a directory is looked for in the session's
    /// `PATH` only: a `cmd.exe` beside the holder's own image, where
    /// `CreateProcessW` would look first, isn't the one started.
    #[test]
    fn a_bare_program_name_is_looked_up_in_path_not_beside_the_holder() {
        let mut dir = TempDir::new("bare-name");
        let bin_dir = TempDir::new("bare-name-bin");
        let copy = bin_dir.path.join("brindle-remote.exe");
        fs::copy(BIN, &copy).unwrap();
        // The planted `cmd.exe` prints the machine's name, never the line
        // the real one is asked to echo.
        let root = std::env::var("SystemRoot").unwrap();
        fs::copy(format!(r"{root}\System32\hostname.exe"), bin_dir.path.join("cmd.exe")).unwrap();
        start(&copy, "hold", &dir.path, &[("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")]).unwrap();
        let mut core = TestCore::connect_to(&dir);
        let holder_pid = core.hello(1).welcome().pid;
        dir.track(holder_pid);

        core.spawn(shell(1, "echo the-real-cmd-ran"));
        let (output, code) = core.output_and_exit(1);
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("the-real-cmd-ran"), "{output}");
        assert_eq!(code, Some(0), "{output}");
    }

    /// A program named without a directory that no directory of the
    /// session's `PATH` has fails its spawn, naming the program.
    #[test]
    fn a_program_in_no_directory_of_path_fails_its_spawn_naming_it() {
        let (_dir, mut core, holder_pid) = holder("not-in-path");
        core.send(ToHolder::Spawn(spawn(1, "brindle-no-such-program", &[])));
        match core.recv() {
            ToCore::SpawnFailed(failed) => {
                assert_eq!(failed.session, 1, "{}", failed.message);
                assert!(failed.message.contains("brindle-no-such-program"), "{}", failed.message);
            }
            other => panic!("expected SPAWN_FAILED, got {other:?}"),
        }
        assert!(alive(holder_pid));
        // The holder carries on with the next spawn.
        core.spawn(shell(2, "exit 3"));
        assert_eq!(core.exit_code(2), Some(3));
    }
}

/// The pipe sessions' programs (H1 version 3's `SPAWN_PIPES`), as `sh` runs
/// them: no terminal, so their bytes are exact.
#[cfg(unix)]
mod pipe_programs {
    use std::path::Path;

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::holder::SpawnPipes;

    use super::support::pipe_spawn;

    /// How the hang-up ends a program that ignores its stdin's end, as the
    /// holder's log names it.
    pub const ENDED_BY_THE_HANG_UP: Option<i32> = None;

    fn sh(session: SessionId, script: &str) -> SpawnPipes {
        pipe_spawn(session, "/bin/sh", &["-c", script])
    }

    /// Copies its stdin to its stdout, byte for byte, until stdin ends.
    pub fn copies(session: SessionId) -> SpawnPipes {
        pipe_spawn(session, "cat", &[])
    }

    /// Writes `1` to `count`, a line each, to its stderr and exits with 0.
    pub fn counts_on_stderr(session: SessionId, count: u32) -> SpawnPipes {
        sh(session, &format!("seq 1 {count} >&2"))
    }

    /// Reads a line, then writes `1` to `count` to stderr and `done` to
    /// stdout, and exits with 0.
    pub fn counts_on_stderr_after_a_line(session: SessionId, count: u32) -> SpawnPipes {
        sh(session, &format!("read x; seq 1 {count} >&2; echo done"))
    }

    /// Ignores its stdin's end and `SIGHUP`, and starts a child that ignores
    /// them too, printing `bg=<pid>.`.
    pub fn ignores_its_end(session: SessionId) -> SpawnPipes {
        sh(session, "trap '' HUP; sleep 1000 & echo \"bg=$!.\"; wait")
    }

    /// Prints `first`, waits for [`super::programs::release`] through `go`,
    /// prints `second`, makes `printed`, and waits for its stdin's end.
    pub fn prints_twice(session: SessionId, go: &Path, printed: &Path) -> SpawnPipes {
        super::programs::make_fifo(go);
        sh(
            session,
            &format!(
                "exec 3<>'{}'; echo first; read x <&3; echo second; : > '{}'; cat >/dev/null",
                go.display(),
                printed.display()
            ),
        )
    }

    /// Prints `ready` once it has `go` open (a release before that would be
    /// lost), waits for [`super::programs::release`] through it, writes
    /// `bye` to stderr, and exits with 3.
    pub fn exits_when_released(session: SessionId, go: &Path) -> SpawnPipes {
        super::programs::make_fifo(go);
        sh(
            session,
            &format!(
                "exec 3<>'{}'; echo ready; read x <&3; echo bye >&2; exit 3",
                go.display()
            ),
        )
    }

    pub fn release(go: &Path) {
        super::programs::release(go);
    }
}

/// The pipe sessions' programs, as PowerShell (or `cmd`) runs them with
/// their standard streams on pipes.
#[cfg(windows)]
mod pipe_programs {
    use std::path::Path;

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::holder::SpawnPipes;

    use super::support::{inherited_env, pipe_spawn};

    /// How the hang-up ends a program that ignores its stdin's end: its job
    /// is terminated with 1.
    pub const ENDED_BY_THE_HANG_UP: Option<i32> = Some(1);

    pub fn powershell(session: SessionId, script: &str) -> SpawnPipes {
        let mut request = pipe_spawn(
            session,
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", script],
        );
        request.env = inherited_env(&[]);
        request
    }

    /// Copies its stdin to its stdout, byte for byte, until stdin ends.
    pub fn copies(session: SessionId) -> SpawnPipes {
        powershell(
            session,
            "$i = [Console]::OpenStandardInput(); $o = [Console]::OpenStandardOutput(); \
             $i.CopyTo($o); $o.Flush()",
        )
    }

    /// Writes `1` to `count`, a line each (`\n`), to its stderr and exits
    /// with 0.
    pub fn counts_on_stderr(session: SessionId, count: u32) -> SpawnPipes {
        powershell(
            session,
            &format!("[Console]::Error.Write(((1..{count}) -join \"`n\") + \"`n\")"),
        )
    }

    /// Reads a line, then writes `1` to `count` to stderr and `done` to
    /// stdout, and exits with 0.
    pub fn counts_on_stderr_after_a_line(session: SessionId, count: u32) -> SpawnPipes {
        powershell(
            session,
            &format!(
                "$null = [Console]::In.ReadLine(); \
                 [Console]::Error.Write(((1..{count}) -join \"`n\") + \"`n\"); \
                 [Console]::Out.Write('done')"
            ),
        )
    }

    /// Never reads its stdin, and waits on a child (`ping`).
    pub fn ignores_its_end(session: SessionId) -> SpawnPipes {
        let mut request = pipe_spawn(session, "cmd.exe", &["/d", "/c", "ping -n 1000 127.0.0.1 >nul"]);
        request.env = inherited_env(&[]);
        request
    }

    /// Prints `first`, waits until `go` exists, prints `second`, makes
    /// `printed`, and waits for its stdin's end.
    pub fn prints_twice(session: SessionId, go: &Path, printed: &Path) -> SpawnPipes {
        powershell(
            session,
            &format!(
                "[Console]::Out.Write(\"first`n\"); \
                 while (-not (Test-Path '{}')) {{ Start-Sleep -Milliseconds 20 }}; \
                 [Console]::Out.Write(\"second`n\"); New-Item '{}' | Out-Null; \
                 $null = [Console]::In.ReadToEnd()",
                go.display(),
                printed.display()
            ),
        )
    }

    /// Prints `ready`, waits until `go` exists, writes `bye` to stderr, and
    /// exits with 3.
    pub fn exits_when_released(session: SessionId, go: &Path) -> SpawnPipes {
        powershell(
            session,
            &format!(
                "[Console]::Out.Write(\"ready`n\"); \
                 while (-not (Test-Path '{}')) {{ Start-Sleep -Milliseconds 20 }}; \
                 [Console]::Error.Write('bye'); exit 3",
                go.display()
            ),
        )
    }

    pub fn release(go: &Path) {
        std::fs::write(go, b"").unwrap();
    }
}

/// `1` to `count`, a line each, as [`pipe_programs::counts_on_stderr`]
/// writes them.
fn counted(count: u32) -> Vec<u8> {
    (1..=count).map(|n| format!("{n}\n")).collect::<String>().into_bytes()
}

/// What a pipe session's program is sent, as an agent's JSON lines may
/// carry it: a line end of each kind, a tab, UTF-8, and bytes that are no
/// text at all. A PTY would rewrite or echo some of it.
const RAW: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":0}\n\tx\r\n\xc3\xa9\xff\x00\x1b[0m end";

#[test]
fn a_pipe_programs_bytes_pass_unchanged_both_ways() {
    let (_dir, mut core, _) = holder("pipes-bytes");
    core.spawn_pipes(pipe_programs::copies(1));
    core.input(1, RAW);
    let mut output = core.output_until(1, b" end");
    // A pipe session has no size and no console window.
    core.send(ToHolder::Resize(h1::Resize {
        session: 1,
        cols: 80,
        rows: 24,
        cell_w: 9,
        cell_h: 18,
    }));
    core.send(ToHolder::Reparent(h1::Reparent {
        session: 1,
        hwnd: 1,
    }));
    let mut errors = Vec::new();
    while errors.len() < 2 {
        match core.recv() {
            ToCore::Error(error) => errors.push((error.session, error.message)),
            ToCore::Output(data) if data.session == 1 => output.extend(data.bytes),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        errors,
        [
            (Some(1), "session 1 is on pipes and has no size".to_string()),
            (Some(1), "session 1 is on pipes and has no console window".to_string()),
        ]
    );
    // A hang-up closes its stdin first, its polite end: it exits on its own.
    core.hang_up(1);
    let (rest, stderr, code) = core.pipes_end(1);
    output.extend(rest);
    assert_eq!(output, RAW);
    assert_eq!((stderr, code), (Vec::new(), Some(0)));
}

#[cfg(unix)]
#[test]
fn a_pipe_program_has_no_controlling_terminal() {
    let (_dir, mut core, holder_pid) = holder("pipes-no-tty");
    let pid = core.spawn_pipes(pipe_programs::copies(1));
    assert_eq!(parent(pid), Some(holder_pid), "the holder is its parent");
    // `/proc/<pid>/stat` after the name: state, ppid, pgrp, session, tty_nr.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<&str> = stat[stat.rfind(')').unwrap() + 1..].split_whitespace().collect();
    let pid_text = pid.to_string();
    assert_eq!(fields[2], pid_text, "its own process group: {stat}");
    assert_eq!(fields[3], pid_text, "its own session: {stat}");
    assert_eq!(fields[4], "0", "no controlling terminal: {stat}");
    core.hang_up(1);
    assert_eq!(core.pipes_end(1).2, Some(0));
}

#[cfg(windows)]
#[test]
fn a_pipe_program_has_no_pseudoconsole_and_no_console_window() {
    let (_dir, mut core, _) = holder("pipes-no-console");
    // Its standard handles are the pipes (a console's would not be
    // redirected), and no console window shows.
    let script = "Add-Type -Name W -Namespace BrindleTest -MemberDefinition \
        '[DllImport(\"kernel32.dll\")] public static extern System.IntPtr GetConsoleWindow(); \
         [DllImport(\"user32.dll\")] public static extern bool IsWindowVisible(System.IntPtr w);'; \
        $w = [BrindleTest.W]::GetConsoleWindow(); \
        [Console]::Out.Write('redirected=' + [Console]::IsInputRedirected + ',' + \
            [Console]::IsOutputRedirected + ',' + [Console]::IsErrorRedirected + \
            ' visible=' + [BrindleTest.W]::IsWindowVisible($w) + '.'); \
        $null = [Console]::In.ReadToEnd()";
    core.spawn_pipes(pipe_programs::powershell(1, script));
    let output = core.output_until(1, b".");
    let output = String::from_utf8_lossy(&output);
    assert!(
        output.contains("redirected=True,True,True visible=False."),
        "{output}"
    );
    core.hang_up(1);
    assert_eq!(core.pipes_end(1).2, Some(0));
}

#[test]
fn a_pipe_sessions_stderr_tail_keeps_its_newest_16_kib() {
    const COUNT: u32 = 10_000;
    let (_dir, mut core, _) = holder("pipes-stderr");
    core.spawn_pipes(pipe_programs::counts_on_stderr(1, COUNT));
    let (output, stderr, code) = core.pipes_end(1);
    let all = counted(COUNT);
    assert!(all.len() > h1::MAX_STDERR_TAIL);
    assert_eq!(stderr.len(), h1::MAX_STDERR_TAIL);
    assert!(
        all.ends_with(&stderr),
        "not the newest bytes: {}",
        String::from_utf8_lossy(&stderr[..64])
    );
    assert_eq!((output, code), (Vec::new(), Some(0)));
}

#[test]
fn a_paused_pipe_sessions_stderr_is_still_read() {
    // Far more than a pipe holds, so the program blocks unless the holder
    // reads its stderr while its stdout is paused.
    const COUNT: u32 = 100_000;
    let (_dir, mut core, _) = holder("pipes-paused");
    let pid = core.spawn_pipes(pipe_programs::counts_on_stderr_after_a_line(1, COUNT));
    core.send(ToHolder::Pause(h1::SessionRef { session: 1 }));
    core.input(1, b"go\n");
    assert!(
        eventually(TIMEOUT, || !alive(pid)),
        "the program blocked writing its stderr"
    );
    core.send(ToHolder::Resume(h1::SessionRef { session: 1 }));
    let (_, stderr, code) = core.pipes_end(1);
    assert!(counted(COUNT).ends_with(&stderr) && !stderr.is_empty());
    assert_eq!(code, Some(0));
}

#[test]
fn a_hang_up_ends_a_pipe_program_that_ignores_its_end_and_its_children() {
    /// `pty::GRACEFUL_SHUTDOWN`: the hang-up ends the group or job one
    /// grace after it closes stdin, and kills what is left one after that.
    const GRACE: Duration = Duration::from_secs(1);
    let (_dir, mut core, _) = holder("pipes-hang-up");
    let pid = core.spawn_pipes(pipe_programs::ignores_its_end(1));
    #[cfg(unix)]
    let child = reported_pid(&core.output_until(1, b"."), "bg");
    #[cfg(windows)]
    let child = until("its ping", TIMEOUT, || {
        children_named(pid, "PING.EXE").first().copied()
    });
    assert!(alive(child));

    core.hang_up(1);
    let started = std::time::Instant::now();
    let (_, _, code) = core.pipes_end(1);
    assert_eq!(code, pipe_programs::ENDED_BY_THE_HANG_UP);
    // Within the graces and a bound for a loaded machine.
    assert!(
        eventually(2 * GRACE + Duration::from_secs(5), || !alive(pid) && !alive(child)),
        "still running {:?} after the hang-up",
        started.elapsed()
    );
}

#[test]
fn an_older_core_isnt_shown_a_pipe_session_and_a_newer_one_gets_it_with_its_tail() {
    let (dir, mut first, _) = holder("pipes-old-core");
    let go = dir.path.join("go");
    let printed = dir.path.join("printed");
    // A terminal session too: the older core is shown that one.
    first.spawn(programs::exits_after_a_line(2, 0));
    first.spawn_pipes(pipe_programs::prints_twice(1, &go, &printed));
    first.output_until(1, b"first\n");

    // An H1 version 2 core adopts the holder.
    let mut older = TestCore::connect_to(&dir);
    let adopted = older.hello_up_to(2, 2);
    assert!(first.closes());
    let welcome = adopted.welcome();
    assert_eq!(welcome.protocol, 2);
    let listed: Vec<_> = welcome.sessions.iter().map(|held| held.session).collect();
    assert_eq!(listed, [2], "only the terminal session is listed");
    // The pipe session prints while that core is connected.
    pipe_programs::release(&go);
    assert!(eventually(TIMEOUT, || printed.exists()));
    // Round trips: what the holder read meanwhile has been handled.
    for _ in 0..2 {
        older.hang_up(99);
        loop {
            match older.recv() {
                ToCore::Error(error) if error.session == Some(99) => break,
                ToCore::Output(data) => assert_ne!(data.session, 1, "{data:?}"),
                other => panic!("{other:?}"),
            }
        }
    }

    // A core that speaks version 3 gets it back, with what it printed.
    let mut newer = TestCore::connect_to(&dir);
    let adopted = newer.hello(3);
    for message in older.until_closed() {
        if let ToCore::Output(data) = message {
            assert_ne!(data.session, 1, "the older core was sent its output");
        }
    }
    let held = adopted
        .welcome()
        .sessions
        .iter()
        .find(|held| held.session == 1)
        .expect("the pipe session is listed");
    assert_eq!((held.mode, held.exit), (h1::SpawnMode::Pipes, None));
    let mut seen = adopted.tails[&1].clone();
    if !contains(&seen, b"second\n") {
        seen.extend(newer.output_until(1, b"second\n"));
    }
    assert_eq!(seen, b"first\nsecond\n");
    newer.hang_up(1);
    assert_eq!(newer.pipes_end(1).2, Some(0));
    newer.input(2, programs::ENTER);
    assert_eq!(newer.exit_code(2), Some(0));
}

#[test]
fn a_pipe_session_that_ended_with_no_core_has_its_stderr_tail_after_its_tail() {
    let (mut dir, mut core, _) = holder("pipes-held-exit");
    let go = dir.path.join("go");
    let pid = core.spawn_pipes(pipe_programs::exits_when_released(1, &go));
    dir.track(pid);
    core.output_until(1, b"ready\n");
    drop(core);
    pipe_programs::release(&go);
    assert!(
        eventually(TIMEOUT, || dir
            .logs()
            .contains("session 1: ended with no core to tell")),
        "{}",
        dir.logs()
    );

    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    let held = &adopted.welcome().sessions[0];
    assert_eq!(
        (held.session, held.mode, held.exit),
        (1, h1::SpawnMode::Pipes, Some(h1::ExitStatus { code: Some(3) }))
    );
    // `hello` checks that it follows the session's `TAIL_END`.
    let stderr = String::from_utf8_lossy(&adopted.stderr_tails[&1]).into_owned();
    assert!(stderr.contains("bye"), "{stderr:?}");
}

#[cfg(windows)]
#[test]
fn a_bare_name_that_is_a_batch_file_fails_with_how_to_run_it() {
    let (dir, mut core, _) = holder("pipes-batch");
    let bin = dir.path.join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("fake-agent.cmd"), b"@echo off\r\n").unwrap();
    let mut request = pipe_spawn(1, "fake-agent", &["--acp"]);
    let path = format!("{};{}", bin.display(), request.env["PATH"]);
    request.env.insert("PATH".into(), path);
    core.send(ToHolder::SpawnPipes(request));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 1);
            assert!(
                failed.message.contains("fake-agent is a batch file")
                    && failed.message.contains("fake-agent.cmd")
                    && failed.message.contains("cmd /c fake-agent --acp"),
                "{}",
                failed.message
            );
        }
        other => panic!("{other:?}"),
    }
}
