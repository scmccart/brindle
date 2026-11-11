//! `brindle-remote hold`, driven by a test core over H1 (tasks 3.2–3.4).

#![cfg(target_os = "linux")]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use brindle_remote_protocol::Frame;
use brindle_remote_protocol::holder::{self as h1, Drain, Goodbye, Previous, ToCore, ToHolder};

use support::*;

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
    eventually(TIMEOUT, || !alive(pid) && dir.holder_sockets().is_empty())
}

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

#[test]
fn a_ready_pipe_is_passed_as_fd_3_whether_or_not_it_is_there_already() {
    // `dup2(3, 3)` does nothing, so the server's exec would close it and
    // `--ready-fd 3` would name a closed fd.
    assert_passed_as_fd_3(true);
    assert_passed_as_fd_3(false);
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
    let socket = &dir.holder_sockets()[0];
    let name = socket.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, format!("holder-{}-0.sock", welcome.build));
    let mode = fs::metadata(&dir.path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);

    // REPARENT is Windows-only.
    core.send(ToHolder::Reparent(h1::Reparent {
        session: 9,
        hwnd: 1,
    }));
    match core.recv() {
        ToCore::Error(error) => {
            assert_eq!(
                (error.session, error.message.as_str()),
                (Some(9), "unsupported")
            )
        }
        other => panic!("{other:?}"),
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
    let (_dir, mut core, _) = holder("missing");
    core.send(ToHolder::Spawn(spawn(
        4,
        "/nonexistent/brindle-no-such-program",
        &[],
    )));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 4);
            assert!(
                failed
                    .message
                    .contains("/nonexistent/brindle-no-such-program")
                    && failed.message.contains("No such file"),
                "{}",
                failed.message
            );
        }
        other => panic!("{other:?}"),
    }
    // The holder still works.
    core.spawn(shell(5, "exit 0"));
    assert_eq!(core.exit_code(5), Some(0));
}

#[test]
fn exit_statuses_are_reported_and_an_empty_holder_exits() {
    let (dir, mut core, pid) = holder("exit");
    core.spawn(shell(1, "read x; exit 3"));
    core.spawn(shell(2, "kill -TERM $$"));
    assert_eq!(core.exit_code(2), None, "killed by a signal");
    assert!(alive(pid), "one session is left");
    core.input(1, b"\n");
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
    const FLOOD: usize = 6 * 1024 * 1024;
    let mut dir = TempDir::new("flood");
    start_holder(&dir.path);
    let socket = dir.holder_sockets().remove(0);
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let send = |stream: &mut std::os::unix::net::UnixStream, message: ToHolder| {
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
    // No newlines, so the PTY passes the bytes through unchanged.
    send(
        &mut stream,
        ToHolder::Spawn(shell(
            1,
            &format!("head -c {FLOOD} /dev/zero | tr '\\0' x; exit 0"),
        )),
    );
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
    let mut output = 0usize;
    let mut code = None;
    let mut last_poke = std::time::Instant::now();
    loop {
        let frame = match Frame::read_from_or_eof(&mut reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => panic!("after {output} bytes of output: {error}"),
        };
        match ToCore::decode(&frame).unwrap() {
            ToCore::Welcome(welcome) => dir.track(welcome.pid),
            ToCore::Spawned(_) => {}
            ToCore::Output(data) => {
                assert_eq!(code, None, "output after EXITED");
                assert!(data.bytes.iter().all(|&byte| byte == b'x'));
                output += data.bytes.len();
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
    assert_eq!(output, FLOOD, "every byte of output");
    assert_eq!(code, Some(Some(0)), "the exit, with its status");
    // The holder closed its side; once the core closes too, it is gone.
    drop(reader);
    drop(stream);
    assert!(
        eventually(TIMEOUT, || dir.holder_sockets().is_empty()
            && server_processes(&dir.path, "hold").is_empty()),
        "the holder didn't exit"
    );
    let logs = dir.logs();
    assert!(!logs.contains("didn't take"), "{logs}");
}

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

/// Makes a FIFO at `path`. A session's program that reads it waits until the
/// test writes to it, which needs no core.
fn make_fifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `c_path` is a NUL-terminated string that outlives the call.
    let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    assert_eq!(result, 0, "mkfifo: {}", std::io::Error::last_os_error());
}

#[test]
fn a_new_core_gets_the_sessions_checkpoints_and_tails() {
    use std::io::Write;

    let (dir, mut first, holder_pid) = holder("adopt");
    let go = dir.path.join("go");
    make_fifo(&go);
    let mut request = shell(
        1,
        &format!(
            "echo one; read x; echo \"two:$x\"; read y <'{}'",
            go.display()
        ),
    );
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
    assert_eq!(adopted.tails[&1], b"one\r\n");

    // A checkpoint replaces the tail it covers ("one\r\n").
    for message in ToHolder::checkpoint(1, b"CP1", 5) {
        second.send(message);
    }
    second.input(1, b"abc\n");
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
    // The program is let go through the FIFO once the core has gone. Input
    // would need a core to carry it, and the exit it causes can reach the
    // holder before that core's disconnect does: the holder then reports
    // the exit to that core and, left without sessions, exits.
    drop(third);
    // Opened for reading too, so the open doesn't wait for the program's.
    let mut release = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&go)
        .unwrap();
    release.write_all(b"\n").unwrap();
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
    assert_eq!(h1::PROTOCOLS.max, 2, "SET_META is H1 version 2");
    let mut request = shell(1, "read x");
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
    second.input(1, b"\n");
}

#[test]
fn what_a_core_sent_before_it_went_counts_while_the_holder_had_output_for_it() {
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    let (dir, mut first, holder_pid) = holder("write-fail");
    // It prints without pause, so the holder always has output to send.
    first.spawn(shell(1, "stty raw -echo; exec yes"));

    // The next core never reads. Once its socket is full, the holder has
    // output queued for it, which it tries to send at each turn.
    let mut silent = UnixStream::connect(&dir.holder_sockets()[0]).unwrap();
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
    let unread = |stream: &UnixStream| {
        let mut waiting: libc::c_int = 0;
        // SAFETY: FIONREAD writes one int, and the fd is open.
        let result = unsafe { libc::ioctl(stream.as_raw_fd(), libc::FIONREAD, &mut waiting) };
        assert_eq!(result, 0);
        waiting
    };
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
    core.spawn(shell(
        1,
        "head -c 1500000 /dev/zero | tr '\\0' x; echo; echo END; read z",
    ));
    core.output_until(1, b"END\r\n");
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert!(adopted.welcome().sessions[0].truncated);
    let tail = &adopted.tails[&1];
    assert_eq!(tail.len() as u64, h1::TAIL_LIMIT);
    assert!(tail.ends_with(b"x\r\nEND\r\n") && tail.starts_with(b"xxx"));

    // A checkpoint clears the flag.
    for message in ToHolder::checkpoint(1, b"CP", h1::TAIL_LIMIT) {
        core.send(message);
    }
    drop(core);
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(3);
    assert!(!adopted.welcome().sessions[0].truncated);
    assert_eq!(adopted.tails[&1], b"");
    core.hang_up_and_wait(1);
}

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

#[test]
fn a_drained_holder_refuses_spawn_and_exits_after_its_last_session() {
    let (dir, mut core, holder_pid) = holder("drain");
    core.spawn(shell(1, "read x"));
    core.send(ToHolder::Drain(Drain {}));
    core.send(ToHolder::Spawn(shell(2, "exit 0")));
    match core.recv() {
        ToCore::SpawnFailed(failed) => {
            assert_eq!(failed.session, 2);
            assert!(failed.message.contains("draining"), "{}", failed.message);
        }
        other => panic!("{other:?}"),
    }
    assert!(alive(holder_pid));
    core.input(1, b"\n");
    assert_eq!(core.exit_code(1), Some(0));
    assert!(holder_exits(&dir, holder_pid));
}

#[test]
fn a_holder_without_a_core_starts_one_from_its_own_binary_even_after_it_was_replaced() {
    let mut dir = TempDir::new("supervise");
    let bin_dir = TempDir::new("supervise-bin");
    let copy = bin_dir.path.join("brindle-remote");
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
    core.spawn(shell(1, "read x"));

    // An install replaces the file.
    let replacement = bin_dir.path.join("new");
    fs::write(&replacement, "#!/bin/sh\necho replaced binary ran\n").unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&replacement, &copy).unwrap();

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
    assert!(logs.contains(&format!("started /proc/{holder_pid}/exe --version")), "{logs}");

    // A core that connects stops the retries and keeps the session.
    let mut core = TestCore::connect_to(&dir);
    let adopted = core.hello(2);
    assert_eq!(adopted.welcome().sessions.len(), 1);
    core.input(1, b"\n");
    assert_eq!(core.exit_code(1), Some(0));
    assert!(holder_exits(&dir, holder_pid));
}
