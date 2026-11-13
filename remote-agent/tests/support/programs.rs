//! Sessions' programs that the upgrade and management tests share, as each
//! platform runs them: a Unix shell's on Linux, `cmd.exe`'s on Windows.
//! (`tests/core.rs` keeps its own, with more of them.)

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::Spawn;

use super::{TestClient, client_shell, client_spawn};

/// What ends a line of input.
#[cfg(unix)]
pub const ENTER: &[u8] = b"\n";
#[cfg(windows)]
pub const ENTER: &[u8] = b"\r";

/// Answers every line of input with [`echo_of`] it.
#[cfg(unix)]
pub fn echoer() -> Spawn {
    client_spawn("cat", &[])
}

#[cfg(windows)]
pub fn echoer() -> Spawn {
    client_shell("for /l %i in (0,0,1) do @(set /p x=& echo got:!x!)")
}

/// What [`echoer`]'s session prints for the line `word`: on Linux the
/// PTY's echo, then `cat`'s; on Windows the answer after the console's
/// echo.
#[cfg(unix)]
pub fn echo_of(word: &str) -> Vec<u8> {
    format!("{word}\r\n{word}\r\n").into_bytes()
}

#[cfg(windows)]
pub fn echo_of(word: &str) -> Vec<u8> {
    format!("got:{word}").into_bytes()
}

/// Waits for a line of input, then exits with `code`.
#[cfg(unix)]
pub fn exits_after_a_line(code: i32) -> Spawn {
    client_shell(&format!("read x; exit {code}"))
}

#[cfg(windows)]
pub fn exits_after_a_line(code: i32) -> Spawn {
    client_shell(&format!("set /p x=& exit {code}"))
}

/// Runs until it is ended, as the session's own program.
#[cfg(unix)]
pub fn sleeps() -> Spawn {
    client_spawn("sleep", &["1000"])
}

#[cfg(windows)]
pub fn sleeps() -> Spawn {
    client_spawn("ping.exe", &["-n", "1000", "127.0.0.1"])
}

/// Sends the line `word` to an [`echoer`] session and waits for its answer.
pub fn say(client: &mut TestClient, session: SessionId, word: &str) {
    client.input(session, &[word.as_bytes(), ENTER].concat());
    client.output_until(session, &echo_of(word));
}

/// Waits for the `EXIT` of a session that was hung up. A signal ended its
/// program on Linux, so it has no exit code; on Windows it has the one the
/// console's end or the holder ended it with.
pub fn hung_up(client: &mut TestClient, session: SessionId) {
    let code = client.exit_code(session);
    #[cfg(unix)]
    assert_eq!(code, None);
    #[cfg(windows)]
    assert!(code.is_some());
}
