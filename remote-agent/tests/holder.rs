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
    /// for another line and exits with 0.
    pub fn echoes_a_line(session: SessionId) -> Spawn {
        shell(session, "echo one; read x; echo \"two:$x\"; read y")
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
    use std::path::{Path, PathBuf};

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::holder::Spawn;

    use super::support::shell;

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
    /// for another line and exits with 0.
    pub fn echoes_a_line(session: SessionId) -> Spawn {
        shell(session, "echo one& set /p x=& echo two:!x!& set /p y=& exit 0")
    }

    /// Line `n` of a file of numbered lines, without its line end.
    fn line(n: usize) -> String {
        format!("#{n:07}# {}", "x".repeat(60))
    }

    /// Writes a file of `lines` numbered lines into `dir`: output that is
    /// recognised in what a pseudoconsole makes of it. `cmd`'s `type` takes
    /// its path unquoted here.
    pub fn numbered_lines(dir: &Path, lines: usize) -> PathBuf {
        let path = dir.join(format!("lines-{lines}.txt"));
        assert!(!path.to_str().unwrap().contains(' '), "{path:?}");
        let text: String = (0..lines).map(|n| line(n) + "\r\n").collect();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Fails unless `output` has every one of `lines` numbered lines, in
    /// order.
    pub fn check_lines(output: &[u8], lines: usize) {
        let output = String::from_utf8_lossy(output);
        let mut from = 0;
        for n in 0..lines {
            match output[from..].find(&line(n)) {
                Some(at) => from += at + line(n).len(),
                None => panic!("line {n} is missing after byte {from} of {}", output.len()),
            }
        }
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

/// Waits for the `EXITED` of a session that was hung up. A signal ended its
/// program on Linux, so it has no exit code; on Windows it has the one the
/// console's end or the holder ended it with.
fn hung_up(core: &mut TestCore, session: u32) {
    let code = core.exit_code(session);
    #[cfg(unix)]
    assert_eq!(code, None);
    #[cfg(windows)]
    assert!(code.is_some());
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
    assert_eq!(dir.holder_endpoints(), [place.endpoint.clone()]);
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
    // REPARENT is Windows-only.
    #[cfg(unix)]
    match core.recv() {
        ToCore::Error(error) => {
            assert_eq!(
                (error.session, error.message.as_str()),
                (Some(9), "unsupported")
            )
        }
        other => panic!("{other:?}"),
    }
    // There it is advisory: a session that is gone gets no answer, so the
    // next message is the next request's.
    #[cfg(windows)]
    {
        core.spawn(programs::exits(1, 0));
        assert_eq!(core.exit_code(1), Some(0));
    }
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
        std::thread::sleep(self.pause);
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
    let pid = core.spawn(shell(
        1,
        "(trap '' HUP; exec sleep 1000) & echo \"bg=$!.\"; wait",
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
    let mut request = programs::echoes_a_line(1);
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
    drop(third);
    let mut fourth = TestCore::connect_to(&dir);
    fourth.hello(4);
    fourth.input(1, programs::ENTER);
    drop(fourth);
    assert!(eventually(TIMEOUT, || !alive(pid)));
    let mut fifth = TestCore::connect_to(&dir);
    let adopted = fifth.hello(5);
    match adopted.welcome().sessions[0].exit {
        Some(exit) => assert_eq!(exit.code, Some(0)),
        // Reaped, but its output was still being drained.
        None => assert_eq!(fifth.exit_code(1), Some(0)),
    }
    assert!(fifth.closes(), "the holder had nothing left");
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn set_meta_replaces_what_the_next_core_gets() {
    let (dir, mut first, _) = holder("set-meta");
    assert_eq!(h1::PROTOCOLS.max, 2, "SET_META is H1 version 2");
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
    assert_eq!(welcome.protocol, 2);
    assert_eq!(welcome.sessions[0].meta, updated);
    second.input(1, programs::ENTER);
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
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
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
    hung_up(&mut core, 1);
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
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(seq[0]), "seq finished, so output went somewhere");
    assert_eq!(state(seq[0]), Some('S'));

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
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
    assert!(holder_exits(&dir, holder_pid));
}

#[cfg(unix)]
#[test]
fn a_paused_session_blocks_until_resumed() {
    let (_dir, mut core, _) = holder("pause");
    let pid = core.spawn(shell(1, "read go; seq 1 300000; echo DONE; read z"));
    core.send(ToHolder::Pause(h1::SessionRef { session: 1 }));
    core.input(1, b"go\n");
    let mut seq = Vec::new();
    assert!(eventually(TIMEOUT, || {
        seq = children_named(pid, "seq");
        !seq.is_empty()
    }));
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(seq[0]), "seq finished, so the PTY was read");
    assert_eq!(state(seq[0]), Some('S'));

    core.send(ToHolder::Resume(h1::SessionRef { session: 1 }));
    let output = core.output_until(1, b"DONE\r\n");
    assert!(contains(&output, b"\r\n300000\r\n"));
    core.hang_up(1);
    assert_eq!(core.exit_code(1), None);
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
    use std::collections::BTreeMap;
    use std::time::Instant;

    use brindle_remote_protocol::holder::Spawn;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GW_OWNER, GetWindow, MSG, PM_REMOVE,
        PeekMessageW, WINDOW_EX_STYLE, WS_OVERLAPPED,
    };
    use windows::core::w;

    use super::*;

    /// How long a hung-up program has before the holder terminates it
    /// (`pty::GRACEFUL_SHUTDOWN`).
    const GRACE: Duration = Duration::from_secs(1);
    /// The exit code of a program the holder terminated.
    const TERMINATED: i32 = 1;

    /// This process's environment with `extra` over it, for programs that
    /// need more than the system's own (PowerShell).
    fn inherited_env(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let set = |name: &str| extra.iter().any(|(set, _)| set.eq_ignore_ascii_case(name));
        std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .filter(|(name, _)| !set(name))
            .chain(extra.iter().map(|(name, value)| (name.to_string(), value.to_string())))
            .collect()
    }

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
        // too, within the grace.
        let hung_up_at = Instant::now();
        core.hang_up(1);
        assert_ne!(core.exit_code(1), Some(TERMINATED));
        assert!(hung_up_at.elapsed() < GRACE, "it took the grace");
        assert!(eventually(TIMEOUT, || !alive(pid) && !alive(ping[0])));

        core.hang_up(2);
        hung_up(&mut core, 2);
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
        hung_up(&mut core, 1);
    }

    /// About 2.9 MB of output: more than a tail and the pipes hold.
    const LINES: usize = 40_000;

    /// Waits for a line, prints [`LINES`] numbered lines and `DONE`, and
    /// waits for another line.
    fn prints_lines_when_told(session: u32, dir: &TempDir) -> Spawn {
        let file = programs::numbered_lines(&dir.path, LINES);
        let script = format!("set /p go=& type {}& echo DONE& set /p z=", file.display());
        shell(session, &script)
    }

    #[test]
    fn without_a_core_a_program_blocks_instead_of_losing_output() {
        let (dir, mut core, holder_pid) = holder("block");
        let pid = core.spawn(prints_lines_when_told(1, &dir));
        // The holder handles the input before it sees the connection end.
        core.input(1, b"go\r");
        drop(core);

        // Long enough to print everything, if anything took it.
        std::thread::sleep(Duration::from_secs(2));
        assert!(alive(pid));

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
        hung_up(&mut core, 1);
        assert!(holder_exits(&dir, holder_pid));
    }

    #[test]
    fn a_paused_session_blocks_until_resumed() {
        let (dir, mut core, _) = holder("pause");
        let pid = core.spawn(prints_lines_when_told(1, &dir));
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
        hung_up(&mut core, 1);
    }

    /// A hidden top-level window of this thread's, destroyed on drop.
    struct Window(HWND);

    impl Window {
        fn new() -> Self {
            // SAFETY: a window of a system class, with no parent, menu or
            // parameter; it is never shown.
            let window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("brindle test owner"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    100,
                    100,
                    None,
                    None,
                    None,
                    None,
                )
            };
            Self(window.unwrap())
        }

        fn id(&self) -> u64 {
            self.0.0 as usize as u64
        }

        /// Handles what other threads sent this thread's windows: a host
        /// that makes one an owner may wait for it to answer.
        fn pump() {
            let mut message = MSG::default();
            // SAFETY: `message` outlives each call.
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                // SAFETY: the message was just read.
                unsafe { DispatchMessageW(&message) };
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: the window is this thread's, as `Window` isn't `Send`.
            unsafe { DestroyWindow(self.0) }.unwrap();
        }
    }

    /// The owner of the window `window`, if it has one (0 if not).
    fn owner(window: u64) -> u64 {
        let window = HWND(window as usize as *mut std::ffi::c_void);
        // SAFETY: any window handle may be asked about.
        let owner = unsafe { GetWindow(window, GW_OWNER) };
        owner.map_or(0, |owner| owner.0 as usize as u64)
    }

    #[test]
    fn reparent_makes_a_window_the_owner_of_the_sessions_console_window() {
        // Only the packaged ConPTY can; a build without it uses Windows' own.
        if !Path::new(BIN).with_file_name("conpty.dll").exists() {
            eprintln!("skipped: no packaged ConPTY beside the helper");
            return;
        }
        let (_dir, mut core, _) = holder("reparent");
        // Prints its console window as `hwnd=N;` and waits.
        let script = "$w = (Add-Type -Name Console -Namespace BrindleTest -PassThru \
            -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern System.IntPtr \
            GetConsoleWindow();')::GetConsoleWindow(); Write-Host ('hwnd=' + $w + ';'); \
            Start-Sleep -Seconds 600";
        core.spawn(powershell(1, script));
        let output = core.output_until(1, b";");
        let text = String::from_utf8_lossy(&output);
        let digits = text.split("hwnd=").nth(1).and_then(|rest| rest.split(';').next());
        let console_window: u64 = digits.and_then(|digits| digits.parse().ok()).unwrap_or(0);
        assert_ne!(console_window, 0, "no console window in {text:?}");
        assert_eq!(owner(console_window), 0);

        let window = Window::new();
        core.send(ToHolder::Reparent(h1::Reparent {
            session: 1,
            hwnd: window.id(),
        }));
        // The host applies it a moment later.
        let deadline = Instant::now() + TIMEOUT;
        while owner(console_window) != window.id() {
            assert!(
                Instant::now() < deadline,
                "the owner is {:#x}, not {:#x}",
                owner(console_window),
                window.id()
            );
            Window::pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        // It has no answer: the next message is the hang-up's.
        core.hang_up(1);
        hung_up(&mut core, 1);
    }
}
