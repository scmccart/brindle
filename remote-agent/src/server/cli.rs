//! The management commands, `brindle-remote list | kill | status | stop`,
//! and the dispatch of those and `check`.
//!
//! The management commands are protocol 1 clients of the core in the
//! current server directory. They never start a core: with none running,
//! `list` prints nothing, `status` says no server is running, and `kill`
//! and `stop` say so on stderr; all of them exit 0.

use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use brindle_remote_protocol::client::{
    self, Hello, Kill, KillTarget, Request, ServerStatus, SessionInfo, ToClient, ToCore,
};
use brindle_remote_protocol::serde_json::{self, Value};
use brindle_remote_protocol::{Frame, RequestId};

use super::platform::conn::Stream;
use super::platform::listen;
use super::{check, dir};

/// How long a command waits for the core's answer. `stop` waits for every
/// session to end, which includes the holders' hang-up grace period.
const REPLY_LIMIT: Duration = Duration::from_secs(60);
const NO_SERVER: &str = "no server is running";

pub const USAGE: &str = "\
usage: brindle-remote <command>
  --version                         print the version
  --server                          serve one session over stdio (remote tabs)
  serve [--ready-fd N]              run the terminal server's core
  hold [--ready-fd N]               run a holder (started by the core)
  check --holder-protocols LIST     exit 0 if this binary drives every H1 version in LIST
  list [--json]                     the server's sessions: id, pid, state, policy, age, title, command, directory
  kill ID | kill --all              hang up one session, or all of them
  status [--json]                   the core and its holders
  stop                              hang up every session and stop the server
The server directory is $BRINDLE_SERVER_DIR, else $XDG_RUNTIME_DIR/brindle,
else ~/.cache/brindle/run.";

/// Runs a management command or `check`; `None` if `command` is neither.
pub fn run(command: &str, args: &[String]) -> Option<Result<(), String>> {
    Some(match command {
        "check" => check::run(args),
        "list" => json_flag("list", args).and_then(list),
        "kill" => kill(args),
        "status" => json_flag("status", args).and_then(status),
        "stop" => no_args("stop", args).and_then(|()| stop()),
        _ => return None,
    })
}

fn no_args(command: &str, args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(format!("{command} takes no arguments\n{USAGE}"))
    }
}

fn json_flag(command: &str, args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--json" => Ok(true),
        _ => Err(format!("usage: brindle-remote {command} [--json]")),
    }
}

fn list(json: bool) -> Result<(), String> {
    let Some(mut core) = Connection::open()? else {
        return Ok(());
    };
    let reply = core.request(|req| ToCore::List(Request { req }))?;
    let ToClient::Sessions(sessions) = reply else {
        return Err(unexpected(&reply));
    };
    if json {
        let text = serde_json::to_string_pretty(&sessions.sessions).map_err(|e| e.to_string())?;
        println!("{text}");
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        for session in &sessions.sessions {
            println!("{}", list_line(session, now));
        }
    }
    Ok(())
}

/// One `list` line: id, pid, attached or detached, policy, age (`-` from a
/// core that doesn't report it), title (`-` when none), command, directory.
fn list_line(session: &SessionInfo, now: u64) -> String {
    let state = if session.attached {
        "attached"
    } else {
        "detached"
    };
    let command = if session.command.is_empty() {
        "(login shell)"
    } else {
        &session.command
    };
    let policy = match session.policy {
        client::Policy::Kill => "kill",
        client::Policy::Detach => "detach",
    };
    let age = match session.started {
        0 => "-".to_string(),
        started => uptime(now.saturating_sub(started)),
    };
    let title = if session.title.is_empty() { "-" } else { &session.title };
    format!(
        "{}\t{}\t{state}\t{policy}\t{age}\t{title}\t{command}\t{}",
        session.session,
        session.pid,
        session.cwd.as_deref().unwrap_or("-")
    )
}

fn kill(args: &[String]) -> Result<(), String> {
    let target = match args {
        [all] if all == "--all" => KillTarget::All,
        [id] => KillTarget::Session(
            id.parse()
                .map_err(|_| format!("kill: {id:?} isn't a session id"))?,
        ),
        _ => return Err("usage: brindle-remote kill ID | kill --all".into()),
    };
    let Some(mut core) = Connection::open()? else {
        eprintln!("{NO_SERVER}");
        return Ok(());
    };
    let reply = core.request(|req| {
        ToCore::Kill(Kill {
            req,
            target: target.clone(),
        })
    })?;
    match reply {
        ToClient::Ok(_) => Ok(()),
        other => Err(unexpected(&other)),
    }
}

fn stop() -> Result<(), String> {
    let Some(mut core) = Connection::open()? else {
        eprintln!("{NO_SERVER}");
        return Ok(());
    };
    match core.request(|req| ToCore::Stop(Request { req }))? {
        ToClient::Ok(_) => Ok(()),
        other => Err(unexpected(&other)),
    }
}

fn status(json: bool) -> Result<(), String> {
    let Some(mut core) = Connection::open()? else {
        if json {
            println!("{}", serde_json::json!({ "running": false }));
        } else {
            println!("{NO_SERVER}");
        }
        return Ok(());
    };
    let reply = core.request(|req| ToCore::Status(Request { req }))?;
    let ToClient::ServerStatus(status) = reply else {
        return Err(unexpected(&reply));
    };
    if json {
        println!("{}", status_json(&status)?);
    } else {
        print!("{}", status_text(&status));
    }
    Ok(())
}

/// `status --json`: `SERVER_STATUS` without its `req`, and `running`.
fn status_json(status: &ServerStatus) -> Result<String, String> {
    let mut value = serde_json::to_value(status).map_err(|error| error.to_string())?;
    if let Value::Object(object) = &mut value {
        object.remove("req");
        object.insert("running".into(), Value::Bool(true));
    }
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

fn status_text(status: &ServerStatus) -> String {
    let mut text = format!(
        "core: version {}, build {}, pid {}, up {}, {} client(s)\n\
         protocols: client {}, holder H{}\n",
        status.core_version,
        status.core_build,
        status.pid,
        uptime(status.uptime_secs),
        status.clients,
        status.protocols,
        status.holder_protocols
    );
    for holder in &status.holders {
        text.push_str(&format!(
            "holder: pid {}, build {}, H{}, {} session(s){}\n",
            holder.pid,
            holder.build,
            holder.protocol,
            holder.sessions,
            if holder.draining { ", draining" } else { "" }
        ));
    }
    text
}

fn uptime(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

fn unexpected(reply: &ToClient) -> String {
    match reply {
        ToClient::Error(error) => error.message.clone(),
        other => format!("unexpected answer from the core: {other:?}"),
    }
}

/// A protocol 1 connection to the core, past its `WELCOME`.
struct Connection {
    stream: Stream,
    next_req: RequestId,
}

impl Connection {
    /// Connects and says `HELLO`; `None` when no core is listening.
    fn open() -> Result<Option<Self>, String> {
        let socket = dir::resolve_from_env()
            .map_err(|error| error.to_string())?
            .join("core.sock");
        let stream = match listen::connect(&socket) {
            Ok(stream) => stream,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(format!("can't connect to {}: {error}", socket.display())),
        };
        stream
            .set_read_timeout(Some(REPLY_LIMIT))
            .map_err(|error| error.to_string())?;
        let mut connection = Self {
            stream,
            next_req: 1,
        };
        connection.send(&ToCore::Hello(Hello {
            min: client::PROTOCOLS.min,
            max: client::PROTOCOLS.max,
            client_version: format!("brindle-remote {}", env!("CARGO_PKG_VERSION")),
            client_token: cli_token(),
        }))?;
        match connection.recv()? {
            ToClient::Welcome(_) => Ok(Some(connection)),
            other => Err(unexpected(&other)),
        }
    }

    fn send(&mut self, message: &ToCore) -> Result<(), String> {
        message
            .encode()
            .and_then(|frame| frame.write_to(&mut self.stream))
            .map_err(|error| format!("can't talk to the core: {error}"))
    }

    fn recv(&mut self) -> Result<ToClient, String> {
        match Frame::read_from_or_eof(&mut self.stream) {
            Ok(Some(frame)) => ToClient::decode(&frame).map_err(|error| error.to_string()),
            Ok(None) => Err("the core closed the connection".into()),
            Err(error) => Err(format!("no answer from the core: {error}")),
        }
    }

    /// Sends a request and waits for its reply, skipping anything else.
    fn request(&mut self, message: impl FnOnce(RequestId) -> ToCore) -> Result<ToClient, String> {
        let req = self.next_req;
        self.next_req += 1;
        self.send(&message(req))?;
        loop {
            let reply = self.recv()?;
            let matches = match &reply {
                ToClient::Ok(ok) => ok.req == req,
                ToClient::Error(error) => error.req == Some(req),
                ToClient::Sessions(sessions) => sessions.req == req,
                ToClient::ServerStatus(status) => status.req == req,
                _ => false,
            };
            if matches {
                return Ok(reply);
            }
        }
    }
}

/// A token no session is owned by, so the core gives this connection none.
fn cli_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("brindle-remote-cli-{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_lines() {
        let mut session = SessionInfo {
            session: 3,
            pid: 4242,
            attached: true,
            command: "htop -d 5".into(),
            cwd: Some("/home/u".into()),
            policy: client::Policy::Kill,
            cols: 80,
            rows: 24,
            title: "htop".into(),
            started: 1000,
            ..Default::default()
        };
        assert_eq!(
            list_line(&session, 1065),
            "3\t4242\tattached\tkill\t1m05s\thtop\thtop -d 5\t/home/u"
        );
        session.attached = false;
        session.policy = client::Policy::Detach;
        session.command.clear();
        session.cwd = None;
        session.title.clear();
        session.started = 0;
        assert_eq!(
            list_line(&session, 1065),
            "3\t4242\tdetached\tdetach\t-\t-\t(login shell)\t-"
        );
    }

    #[test]
    fn uptimes() {
        assert_eq!(uptime(5), "5s");
        assert_eq!(uptime(65), "1m05s");
        assert_eq!(uptime(7322), "2h02m");
    }
}
