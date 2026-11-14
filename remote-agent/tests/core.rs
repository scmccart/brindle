//! `brindle-remote serve`, driven by test clients over protocol 1, with real
//! holders under it (task groups 4 and 5).
//!
//! Most tests run on both platforms with the platform's own programs
//! ([`programs`]). Those that need a Unix shell, exact PTY bytes, signals
//! or `/proc` are Linux's, and Windows has its own beside them
//! ([`on_windows`]) for the same behaviour over a pseudoconsole. Its output
//! carries the console host's own escape sequences, so where Linux compares
//! a replay with the bytes a program printed, Windows compares it with what
//! an attached client saw.

#![cfg(any(target_os = "linux", windows))]

mod support;

#[cfg(unix)]
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, ToClient};

use support::*;

/// The sessions' programs, as a Unix shell runs them.
#[cfg(unix)]
mod programs {
    use std::path::Path;

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::client::Spawn;

    use super::support::{TestClient, client_shell, client_spawn, reported_pid};

    /// What ends a line of input.
    pub const ENTER: &[u8] = b"\n";
    /// A program that doesn't exist, and what the error says of it.
    pub const MISSING: (&str, &str) = ("/nonexistent/brindle-no-such-program", "No such file");
    /// How many bytes [`flood`] prints.
    const FLOOD: usize = 6 * 1024 * 1024;

    /// Exits with `code`.
    pub fn exits(code: i32) -> Spawn {
        client_shell(&format!("exit {code}"))
    }

    /// Waits for a line of input, then exits with `code`.
    pub fn exits_after_a_line(code: i32) -> Spawn {
        client_shell(&format!("read x; exit {code}"))
    }

    /// Waits for a line of input.
    pub fn waits() -> Spawn {
        client_shell("read x")
    }

    /// Runs until it is ended.
    pub fn sleeps() -> Spawn {
        client_shell("sleep 1000")
    }

    /// Answers every line of input with [`echo_of`] it.
    pub fn echoer() -> Spawn {
        client_spawn("cat", &[])
    }

    /// What [`echoer`]'s session prints for the line `word`: the PTY's
    /// echo, then `cat`'s.
    pub fn echo_of(word: &str) -> Vec<u8> {
        format!("{word}\r\n{word}\r\n").into_bytes()
    }

    /// Prints a flood and exits with 0. No newlines, so the PTY passes the
    /// bytes through unchanged.
    pub fn flood(_dir: &Path) -> Spawn {
        client_shell(&format!(
            "head -c {FLOOD} /dev/zero | tr '\\0' x; exit 0"
        ))
    }

    /// Fails unless `output` is all of [`flood`]'s.
    pub fn check_flood(output: &[u8]) {
        assert!(output.iter().all(|&byte| byte == b'x'));
        assert_eq!(output.len(), FLOOD, "every byte of output");
    }

    /// Starts a child that runs until it is ended, and waits for it.
    pub fn with_a_child() -> Spawn {
        client_shell("sleep 1000 & echo \"bg=$!.\"; wait")
    }

    /// The child of [`with_a_child`]'s program `pid`, in `session`.
    pub fn child_of(client: &mut TestClient, session: SessionId, _pid: u32) -> u32 {
        reported_pid(&client.output_until(session, b"."), "bg")
    }

    /// Sets the title `build`, prints `ready.` and waits.
    pub fn titled() -> Spawn {
        client_shell("printf '\\033]2;build\\007ready.'; exec cat")
    }

    /// The title [`titled`]'s session gets for `text`.
    pub fn title(text: &str) -> String {
        text.to_string()
    }
}

/// The sessions' programs, as `cmd.exe` runs them.
#[cfg(windows)]
mod programs {
    use std::path::Path;

    use brindle_remote_protocol::SessionId;
    use brindle_remote_protocol::client::Spawn;

    use super::support::{
        TIMEOUT, TestClient, check_lines, children_named, client_shell, client_spawn, cmd_title,
        eventually, numbered_lines,
    };

    /// What ends a line of input.
    pub const ENTER: &[u8] = b"\r";
    /// A program that doesn't exist, and what the error says of it.
    pub const MISSING: (&str, &str) = (r"C:\nonexistent\brindle-no-such-program.exe", "cannot find");
    /// How many lines [`flood`] prints: about 6 MB.
    const FLOOD_LINES: usize = 85_000;

    /// Exits with `code`.
    pub fn exits(code: i32) -> Spawn {
        client_shell(&format!("exit {code}"))
    }

    /// Waits for a line of input, then exits with `code`.
    pub fn exits_after_a_line(code: i32) -> Spawn {
        client_shell(&format!("set /p x=& exit {code}"))
    }

    /// Waits for a line of input.
    pub fn waits() -> Spawn {
        exits_after_a_line(0)
    }

    /// Runs until it is ended. It is the session's own program, not a
    /// shell's child: a program that joins a console while it closes isn't
    /// told to end, and these sessions are closed as soon as they start.
    pub fn sleeps() -> Spawn {
        client_spawn("ping.exe", &["-n", "1000", "127.0.0.1"])
    }

    /// Answers every line of input with [`echo_of`] it.
    pub fn echoer() -> Spawn {
        client_shell("for /l %i in (0,0,1) do @(set /p x=& echo got:!x!)")
    }

    /// What [`echoer`]'s session prints for the line `word`, after the
    /// console's echo of it.
    pub fn echo_of(word: &str) -> Vec<u8> {
        format!("got:{word}").into_bytes()
    }

    /// Prints a flood and exits with 0.
    pub fn flood(dir: &Path) -> Spawn {
        let file = numbered_lines(dir, FLOOD_LINES);
        client_shell(&format!("type {}& exit 0", file.display()))
    }

    /// Fails unless `output` is all of [`flood`]'s.
    pub fn check_flood(output: &[u8]) {
        check_lines(output, FLOOD_LINES);
    }

    /// Starts a child that runs until it is ended, and waits for it.
    pub fn with_a_child() -> Spawn {
        client_shell("ping -t 127.0.0.1 >nul")
    }

    /// The child of [`with_a_child`]'s program `pid`.
    pub fn child_of(_client: &mut TestClient, _session: SessionId, pid: u32) -> u32 {
        let mut children = Vec::new();
        assert!(
            eventually(TIMEOUT, || {
                children = children_named(pid, "ping");
                !children.is_empty()
            }),
            "the program started no child"
        );
        children[0]
    }

    /// Sets the title `build`, prints `ready.` and waits.
    pub fn titled() -> Spawn {
        client_shell("title build& echo ready.& set /p x=")
    }

    /// The title [`titled`]'s session gets for `text`: cmd's, which is
    /// prefixed when the tests run elevated.
    pub fn title(text: &str) -> String {
        cmd_title(text)
    }
}

/// A core in a fresh directory, with a connected client.
fn core(tag: &str) -> (TempDir, TestClient) {
    let mut dir = TempDir::new(tag);
    start_core(&dir.path, &[]).unwrap();
    let client = TestClient::hello(&dir.path, &format!("token-{tag}"));
    dir.track(client.core_pid());
    (dir, client)
}

/// Sends the line `word` to an [`programs::echoer`] session and waits for
/// its answer.
fn say(client: &mut TestClient, session: SessionId, word: &str) {
    client.input(session, &[word.as_bytes(), programs::ENTER].concat());
    client.output_until(session, &programs::echo_of(word));
}

/// Waits for the `EXIT` of a session that was hung up. On Linux its
/// status isn't the server's to decide: the signal nearly always ends the
/// program (no exit code), but one blocked reading the terminal now and then
/// gets through "the read failed, exit 1" first. On Windows it has the code
/// the console's end or the holder ended it with.
fn hung_up(client: &mut TestClient, session: SessionId) {
    let code = client.exit_code(session);
    #[cfg(windows)]
    assert!(code.is_some());
    #[cfg(unix)]
    let _ = code;
}

// 4.1

#[test]
fn racing_starts_leave_one_detached_core() {
    let mut dir = TempDir::new("race");
    let starts: Vec<_> = (0..2)
        .map(|_| {
            let path = dir.path.clone();
            thread::spawn(move || start_core(&path, &[]))
        })
        .collect();
    for start in starts {
        // Both starters are told a core is listening.
        start.join().unwrap().unwrap();
    }
    let client = TestClient::hello(&dir.path, "t");
    let pid = client.core_pid();
    dir.track(pid);
    assert!(
        eventually(TIMEOUT, || server_processes(&dir.path, "serve") == [pid]),
        "cores: {:?}",
        server_processes(&dir.path, "serve")
    );
    assert_ne!(parent(pid), Some(std::process::id()));

    let welcome = client.welcome.as_ref().unwrap();
    assert_eq!(welcome.protocol, 1);
    assert_eq!(welcome.core_version, env!("CARGO_PKG_VERSION"));
    assert!(!welcome.core_build.is_empty());
    assert!(welcome.holder_protocols.is_empty(), "no holder yet");
}

#[test]
fn sessions_share_one_holder_that_is_neither_the_core_nor_the_client() {
    let (mut dir, mut client) = core("share");
    let core_pid = client.core_pid();
    let pids: Vec<u32> = (0..3)
        .map(|_| client.spawn(programs::waits()).1)
        .collect();
    let holder = parent(pids[0]).unwrap();
    dir.track(holder);
    assert!(pids.iter().all(|&pid| parent(pid) == Some(holder)));
    assert_eq!(server_processes(&dir.path, "hold"), [holder]);
    assert_eq!(dir.holder_endpoints().len(), 1);
    for pid in [core_pid, holder] {
        let parent = parent(pid);
        assert_ne!(parent, Some(std::process::id()));
        assert_ne!(parent, Some(core_pid));
    }

    // The next client hears of the holder's protocol.
    let other = TestClient::hello(&dir.path, "other");
    assert_eq!(other.welcome.as_ref().unwrap().holder_protocols, [brindle_remote_protocol::holder::PROTOCOLS.max]);
}

#[test]
fn an_unused_core_and_its_holder_exit_and_remove_their_sockets() {
    // Never used at all.
    let idle = TempDir::new("unused-idle");
    start_core(&idle.path, &[]).unwrap();
    let pid = server_processes(&idle.path, "serve")[0];
    let started = Instant::now();
    assert!(gone(pid));
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert!(core_endpoint_gone(&idle.path));

    // Used, then left.
    let (dir, mut client) = core("unused");
    let core_pid = client.core_pid();
    let (session, _) = client.spawn(programs::exits_after_a_line(0));
    let holder = server_processes(&dir.path, "hold")[0];
    client.input(session, programs::ENTER);
    assert_eq!(client.exit_code(session), Some(0));
    drop(client);
    assert!(gone(holder) && gone(core_pid));
    assert!(core_endpoint_gone(&dir.path));
    assert!(dir.holder_endpoints().is_empty());
}

#[test]
fn a_new_core_adopts_the_holder_and_continues_the_session_ids() {
    let (mut dir, mut client) = core("adopt");
    let first = client.spawn(programs::waits());
    let second = client.spawn(programs::waits());
    assert_eq!((first.0, second.0), (1, 2));
    let holder = parent(first.1).unwrap();
    dir.track(holder);

    kill(client.core_pid());
    assert!(client.closes());
    assert!(gone(client.core_pid()));
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "someone-else");
    dir.track(client.core_pid());
    let (third, pid) = client.spawn(programs::waits());
    assert_eq!(third, 3);
    assert_eq!(parent(pid), Some(holder), "the adopted holder is reused");
    assert!(alive(first.1) && alive(second.1));
    let logs = dir.logs();
    assert!(
        logs.contains("previous core Crash) with 2 session(s)"),
        "{logs}"
    );
}

// 4.2

#[cfg(unix)]
#[test]
fn a_session_gets_exactly_the_requests_environment() {
    let mut dir = TempDir::new("env");
    // Set on the core (and so its holder): the program must not see it.
    start_core(&dir.path, &[("FOO", "1")]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    dir.track(client.core_pid());

    let mut request = client_spawn("env", &[]);
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
    let (session, _) = client.spawn(request);
    let (output, code) = client.output_and_exit(session);
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
fn a_missing_command_is_an_error_naming_it() {
    let (program, reason) = programs::MISSING;
    let (_dir, mut client) = core("missing");
    match client.spawn_reply(client_spawn(program, &[])) {
        ToClient::Error(error) => {
            assert!(error.session.is_none());
            assert!(
                error.message.contains(program) && error.message.contains(reason),
                "{}",
                error.message
            );
        }
        other => panic!("{other:?}"),
    }
    // The core and its holder still work.
    let (session, _) = client.spawn(programs::exits(0));
    assert_eq!(client.exit_code(session), Some(0));
}

#[test]
fn exit_statuses_reach_the_client() {
    let (_dir, mut client) = core("exit");
    let (three, _) = client.spawn(programs::exits_after_a_line(3));
    #[cfg(unix)]
    {
        let (signalled, _) = client.spawn(client_shell("kill -TERM $$"));
        assert_eq!(client.exit_code(signalled), None, "killed by a signal");
    }
    client.input(three, programs::ENTER);
    assert_eq!(client.exit_code(three), Some(3));
    // It is gone: input for it is refused.
    client.input(three, b"x");
    match client.recv() {
        ToClient::Error(error) => assert_eq!(
            (error.session, error.message.as_str()),
            (Some(three), &*format!("no session {three}"))
        ),
        other => panic!("{other:?}"),
    }
}

/// The perf run's failure: a flood, then the program's exit, read slowly by
/// the client. The holder becomes empty as soon as the program ends, so it
/// exits while the core is still busy with the flood (paused, sending
/// checkpoints). The client must still get every byte and the exit, not a
/// lost session.
#[test]
fn a_flood_and_its_exit_reach_a_slow_client() {
    let (dir, mut client) = core("flood");
    let (session, _) = client.spawn(programs::flood(&dir.path));
    let mut output = Vec::new();
    let mut since_pause = 0usize;
    let deadline = Instant::now() + Duration::from_secs(60);
    let exit = loop {
        assert!(Instant::now() < deadline, "only {} bytes arrived", output.len());
        match client.recv() {
            ToClient::Data(data) if data.session == session => {
                since_pause += data.bytes.len();
                output.extend(data.bytes);
                if since_pause >= 64 * 1024 {
                    since_pause = 0;
                    thread::sleep(Duration::from_millis(30));
                }
            }
            ToClient::Exit(exit) if exit.session == session => break exit,
            other => panic!("after {} bytes: {other:?}", output.len()),
        }
    };
    programs::check_flood(&output);
    assert_eq!((exit.code, exit.lost), (Some(0), None), "{}", dir.logs());
    assert!(
        eventually(TIMEOUT, || dir.holder_endpoints().is_empty()),
        "the empty holder didn't exit"
    );
    let logs = dir.logs();
    assert!(!logs.contains("lost"), "{logs}");
}

#[test]
fn spawning_as_the_last_session_ends_still_works() {
    // An empty holder exits, maybe with the next SPAWN unread; the core
    // sends it to a new holder.
    let (_dir, mut client) = core("respawn");
    for round in 0..10 {
        let (session, _) = client.spawn(programs::exits(round));
        assert_eq!(client.exit_code(session), Some(round));
    }
}

#[cfg(unix)]
#[test]
fn input_resize_and_close() {
    let (dir, mut client) = core("io");
    let (session, _) = client.spawn(client_shell(
        "read line; stty size; echo \"got $line\"; read z",
    ));
    client.send(client::ToCore::Resize(client::Resize {
        session,
        cols: 100,
        rows: 30,
        cell_w: 8,
        cell_h: 16,
    }));
    client.input(session, b"hello\n");
    let output = client.output_until(session, b"got hello");
    assert!(
        contains(&output, b"30 100"),
        "{}",
        String::from_utf8_lossy(&output)
    );

    let (closed, pid) = client.spawn(client_shell("sleep 1000"));
    client.send(client::ToCore::Close(client::SessionRef {
        session: closed,
    }));
    assert_eq!(client.exit_code(closed), None);
    assert!(!alive(pid));

    // Another client can't use these sessions.
    let mut other = TestClient::hello(&dir.path, "other");
    other.input(session, b"x");
    match other.recv() {
        ToClient::Error(error) => {
            assert_eq!(error.session, Some(session));
            assert!(
                error.message.contains("isn't attached"),
                "{}",
                error.message
            );
        }
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_stalled_client_doesnt_delay_another_clients_session() {
    let (dir, mut stalled) = core("stall");
    let (flood, flood_pid) = stalled.spawn(client_shell("exec yes flood"));
    // From here on it stops reading.
    let mut reader = TestClient::hello(&dir.path, "reader");
    let (echo, _) = reader.spawn(client_shell(
        "while read line; do echo \"pong $line\"; done",
    ));

    // The flood's program ends up blocked, not the core.
    let mut samples = 0;
    assert!(
        eventually(TIMEOUT, || {
            samples = if state(flood_pid) == Some('S') {
                samples + 1
            } else {
                0
            };
            samples >= 10
        }),
        "the flooding program never blocked"
    );
    let core_rss = rss_kib(stalled.core_pid());
    assert!(core_rss < 128 * 1024, "the core holds {core_rss} KiB");

    for round in 0..5 {
        let sent = Instant::now();
        reader.input(echo, format!("{round}\n").as_bytes());
        reader.output_until(echo, format!("pong {round}\r\n").as_bytes());
        assert!(
            sent.elapsed() < Duration::from_secs(5),
            "round {round} took {:?}",
            sent.elapsed()
        );
    }

    // Once the stalled client reads again, its session flows again: far
    // more arrives than was queued when it was paused.
    let mut received = 0usize;
    let deadline = Instant::now() + TIMEOUT;
    while received < 32 * 1024 * 1024 {
        assert!(Instant::now() < deadline, "only {received} bytes resumed");
        match stalled.recv() {
            ToClient::Data(data) if data.session == flood => received += data.bytes.len(),
            _ => {}
        }
    }
}

// 4.3

/// A little more than the core's reconnect grace period.
const PAST_GRACE: Duration = Duration::from_secs(12);

fn detach(mut spawn: client::Spawn) -> client::Spawn {
    spawn.policy = client::Policy::Detach;
    spawn
}

/// Kills the client's core (SIGKILL, or `TerminateProcess`) and waits until
/// another core (the holders' own) accepts connections.
fn kill_core_and_wait_for_another(dir: &TempDir, client: &mut TestClient) {
    let old = client.core_pid();
    kill(old);
    assert!(client.closes());
    assert!(
        eventually(Duration::from_secs(60), || {
            server_processes(&dir.path, "serve")
                .iter()
                .any(|&pid| pid != old)
                && core_listens(&dir.path)
        }),
        "no new core: {}",
        dir.logs()
    );
}

#[test]
fn a_sigkilled_clients_kill_session_ends_with_its_children_and_detach_survives() {
    let (mut dir, mut client) = core("sigkill");
    let core_pid = client.core_pid();
    let (with_child, pid) = client.spawn(programs::with_a_child());
    let background = programs::child_of(&mut client, with_child, pid);
    let echoer = programs::echoer();
    let command = [echoer.command.clone().unwrap_or_default()]
        .into_iter()
        .chain(echoer.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let (kept, kept_pid) = client.spawn(detach(echoer));
    dir.track(parent(pid).unwrap());

    let mut process = client.into_process();
    process.kill().unwrap();
    process.wait().unwrap();
    assert!(
        eventually(Duration::from_secs(5), || !alive(pid) && !alive(background)),
        "the kill session outlived its client"
    );
    thread::sleep(Duration::from_millis(500));
    assert!(alive(kept_pid), "the detach session ended with its client");
    assert!(alive(core_pid), "the core keeps the detach session");

    // Listed as detached, through the holder (LIST is task 6.2): with the
    // core gone, a test core takes the holder over and reads its list.
    kill(core_pid);
    assert!(gone(core_pid));
    let mut holder = TestCore::connect_to(&dir);
    let adopted = holder.hello(99);
    let sessions = &adopted.welcome().sessions;
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let held = &sessions[0];
    assert_eq!((held.session, held.pid, held.exit), (kept, kept_pid, None));
    let meta = held.meta.as_object().unwrap();
    let keys: Vec<&str> = meta.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["command", "cwd", "key", "owner", "policy", "profile", "scrollback", "started"]
    );
    assert_eq!(meta["policy"], "detach");
    assert_eq!(meta["owner"], "token-sigkill");
    assert_eq!(meta["command"], command);
    assert_eq!(meta["scrollback"], 1000);
}

#[test]
fn a_detach_session_goes_back_to_its_owners_token() {
    let (dir, mut client) = core("detach");
    let (session, pid) = client.spawn(detach(programs::echoer()));
    drop(client);
    thread::sleep(Duration::from_millis(500));
    assert!(alive(pid));

    let mut stranger = TestClient::hello(&dir.path, "stranger");
    stranger.input(session, b"x\n");
    match stranger.recv() {
        ToClient::Error(error) => assert_eq!(error.session, Some(session)),
        other => panic!("{other:?}"),
    }
    let mut owner = TestClient::hello(&dir.path, "token-detach");
    say(&mut owner, session, "back");
}

#[test]
fn a_client_back_within_the_grace_period_keeps_its_kill_sessions() {
    let mut dir = TempDir::new("grace-back");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "back");
    let old_core = client.core_pid();
    let (session, pid) = client.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());

    kill_core_and_wait_for_another(&dir, &mut client);
    let mut client = TestClient::hello(&dir.path, "back");
    assert_ne!(client.core_pid(), old_core);
    say(&mut client, session, "still here");

    thread::sleep(PAST_GRACE);
    assert!(alive(pid));
    say(&mut client, session, "later");
}

#[test]
fn kill_sessions_whose_owner_doesnt_come_back_end_after_the_grace_period() {
    let mut dir = TempDir::new("grace-gone");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "away");
    let (_, pid) = client.spawn(programs::echoer());
    let (kept, kept_pid) = client.spawn(detach(programs::echoer()));
    dir.track(parent(pid).unwrap());

    kill_core_and_wait_for_another(&dir, &mut client);
    // Someone else's client keeps no session alive.
    let _observer = TestClient::hello(&dir.path, "someone-else");
    let adopted = Instant::now();
    assert!(alive(pid) && alive(kept_pid));
    assert!(gone(pid));
    assert!(
        adopted.elapsed() >= Duration::from_secs(8),
        "hung up after {:?}",
        adopted.elapsed()
    );
    assert!(alive(kept_pid), "a detach session waits for its owner");

    let mut owner = TestClient::hello(&dir.path, "away");
    say(&mut owner, kept, "x");
}

#[test]
fn kill_and_stop_end_sessions_whatever_their_policy() {
    let (dir, mut client) = core("stop");
    let core_pid = client.core_pid();
    let (first, first_pid) = client.spawn(detach(programs::echoer()));
    client.send(client::ToCore::Kill(client::Kill {
        req: 100,
        target: client::KillTarget::Session(first),
    }));
    assert!(matches!(client.reply(100), ToClient::Ok(_)));
    hung_up(&mut client, first);
    assert!(!alive(first_pid));

    client.send(client::ToCore::Kill(client::Kill {
        req: 101,
        target: client::KillTarget::Session(999),
    }));
    match client.reply(101) {
        ToClient::Error(error) => assert_eq!(error.message, "no session 999"),
        other => panic!("{other:?}"),
    }

    let (kept, kept_pid) = client.spawn(detach(programs::echoer()));
    let (plain, plain_pid) = client.spawn(programs::sleeps());
    let holder = parent(kept_pid).unwrap();
    client.send(client::ToCore::Stop(client::Request { req: 102 }));
    assert!(matches!(client.reply(102), ToClient::Ok(_)));
    hung_up(&mut client, kept);
    // It doesn't read the terminal, so on Linux only the signal ends it.
    #[cfg(unix)]
    assert_eq!(client.exit_code(plain), None);
    #[cfg(windows)]
    hung_up(&mut client, plain);
    assert!(client.closes());
    assert!(!alive(kept_pid) && !alive(plain_pid));
    assert!(gone(core_pid) && gone(holder));
    assert!(core_endpoint_gone(&dir.path));
    assert!(dir.holder_endpoints().is_empty());
}

// 5.1

#[cfg(unix)]
#[test]
fn a_cursor_position_query_gets_only_the_clients_reply() {
    let (_dir, mut client) = core("query");
    // Reads, after two seconds, everything that came back to it.
    let (session, _) = client.spawn(client_shell(
        "stty raw -echo min 0 time 20; printf 'ask\\033[6n'; sleep 2; \
         got=$(dd bs=4096 count=1 2>/dev/null | od -An -c | tr -d ' \\n'); \
         printf 'got=%s.\\r\\n' \"$got\"",
    ));
    client.output_until(session, b"ask\x1b[6n");
    client.input(session, b"\x1b[5;7R");
    let output = client.output_until(session, b".\r\n");
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("got=033[5;7R."), "{text:?}");
    assert_eq!(client.exit_code(session), Some(0));
}

/// What makes a session's emulator panic in a debug core.
const PANIC_MARKER: &str = "brindle-test-panic-now";

#[test]
fn a_panicking_emulator_leaves_its_session_and_the_others_running() {
    let mut dir = TempDir::new("panic");
    start_core(&dir.path, &[("BRINDLE_TEST_FEED_PANIC", PANIC_MARKER)]).unwrap();
    let mut client = TestClient::hello(&dir.path, "t");
    dir.track(client.core_pid());
    let (broken, _) = client.spawn(programs::echoer());
    let (other, _) = client.spawn(programs::echoer());

    say(&mut client, broken, PANIC_MARKER);
    assert!(
        eventually(TIMEOUT, || dir.logs().contains("the emulator panicked")),
        "{}",
        dir.logs()
    );
    // Still forwarding, and not breaking again.
    say(&mut client, broken, &format!("after-{PANIC_MARKER}"));
    say(&mut client, other, "fine");
    assert_eq!(dir.logs().matches("the emulator panicked").count(), 1);

    // Its replay is just a reset; the other's is its screen. (One
    // connection's messages are handled in order, so each DETACH is.)
    client.detach(broken);
    client.detach(other);
    let (_, replay) = client.attach(broken, 80, 24).unwrap();
    assert_eq!(replay, b"\x1bc");
    let (_, replay) = client.attach(other, 80, 24).unwrap();
    assert!(contains(&replay, b"fine"));
}

// 5.3

/// Spawns `bytes` as a detached fixture session and detaches from it.
#[cfg(unix)]
fn detached_fixture(
    dir: &TempDir,
    client: &mut TestClient,
    bytes: &[u8],
    scrollback: u32,
) -> (SessionId, u32) {
    let (session, pid) = client.spawn(fixture_spawn(dir, "fixture", bytes, scrollback));
    client.detach(session);
    (session, pid)
}

#[cfg(unix)]
#[test]
fn attaching_to_a_full_screen_session_replays_its_screen() {
    let (dir, mut client) = core("attach-full");
    let fixture = full_screen_fixture(1);
    let (session, pid) = detached_fixture(&dir, &mut client, &fixture, 1000);
    let expected = snapshot(&replayed(&fixture, 80, 24, 1000));

    let mut other = TestClient::hello(&dir.path, "other");
    let (start, screen) = other.attach_until(session, 1000, |screen| snapshot(screen) == expected);
    assert_eq!((start.session, start.pid), (session, pid));
    let mode = *screen.term().mode();
    use brindle_term_core::alacritty_terminal::term::TermMode;
    assert!(mode.contains(TermMode::ALT_SCREEN | TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE));
    assert!(!mode.contains(TermMode::SHOW_CURSOR));
    assert_eq!(screen.title().as_deref(), Some("htop-like"));
}

#[cfg(unix)]
#[test]
fn attaching_replays_the_scrollback() {
    let (dir, mut client) = core("attach-scroll");
    let lines: String = (1..=500).map(|n| format!("line {n}\r\n")).collect();
    let (session, _) = detached_fixture(&dir, &mut client, lines.as_bytes(), 1000);
    let expected = snapshot(&replayed(lines.as_bytes(), 80, 24, 1000));
    let (_, screen) = client.attach_until(session, 1000, |screen| snapshot(screen) == expected);
    let text: Vec<String> = rows(&screen)
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect();
    let want: Vec<String> = (1..=500).map(|n| format!("line {n}")).collect();
    assert_eq!(text, want);
}

/// Three finished commands with their OSC 133 marks, then a prompt and an
/// OSC 7501 report of a working program.
fn marks_fixture() -> Vec<u8> {
    let mut fixture = Vec::new();
    for (n, exit) in [(1, 0), (2, 1), (3, 0)] {
        fixture.extend_from_slice(
            format!(
                "\x1b]133;A\x07$ \x1b]133;B\x07cmd{n}\r\n\x1b]133;C\x07output {n}\r\n\x1b]133;D;{exit}\x07"
            )
            .as_bytes(),
        );
    }
    fixture.extend_from_slice(b"\x1b]133;A\x07$ \x1b]7501;state=working:app=tests\x1b\\");
    fixture
}

#[cfg(unix)]
#[test]
fn attaching_replays_status_records_and_prompt_marks() {
    let (dir, mut client) = core("attach-marks");
    let fixture = marks_fixture();
    let (session, _) = detached_fixture(&dir, &mut client, &fixture, 1000);
    let expected = snapshot(&replayed(&fixture, 80, 24, 1000));
    let (_, screen) = client.attach_until(session, 1000, |screen| snapshot(screen) == expected);
    use brindle_term_core::status::State;
    assert_eq!(
        screen.records().get("").map(|record| record.state),
        Some(State::Working)
    );
    let marks = screen.marks();
    assert_eq!(marks.commands().len(), 4);
    assert_eq!(marks.prompt_lines().collect::<Vec<_>>(), [0, 2, 4, 6]);
}

#[cfg(unix)]
#[test]
fn an_attached_session_cant_be_taken_and_the_attach_size_applies() {
    let (dir, mut owner) = core("attach-refused");
    let (session, _) = owner.spawn(detach(client_shell(
        "while read line; do stty size; done",
    )));
    let mut other = TestClient::hello(&dir.path, "other");
    let refusal = other.attach(session, 100, 30).unwrap_err();
    assert_eq!(refusal.session, Some(session));
    assert!(refusal.message.contains("attached"), "{}", refusal.message);
    assert!(
        other
            .attach(999, 80, 24)
            .unwrap_err()
            .message
            .contains("no session 999")
    );
    assert!(
        owner
            .attach(session, 80, 24)
            .unwrap_err()
            .message
            .contains("already attached")
    );

    owner.detach(session);
    let (start, _) = loop {
        match other.attach(session, 100, 30) {
            Ok(replay) => break replay,
            // The DETACH may not have been read yet.
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    };
    assert_eq!(start.session, session);
    other.input(session, b"\n");
    other.output_until(session, b"30 100");
    // The first client no longer has it.
    owner.input(session, b"x");
    match owner.recv() {
        ToClient::Error(error) => assert!(error.message.contains("isn't attached")),
        message => panic!("{message:?}"),
    }
}

// 5.4

#[cfg(unix)]
#[test]
fn after_a_core_restart_a_full_screen_session_rebuilds_from_checkpoint_and_tail() {
    let mut dir = TempDir::new("rebuild-full");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "first");
    let first = full_screen_fixture(1);
    let second = full_screen_fixture(2);
    fs::write(dir.path.join("second"), &second).unwrap();
    let mut spawn = fixture_spawn(&dir, "first", &first, 1000);
    // The second frame comes after a line of input.
    spawn.args[1] = format!(
        "stty raw -echo; cat '{}'; read x; cat '{}'; exec sleep 1000",
        dir.path.join("first").display(),
        dir.path.join("second").display()
    );
    let (session, pid) = client.spawn(spawn);
    dir.track(parent(pid).unwrap());
    client.output_until(session, b"frame 1\x1b[12;30H");
    // The checkpoint, of the first frame.
    client.detach(session);
    client.attach(session, 80, 24).unwrap();
    // The tail: the second frame.
    client.input(session, b"\n");
    client.output_until(session, b"frame 2\x1b[12;30H");

    kill_core_and_wait_for_another(&dir, &mut client);
    let mut other = TestClient::hello(&dir.path, "other");
    let both = [first, second].concat();
    let expected = snapshot(&replayed(&both, 80, 24, 1000));
    let (start, screen) = other.attach_until(session, 1000, |screen| snapshot(screen) == expected);
    assert_eq!(start.pid, pid);
    assert_eq!(snapshot(&screen), expected);
    let logs = dir.logs();
    assert!(!logs.contains("truncated"), "{logs}");
}

/// Numbered, coloured lines: about `kib` KiB of them.
#[cfg(unix)]
fn lines_fixture(kib: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut n = 0;
    while out.len() < kib * 1024 {
        out.extend_from_slice(
            format!(
                "\x1b[3{}mline {n:06}\x1b[0m {}\r\n",
                n % 8,
                "-".repeat(40 + n % 30)
            )
            .as_bytes(),
        );
        n += 1;
    }
    out.extend_from_slice(b"all printed\r\n");
    out
}

#[cfg(unix)]
#[test]
fn a_session_with_more_output_than_a_tail_holds_rebuilds_exactly() {
    let mut dir = TempDir::new("rebuild-long");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "first");
    // More than the 1 MiB tail: only checkpoints keep it exact.
    let fixture = lines_fixture(1536);
    let mut spawn = fixture_spawn(&dir, "lines", &fixture, 1000);
    spawn.args[1] = format!(
        "stty raw -echo; cat '{}'; read x; printf 'ack\\r\\n'; exec sleep 1000",
        dir.path.join("lines").display()
    );
    let (session, pid) = client.spawn(spawn);
    dir.track(parent(pid).unwrap());
    client.output_until(session, b"all printed\r\n");
    // The core takes no checkpoint while it is behind its holder, so its
    // first one comes with the end of the flood, and is only queued for
    // the holder when the client has `all printed`. This line of input is
    // queued behind it, so its answer means the holder has the checkpoint.
    client.input(session, b"\n");
    client.output_until(session, b"ack\r\n");

    kill_core_and_wait_for_another(&dir, &mut client);
    let mut other = TestClient::hello(&dir.path, "other");
    let expected = snapshot(&replayed(&[&fixture[..], b"ack\r\n"].concat(), 80, 24, 1000));
    let (_, screen) = other.attach_until(session, 1000, |screen| snapshot(screen) == expected);
    assert_eq!(snapshot(&screen), expected);
    let logs = dir.logs();
    assert!(!logs.contains("truncated"), "{logs}");
}

#[cfg(unix)]
#[test]
fn a_truncated_tail_still_gives_a_working_session() {
    let mut dir = TempDir::new("rebuild-truncated");
    start_core(&dir.path, &[]).unwrap();
    let mut client = TestClient::hello(&dir.path, "first");
    let (session, pid) = client.spawn(detach(client_shell(
        "stty raw -echo; while read line; do \
         head -c 1500000 /dev/zero | tr '\\0' x; printf '\\r\\nEND %s\\r\\n' \"$line\"; done",
    )));
    let holder = parent(pid).unwrap();
    dir.track(holder);
    kill(client.core_pid());
    assert!(client.closes());

    // A core that never checkpoints: the tail overflows.
    let mut test_core = TestCore::connect_to(&dir);
    test_core.hello(7);
    test_core.input(session, b"one\n");
    test_core.output_until(session, b"END one\r\n");
    drop(test_core);

    start_core(&dir.path, &[]).unwrap();
    let mut other = TestClient::hello(&dir.path, "other");
    dir.track(other.core_pid());
    let (start, screen) = other.attach_until(session, 1000, |screen| {
        rows(screen).iter().any(|row| row == "END one")
    });
    assert_eq!(start.pid, pid);
    assert!(rows(&screen).iter().any(|row| row.starts_with("xxxx")));
    assert!(dir.logs().contains("tail was truncated"), "{}", dir.logs());
    other.input(session, b"two\n");
    other.output_until(session, b"END two\r\n");
}

// 5.5

#[cfg(unix)]
#[test]
fn a_client_back_after_a_core_restart_gets_its_replay_then_live_output() {
    let mut dir = TempDir::new("reconnect-replay");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut client = TestClient::hello(&dir.path, "back");
    let first = full_screen_fixture(1);
    let second = full_screen_fixture(2);
    fs::write(dir.path.join("second"), &second).unwrap();
    // A kill session, attached all along.
    let mut spawn = fixture_spawn(&dir, "first", &first, 1000);
    spawn.policy = client::Policy::Kill;
    spawn.args[1] = format!(
        "stty raw -echo; cat '{}'; read x; cat '{}'; exec sleep 1000",
        dir.path.join("first").display(),
        dir.path.join("second").display()
    );
    let (session, pid) = client.spawn(spawn);
    dir.track(parent(pid).unwrap());
    client.output_until(session, b"frame 1\x1b[12;30H");

    kill_core_and_wait_for_another(&dir, &mut client);
    let mut client = TestClient::hello(&dir.path, "back");
    let (start, replay) = client.reconnect_replay();
    assert_eq!((start.session, start.pid, start.req), (session, pid, None));
    let mut screen = replayed(&replay, 80, 24, 1000);
    assert_eq!(snapshot(&screen), snapshot(&replayed(&first, 80, 24, 1000)));

    // Live output follows the replay.
    client.input(session, b"\n");
    let live = client.output_until(session, b"frame 2\x1b[12;30H");
    screen.feed(&live);
    let both = [first, second].concat();
    assert_eq!(snapshot(&screen), snapshot(&replayed(&both, 80, 24, 1000)));
    assert!(alive(pid));
}

/// A process's resident set, in KiB.
#[cfg(unix)]
fn rss_kib(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next()?.parse().ok())
        .unwrap()
}

// terminal-server-on-close 2.1–2.4

/// Input to `session` is refused because the session isn't attached to
/// `client`: the core gave it no replay of it.
fn not_given_back(client: &mut TestClient, session: SessionId) {
    client.input(session, b"x");
    loop {
        match client.recv() {
            ToClient::Error(error) if error.session == Some(session) => return,
            ToClient::ReplayStart(start) if start.session == session => {
                panic!("session {session} was given back")
            }
            _ => {}
        }
    }
}

#[test]
fn sessions_list_their_key_title_start_and_profile_and_keys_outlive_ids() {
    let (dir, mut client) = core("keys");
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut spawn = detach(programs::titled());
    spawn.profile = Some("Shell".into());
    let reply = client.spawn_reply(spawn);
    let ToClient::Spawned(spawned) = reply else { panic!("{reply:?}") };
    assert_eq!(spawned.key.len(), 32, "{spawned:?}");
    client.output_until(spawned.session, b"ready.");
    let listed = client.list();
    let info = listed.iter().find(|s| s.session == spawned.session).unwrap();
    assert_eq!(info.key, spawned.key);
    assert_eq!(info.title, programs::title("build"));
    assert_eq!(info.profile.as_deref(), Some("Shell"));
    assert!(info.started >= before && info.started <= before + 60, "{info:?}");

    // A fresh server starts its ids over; the key tells the sessions apart.
    client.send(client::ToCore::Stop(client::Request { req: 900 }));
    assert!(matches!(client.reply(900), ToClient::Ok(_)));
    assert!(client.closes());
    start_core(&dir.path, &[]).unwrap();
    let mut again = TestClient::hello(&dir.path, "token-keys");
    let reply = again.spawn_reply(detach(programs::echoer()));
    let ToClient::Spawned(second) = reply else { panic!("{reply:?}") };
    assert_eq!(second.session, spawned.session);
    assert_ne!(second.key, spawned.key);
    again.send(client::ToCore::Stop(client::Request { req: 901 }));
    assert!(matches!(again.reply(901), ToClient::Ok(_)));
}

#[test]
fn an_explicit_detach_keeps_a_kill_session_and_clears_its_owner() {
    let (dir, mut client) = core("explicit-detach");
    let (session, pid) = client.spawn(programs::echoer());
    client.detach(session);
    let listed = client.list();
    let info = listed.iter().find(|s| s.session == session).unwrap();
    assert_eq!((info.attached, info.policy), (false, client::Policy::Detach));
    drop(client);
    thread::sleep(Duration::from_millis(500));
    assert!(alive(pid), "the detached kill session ended with its client");

    // Its old client gets nothing back; anyone may attach it.
    let mut back = TestClient::hello(&dir.path, "token-explicit-detach");
    not_given_back(&mut back, session);
    let mut other = TestClient::hello(&dir.path, "other");
    other.attach(session, 80, 24).unwrap();
}

#[test]
fn owner_and_policy_changes_survive_a_core_restart() {
    let mut dir = TempDir::new("set-meta-restart");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let (left, left_pid) = a.spawn(programs::echoer());
    let (taken, _) = a.spawn(detach(programs::echoer()));
    dir.track(parent(left_pid).unwrap());
    a.detach(left);
    a.detach(taken);
    let mut b = TestClient::hello(&dir.path, "b");
    b.attach(taken, 80, 24).unwrap();

    kill_core_and_wait_for_another(&dir, &mut a);
    assert!(b.closes());
    // Past the grace period: `left` was `kill` at spawn, and only its
    // stored policy (now `detach`) keeps it running.
    thread::sleep(PAST_GRACE);
    assert!(alive(left_pid), "the detached kill session ended after the restart");

    let mut a = TestClient::hello(&dir.path, "a");
    not_given_back(&mut a, left);
    not_given_back(&mut a, taken);
    let listed = a.list();
    let info = listed.iter().find(|s| s.session == left).unwrap();
    assert_eq!((info.attached, info.policy), (false, client::Policy::Detach));

    let mut b = TestClient::hello(&dir.path, "b");
    let (start, _) = b.reconnect_replay();
    assert_eq!(start.session, taken);
}

#[test]
fn an_attach_holds_when_the_core_is_killed_as_its_replay_starts() {
    let mut dir = TempDir::new("attach-kill");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let (session, pid) = a.spawn(detach(programs::echoer()));
    dir.track(parent(pid).unwrap());
    // A goes without detaching, so the session is still A's, in the holder
    // too: a core that forgot B's attach would give it back to A.
    drop(a);
    let mut b = TestClient::hello(&dir.path, "b");
    assert!(
        eventually(TIMEOUT, || {
            b.list().iter().any(|s| s.session == session && !s.attached)
        }),
        "A's session never lost its client"
    );

    // Killed on REPLAY_START, without reading any further.
    const REQ: brindle_remote_protocol::RequestId = 1 << 32;
    b.send(client::ToCore::Attach(client::Attach {
        req: REQ,
        session,
        cols: 80,
        rows: 24,
    }));
    loop {
        match b.recv() {
            ToClient::ReplayStart(start) if start.req == Some(REQ) => break,
            ToClient::Error(error) => panic!("attach refused: {}", error.message),
            _ => {}
        }
    }
    kill_core_and_wait_for_another(&dir, &mut b);

    let mut a = TestClient::hello(&dir.path, "a");
    not_given_back(&mut a, session);
    let mut b = TestClient::hello(&dir.path, "b");
    let (start, _) = b.reconnect_replay();
    assert_eq!((start.session, start.pid), (session, pid));
    // A still doesn't have it, now that B does.
    not_given_back(&mut a, session);
    say(&mut b, session, "mine");
}

#[test]
fn an_attaching_client_owns_the_session_and_its_policy_applies_when_it_goes() {
    let mut dir = TempDir::new("attach-owner");
    start_core(&dir.path, &[SUPERVISED]).unwrap();
    let mut a = TestClient::hello(&dir.path, "a");
    let (session, pid) = a.spawn(programs::echoer());
    dir.track(parent(pid).unwrap());
    // After a core restart the kill session waits for its owner; B takes it.
    kill_core_and_wait_for_another(&dir, &mut a);
    let mut b = TestClient::hello(&dir.path, "b");
    b.attach(session, 80, 24).unwrap();
    drop(b);
    assert!(
        eventually(Duration::from_secs(5), || !alive(pid)),
        "the kill session outlived the client that attached it"
    );
}

/// `REPARENT` is for Windows: a Linux core refuses it, and goes on.
#[cfg(unix)]
#[test]
fn a_linux_core_refuses_reparent() {
    let (_dir, mut client) = core("reparent");
    let (session, _) = client.spawn(programs::echoer());
    client.send(client::ToCore::Reparent(client::Reparent {
        session,
        hwnd: 0x1234,
    }));
    match client.recv() {
        ToClient::Error(error) => {
            assert_eq!(error.session, Some(session));
            assert!(error.message.contains("REPARENT is for Windows"), "{}", error.message);
        }
        other => panic!("{other:?}"),
    }
    say(&mut client, session, "still");
}

/// The systemd unit a process runs in, from its cgroup path.
#[cfg(unix)]
fn unit_of(pid: u32) -> String {
    let cgroup = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
    cgroup.trim().rsplit('/').next().unwrap_or_default().to_string()
}

#[cfg(unix)]
#[test]
fn holders_and_restarted_cores_run_in_scopes_of_their_own() {
    // `start_core` sets BRINDLE_NO_SCOPE=1 for the starter; "0" overrides it
    // for the roles the core and holder start.
    if let brindle_remote_protocol::scope::Decision::Plain(reason) =
        brindle_remote_protocol::scope::current()
    {
        eprintln!("skipped: {reason}");
        return;
    }
    let mut dir = TempDir::new("scopes");
    start_core(&dir.path, &[SUPERVISED, ("BRINDLE_NO_SCOPE", "0")]).unwrap();
    let mut client = TestClient::hello(&dir.path, "scopes");
    let (_, pid) = client.spawn(client_spawn("cat", &[]));
    let holder = parent(pid).unwrap();
    dir.track(holder);
    let unit = unit_of(holder);
    assert!(unit.starts_with("brindle-hold-") && unit.ends_with(".scope"), "holder in {unit}");
    assert_eq!(unit_of(pid), unit, "the holder's programs share its scope");

    kill_core_and_wait_for_another(&dir, &mut client);
    let client = TestClient::hello(&dir.path, "scopes");
    dir.track(client.core_pid());
    let unit = unit_of(client.core_pid());
    assert!(unit.starts_with("brindle-serve-") && unit.ends_with(".scope"), "core in {unit}");
}

/// The Windows tests of what the Linux-only ones above cover with a Unix
/// shell: the same behaviour, over a pseudoconsole. A replay is compared
/// with the session's output as an attached client saw it, since the
/// console host writes its own stream for what a program prints.
#[cfg(windows)]
mod on_windows {
    use std::fs;
    use std::path::Path;

    use brindle_term_core::alacritty_terminal::term::TermMode;

    use super::*;

    /// How long a session that has printed everything stays quiet before
    /// its output counts as all there.
    const QUIET: Duration = Duration::from_secs(1);

    /// An interactive `cmd` whose prompt is `ready>`.
    fn interactive() -> client::Spawn {
        let mut request = client_spawn("cmd.exe", &["/d"]);
        request.env = inherited_env(&[("PROMPT", "ready$G")]);
        request
    }

    /// A PowerShell session running `script`.
    fn powershell(script: &str) -> client::Spawn {
        let mut request =
            client_spawn("powershell.exe", &["-NoProfile", "-Command", script]);
        request.env = inherited_env(&[]);
        request
    }

    /// A `detach` PowerShell session that prints each of `parts` as it is
    /// (the escape sequences too), waiting for a line of input before every
    /// one but the first, and then runs until it is ended.
    fn printer(dir: &TempDir, parts: &[&[u8]], scrollback: u32) -> client::Spawn {
        let mut script = String::new();
        for (n, part) in parts.iter().enumerate() {
            let path = dir.path.join(format!("part-{n}"));
            fs::write(&path, part).unwrap();
            if n > 0 {
                script.push_str("[void][Console]::ReadLine(); ");
            }
            script.push_str(&format!(
                "[Console]::Write([IO.File]::ReadAllText('{}')); ",
                path.display()
            ));
        }
        script.push_str("Start-Sleep -Seconds 1000");
        let mut request = powershell(&script);
        request.scrollback = scrollback;
        request.policy = client::Policy::Detach;
        request
    }

    /// The console's size as `size=COLSxROWS;`, asked of an [`interactive`]
    /// session.
    const SIZE: &[u8] = b"powershell -NoProfile -Command \"$s = $Host.UI.RawUI.WindowSize; \
                          Write-Host ('size=' + $s.Width + 'x' + $s.Height + ';')\"\r";

    /// Asks an [`interactive`] session for its size until it is 100x30: the
    /// console applies a resize a moment later.
    fn becomes_100x30(client: &mut TestClient, session: SessionId) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            client.input(session, SIZE);
            let output = client.output_until(session, b"ready>");
            if contains(&output, b"size=100x30;") {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                String::from_utf8_lossy(&output)
            );
        }
    }

    #[test]
    fn a_session_gets_exactly_the_requests_environment() {
        let mut dir = TempDir::new("env");
        // Set on the core (and so its holder): the program must not see it.
        start_core(&dir.path, &[("FOO", "1")]).unwrap();
        let mut client = TestClient::hello(&dir.path, "t");
        dir.track(client.core_pid());

        // Wide enough for every variable to be one line.
        let mut request = client_spawn("cmd.exe", &["/d", "/c", "set& cd"]);
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
        let (session, _) = client.spawn(request);
        let (output, code) = client.output_and_exit(session);
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
    fn input_resize_and_close() {
        let (dir, mut client) = core("io");
        let (session, _) = client.spawn(interactive());
        client.output_until(session, b"ready>");
        client.send(client::ToCore::Resize(client::Resize {
            session,
            cols: 100,
            rows: 30,
            cell_w: 8,
            cell_h: 16,
        }));
        becomes_100x30(&mut client, session);
        // The caret goes when `cmd` runs the line, so only the program's
        // own output matches.
        client.input(session, b"echo got^ hello\r");
        client.output_until(session, b"got hello");

        let (closed, pid) = client.spawn(programs::sleeps());
        client.send(client::ToCore::Close(client::SessionRef {
            session: closed,
        }));
        hung_up(&mut client, closed);
        assert!(!alive(pid));

        // Another client can't use these sessions.
        let mut other = TestClient::hello(&dir.path, "other");
        other.input(session, b"x");
        match other.recv() {
            ToClient::Error(error) => {
                assert_eq!(error.session, Some(session));
                assert!(
                    error.message.contains("isn't attached"),
                    "{}",
                    error.message
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stalled_client_doesnt_delay_another_clients_session() {
        let (dir, mut stalled) = core("stall");
        let core_pid = stalled.core_pid();
        let file = numbered_lines(&dir.path, 20_000);
        let (flood, _) = stalled.spawn(client_shell(&format!(
            "for /l %i in (0,0,1) do @type {}",
            file.display()
        )));
        // From here on it stops reading.
        let mut reader = TestClient::hello(&dir.path, "reader");
        let (echo, _) = reader.spawn(programs::echoer());

        // Long enough for the flood to fill every queue on its way. Then
        // its program is blocked, not the core: the core holds no more of
        // it, however long the flood goes on.
        thread::sleep(Duration::from_secs(3));
        let held = working_set_kib(core_pid);
        assert!(held < 128 * 1024, "the core holds {held} KiB");

        for round in 0..5 {
            let sent = Instant::now();
            say(&mut reader, echo, &format!("ping{round}"));
            assert!(
                sent.elapsed() < Duration::from_secs(5),
                "round {round} took {:?}",
                sent.elapsed()
            );
        }
        let held = working_set_kib(core_pid);
        assert!(held < 128 * 1024, "the core holds {held} KiB");

        // Once the stalled client reads again, its session flows again: far
        // more arrives than was queued when it was paused.
        let mut received = 0usize;
        let deadline = Instant::now() + TIMEOUT;
        while received < 16 * 1024 * 1024 {
            assert!(Instant::now() < deadline, "only {received} bytes resumed");
            match stalled.recv() {
                ToClient::Data(data) if data.session == flood => received += data.bytes.len(),
                _ => {}
            }
        }
    }

    /// The console host passes a colour query through to the terminal, and
    /// the program gets one answer: the client's, since the core's emulator
    /// answers nothing.
    #[test]
    fn a_colour_query_gets_only_the_clients_reply() {
        let (_dir, mut client) = core("query");
        // Reads, after two seconds, everything that came back to it.
        let script = "$e = [char]27; $b = [char]7; \
            [Console]::Write('ask' + $e + ']11;?' + $b); Start-Sleep -Seconds 2; $r = ''; \
            while ([Console]::KeyAvailable) { $r += [Console]::ReadKey($true).KeyChar }; \
            [Console]::Write('got=' + ($r -replace $e, 'ESC' -replace $b, 'BEL') + '.')";
        let (session, _) = client.spawn(powershell(script));
        client.output_until(session, b"\x1b]11;?");
        client.input(session, b"\x1b]11;rgb:1010/1414/1818\x07");
        let mut output = Vec::new();
        let answered = |output: &[u8]| {
            let text = String::from_utf8_lossy(output);
            text.split("got=").nth(1).is_some_and(|rest| rest.contains('.'))
        };
        while !answered(&output) {
            output.extend(client.output_until(session, b"."));
        }
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("got=ESC]11;rgb:1010/1414/1818BEL."), "{text:?}");
        assert_eq!(client.exit_code(session), Some(0));
    }

    /// Spawns a [`printer`] of `bytes`, reads everything it prints and
    /// detaches from it: the session, its program, and what the client saw.
    fn detached_printer(
        dir: &TempDir,
        client: &mut TestClient,
        bytes: &[u8],
        printed: &[u8],
    ) -> (SessionId, u32, Vec<u8>) {
        let (session, pid) = client.spawn(printer(dir, &[bytes], 1000));
        let seen = client.output_until_quiet(session, printed, QUIET);
        client.detach(session);
        (session, pid, seen)
    }

    #[test]
    fn attaching_to_a_full_screen_session_replays_its_screen() {
        let (dir, mut client) = core("attach-full");
        let fixture = full_screen_fixture(1);
        let (session, pid, seen) = detached_printer(&dir, &mut client, &fixture, b"Quit frame 1");
        let expected = snapshot(&replayed(&seen, 80, 24, 1000));

        let mut other = TestClient::hello(&dir.path, "other");
        let (start, screen) =
            other.attach_until(session, 1000, |screen| snapshot(screen) == expected);
        assert_eq!((start.session, start.pid), (session, pid));
        let mode = *screen.term().mode();
        assert!(mode.contains(TermMode::ALT_SCREEN | TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE));
        assert!(!mode.contains(TermMode::SHOW_CURSOR));
        assert_eq!(screen.title().as_deref(), Some("htop-like"));
        let text = rows(&screen);
        assert!(text.iter().any(|row| row.contains("PID USER      CPU%")), "{text:?}");
        assert!(text.iter().any(|row| row.ends_with("Quit frame 1")), "{text:?}");
    }

    #[test]
    fn attaching_replays_the_scrollback() {
        let (dir, mut client) = core("attach-scroll");
        let lines: String = (1..=500).map(|n| format!("line {n}\r\n")).collect();
        let (session, _, seen) =
            detached_printer(&dir, &mut client, lines.as_bytes(), b"line 500");
        let expected = snapshot(&replayed(&seen, 80, 24, 1000));
        let (_, screen) = client.attach_until(session, 1000, |screen| snapshot(screen) == expected);
        let text: Vec<String> = rows(&screen)
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect();
        let want: Vec<String> = (1..=500).map(|n| format!("line {n}")).collect();
        assert_eq!(text, want);
    }

    #[test]
    fn attaching_replays_status_records_and_prompt_marks() {
        let (dir, mut client) = core("attach-marks");
        let fixture = marks_fixture();
        let (session, _, seen) = detached_printer(&dir, &mut client, &fixture, b"app=tests");
        let expected = snapshot(&replayed(&seen, 80, 24, 1000));
        let (_, screen) = client.attach_until(session, 1000, |screen| snapshot(screen) == expected);
        use brindle_term_core::status::State;
        assert_eq!(
            screen.records().get("").map(|record| record.state),
            Some(State::Working)
        );
        let marks = screen.marks();
        assert_eq!(marks.commands().len(), 4);
        assert_eq!(marks.prompt_lines().collect::<Vec<_>>(), [0, 2, 4, 6]);
    }

    #[test]
    fn an_attached_session_cant_be_taken_and_the_attach_size_applies() {
        let (dir, mut owner) = core("attach-refused");
        let (session, _) = owner.spawn(detach(interactive()));
        owner.output_until(session, b"ready>");
        let mut other = TestClient::hello(&dir.path, "other");
        let refusal = other.attach(session, 100, 30).unwrap_err();
        assert_eq!(refusal.session, Some(session));
        assert!(refusal.message.contains("attached"), "{}", refusal.message);
        assert!(
            other
                .attach(999, 80, 24)
                .unwrap_err()
                .message
                .contains("no session 999")
        );
        assert!(
            owner
                .attach(session, 80, 24)
                .unwrap_err()
                .message
                .contains("already attached")
        );

        owner.detach(session);
        let (start, _) = loop {
            match other.attach(session, 100, 30) {
                Ok(replay) => break replay,
                // The DETACH may not have been read yet.
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        };
        assert_eq!(start.session, session);
        becomes_100x30(&mut other, session);
        // The first client no longer has it.
        owner.input(session, b"x");
        match owner.recv() {
            ToClient::Error(error) => assert!(error.message.contains("isn't attached")),
            message => panic!("{message:?}"),
        }
    }

    #[test]
    fn after_a_core_restart_a_full_screen_session_rebuilds_from_checkpoint_and_tail() {
        let mut dir = TempDir::new("rebuild-full");
        start_core(&dir.path, &[SUPERVISED]).unwrap();
        let mut client = TestClient::hello(&dir.path, "first");
        let first = full_screen_fixture(1);
        let second = full_screen_fixture(2);
        // The second frame comes after a line of input.
        let (session, pid) = client.spawn(printer(&dir, &[&first, &second], 1000));
        dir.track(parent(pid).unwrap());
        let mut seen = client.output_until_quiet(session, b"Quit frame 1", QUIET);
        // The checkpoint, of the first frame.
        client.detach(session);
        client.attach(session, 80, 24).unwrap();
        // The tail: the second frame.
        client.input(session, b"\r");
        seen.extend(client.output_until_quiet(session, b"Quit frame 2", QUIET));

        kill_core_and_wait_for_another(&dir, &mut client);
        let mut other = TestClient::hello(&dir.path, "other");
        let expected = snapshot(&replayed(&seen, 80, 24, 1000));
        let (start, screen) =
            other.attach_until(session, 1000, |screen| snapshot(screen) == expected);
        assert_eq!(start.pid, pid);
        assert!(rows(&screen).iter().any(|row| row.ends_with("Quit frame 2")));
        let logs = dir.logs();
        assert!(!logs.contains("truncated"), "{logs}");
    }

    #[test]
    fn a_session_with_more_output_than_a_tail_holds_rebuilds_exactly() {
        let mut dir = TempDir::new("rebuild-long");
        start_core(&dir.path, &[SUPERVISED]).unwrap();
        let mut client = TestClient::hello(&dir.path, "first");
        // More than the 1 MiB tail: only checkpoints keep it exact.
        let file = numbered_lines(&dir.path, 21_000);
        let script = format!("type {}& echo all-printed& set /p z=", file.display());
        let (session, pid) = client.spawn(detach(client_shell(&script)));
        dir.track(parent(pid).unwrap());
        let seen = client.output_until_quiet(session, b"all-printed", QUIET);
        assert!(seen.len() > 1024 * 1024 + 256 * 1024, "{} bytes", seen.len());

        kill_core_and_wait_for_another(&dir, &mut client);
        let mut other = TestClient::hello(&dir.path, "other");
        let expected = snapshot(&replayed(&seen, 80, 24, 1000));
        let (_, screen) = other.attach_until(session, 1000, |screen| snapshot(screen) == expected);
        assert!(rows(&screen).iter().any(|row| row == "all-printed"));
        let logs = dir.logs();
        assert!(!logs.contains("truncated"), "{logs}");
    }

    #[test]
    fn a_truncated_tail_still_gives_a_working_session() {
        let mut dir = TempDir::new("rebuild-truncated");
        start_core(&dir.path, &[]).unwrap();
        let mut client = TestClient::hello(&dir.path, "first");
        // Prints more than a tail holds for every line of input.
        let file = numbered_lines(&dir.path, 21_000);
        let script = format!(
            "for /l %i in (0,0,1) do @(set /p x=& type {}& echo END !x!)",
            file.display()
        );
        let (session, pid) = client.spawn(detach(client_shell(&script)));
        let holder = parent(pid).unwrap();
        dir.track(holder);
        kill(client.core_pid());
        assert!(client.closes());

        // A core that never checkpoints: the tail overflows.
        let mut test_core = TestCore::connect_to(&dir);
        test_core.hello(7);
        test_core.input(session, b"one\r");
        test_core.output_until(session, b"END one");
        drop(test_core);

        start_core(&dir.path, &[]).unwrap();
        let mut other = TestClient::hello(&dir.path, "other");
        dir.track(other.core_pid());
        let (start, screen) = other.attach_until(session, 1000, |screen| {
            rows(screen).iter().any(|row| row == "END one")
        });
        assert_eq!(start.pid, pid);
        assert!(rows(&screen).iter().any(|row| row.starts_with("#00")));
        assert!(dir.logs().contains("tail was truncated"), "{}", dir.logs());
        other.input(session, b"two\r");
        other.output_until(session, b"END two");
    }

    #[test]
    fn a_client_back_after_a_core_restart_gets_its_replay_then_live_output() {
        let mut dir = TempDir::new("reconnect-replay");
        start_core(&dir.path, &[SUPERVISED]).unwrap();
        let mut client = TestClient::hello(&dir.path, "back");
        let first = full_screen_fixture(1);
        let second = full_screen_fixture(2);
        // A kill session, attached all along.
        let mut spawn = printer(&dir, &[&first, &second], 1000);
        spawn.policy = client::Policy::Kill;
        let (session, pid) = client.spawn(spawn);
        dir.track(parent(pid).unwrap());
        let mut seen = client.output_until_quiet(session, b"Quit frame 1", QUIET);

        kill_core_and_wait_for_another(&dir, &mut client);
        let mut client = TestClient::hello(&dir.path, "back");
        let (start, replay) = client.reconnect_replay();
        assert_eq!((start.session, start.pid, start.req), (session, pid, None));
        let mut screen = replayed(&replay, 80, 24, 1000);
        assert_eq!(snapshot(&screen), snapshot(&replayed(&seen, 80, 24, 1000)));

        // Live output follows the replay.
        client.input(session, b"\r");
        let live = client.output_until_quiet(session, b"Quit frame 2", QUIET);
        screen.feed(&live);
        seen.extend(live);
        assert_eq!(snapshot(&screen), snapshot(&replayed(&seen, 80, 24, 1000)));
        assert!(rows(&screen).iter().any(|row| row.ends_with("Quit frame 2")));
        assert!(alive(pid));
    }

    /// A client's `REPARENT` reaches the session's holder, which makes the
    /// window the owner of the session's console window.
    #[test]
    fn reparent_reaches_the_sessions_holder() {
        // Only the packaged ConPTY can; a build without it uses Windows' own.
        if !Path::new(BIN).with_file_name("conpty.dll").exists() {
            eprintln!("skipped: no packaged ConPTY beside the helper");
            return;
        }
        let (dir, mut client) = core("reparent");
        let (session, _) = client.spawn(powershell(CONSOLE_WINDOW));
        let console_window = wait_for_console_window(|| client.output_until(session, b";"));
        assert_eq!(window_owner(console_window), 0);

        // Another client's session isn't this one's to reparent.
        let window = Window::new();
        let mut other = TestClient::hello(&dir.path, "other");
        other.send(client::ToCore::Reparent(client::Reparent {
            session,
            hwnd: window.id(),
        }));
        match other.recv() {
            ToClient::Error(error) => {
                assert_eq!(error.session, Some(session));
                assert!(error.message.contains("isn't attached"), "{}", error.message);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(window_owner(console_window), 0);

        client.send(client::ToCore::Reparent(client::Reparent {
            session,
            hwnd: window.id(),
        }));
        // The host applies it a moment later.
        let deadline = Instant::now() + TIMEOUT;
        while window_owner(console_window) != window.id() {
            assert!(
                Instant::now() < deadline,
                "the owner is {:#x}, not {:#x}",
                window_owner(console_window),
                window.id()
            );
            Window::pump();
            thread::sleep(Duration::from_millis(10));
        }
        // It has no answer: the next message is the close's.
        client.send(client::ToCore::Close(client::SessionRef { session }));
        hung_up(&mut client, session);
    }

    /// The names in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A core removes the helper copies of other builds when it starts,
    /// except one that something still runs from.
    #[test]
    fn a_starting_core_prunes_the_helper_copies_of_other_builds() {
        let (dir, mut client) = core("prune");
        let build = client.welcome.as_ref().unwrap().core_build.clone();
        let core_pid = client.core_pid();
        client.send(client::ToCore::Stop(client::Request { req: 1 }));
        assert!(matches!(client.reply(1), ToClient::Ok(_)));
        assert!(gone(core_pid));

        // With `BRINDLE_SERVER_DIR` set, the copies are in its `bin`.
        let copies = dir.path.join("bin");
        let copy = |name: &str| copies.join(name).join("brindle-remote.exe");
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let cmd = Path::new(&system).join("System32").join("cmd.exe");
        for name in [build.as_str(), "stale", "running"] {
            fs::create_dir_all(copies.join(name)).unwrap();
            fs::copy(&cmd, copy(name)).unwrap();
        }
        // Waits for input that never comes.
        let mut running = std::process::Command::new(copy("running"))
            .arg("/k")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        start_core(&dir.path, &[]).unwrap();
        let pruned = names(&copies);
        running.kill().unwrap();
        running.wait().unwrap();
        let mut kept = [build.as_str(), "running"];
        kept.sort_unstable();
        assert_eq!(pruned, kept);
        assert!(copy(&build).is_file());
    }
}
