//! The terminal server's client protocol, between a client (Brindle, or the
//! `brindle-remote` management commands) and the core. This is protocol 1.
//!
//! Frames use the crate's header (`type: u8`, `len: u32 BE`). `DATA` is
//! binary: a `u32` BE session id, then the bytes (input from the client,
//! output from the core). Every other message is a JSON object; unknown
//! fields are ignored and missing optional fields take their defaults, so
//! fields can be added within a version. A new message type needs a new
//! version.
//!
//! | type | name            | direction     | payload                                                        |
//! |-----:|-----------------|---------------|----------------------------------------------------------------|
//! |    1 | `DATA`          | both          | session, bytes                                                 |
//! |    2 | `HELLO`         | client → core | `min, max, client_version, client_token`                       |
//! |    3 | `SPAWN`         | client → core | `req, command, args, cwd, env, cols, rows, cell_w, cell_h, scrollback, policy` |
//! |    4 | `RESIZE`        | client → core | `session, cols, rows, cell_w, cell_h`                          |
//! |    5 | `CLOSE`         | client → core | `session`                                                      |
//! |    6 | `ATTACH`        | client → core | `req, session, cols, rows`                                     |
//! |    7 | `DETACH`        | client → core | `session`                                                      |
//! |    8 | `LIST`          | client → core | `req`                                                          |
//! |    9 | `KILL`          | client → core | `req, target: {"session": id} \| "all"`                        |
//! |   10 | `STATUS`        | client → core | `req`                                                          |
//! |   11 | `STOP`          | client → core | `req`                                                          |
//! |   12 | `STEP_DOWN`     | client → core | `req`                                                          |
//! |   13 | `REPARENT`      | client → core | `session, hwnd` (reserved; a Linux core refuses it)            |
//! |   32 | `WELCOME`       | core → client | `protocol, core_version, core_build, pid, holder_protocols`    |
//! |   33 | `ERROR`         | core → client | `req?, session?, message`                                      |
//! |   34 | `SPAWNED`       | core → client | `req, session, pid`                                            |
//! |   35 | `EXIT`          | core → client | `session, code?`                                               |
//! |   36 | `REPLAY_START`  | core → client | `req?, session, pid`                                           |
//! |   37 | `REPLAY_END`    | core → client | `session`                                                      |
//! |   38 | `SESSIONS`      | core → client | `req, sessions: [{session, pid, attached, command, cwd, policy, cols, rows}]` |
//! |   39 | `SERVER_STATUS` | core → client | `req, core_version, core_build, protocols, holder_protocols, pid, uptime_secs, clients, holders` |
//! |   40 | `OK`            | core → client | `req`                                                          |
//!
//! Exchanges:
//! - `HELLO` → `WELCOME`, or `ERROR` naming both ranges ([`negotiate`]).
//!   After `WELCOME`, the core replays each session it still holds for the
//!   client's `client_token` (`REPLAY_START` without `req`, `DATA`…,
//!   `REPLAY_END`).
//! - `SPAWN` → `SPAWNED` or `ERROR`; `ATTACH` → `REPLAY_START`, `DATA`…,
//!   `REPLAY_END`, or `ERROR`.
//! - `LIST` → `SESSIONS`; `STATUS` → `SERVER_STATUS`; `KILL`, `STOP` and
//!   `STEP_DOWN` → `OK` or `ERROR`.
//! - `RESIZE`, `CLOSE`, `DETACH` and `REPARENT` have no reply; a refused one
//!   gets an `ERROR` with its `session`.

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

use crate::Frame;
use crate::wire::{self, RequestId, SessionId};

pub use crate::version::{NegotiationError, VersionRange, negotiate};

/// The client protocol versions this build speaks.
pub const PROTOCOLS: VersionRange = VersionRange::new(1, 1);

pub const MSG_DATA: u8 = 1;
pub const MSG_HELLO: u8 = 2;
pub const MSG_SPAWN: u8 = 3;
pub const MSG_RESIZE: u8 = 4;
pub const MSG_CLOSE: u8 = 5;
pub const MSG_ATTACH: u8 = 6;
pub const MSG_DETACH: u8 = 7;
pub const MSG_LIST: u8 = 8;
pub const MSG_KILL: u8 = 9;
pub const MSG_STATUS: u8 = 10;
pub const MSG_STOP: u8 = 11;
pub const MSG_STEP_DOWN: u8 = 12;
pub const MSG_REPARENT: u8 = 13;
pub const MSG_WELCOME: u8 = 32;
pub const MSG_ERROR: u8 = 33;
pub const MSG_SPAWNED: u8 = 34;
pub const MSG_EXIT: u8 = 35;
pub const MSG_REPLAY_START: u8 = 36;
pub const MSG_REPLAY_END: u8 = 37;
pub const MSG_SESSIONS: u8 = 38;
pub const MSG_SERVER_STATUS: u8 = 39;
pub const MSG_OK: u8 = 40;

/// Bytes for one session: input from the client, output from the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    pub session: SessionId,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub min: u32,
    pub max: u32,
    /// The client's release, for the core's log and `status`.
    #[serde(default)]
    pub client_version: String,
    /// Random per Brindle process; a reconnecting client gets back the
    /// sessions it owns under this token.
    pub client_token: String,
}

impl Hello {
    pub fn range(&self) -> VersionRange {
        VersionRange::new(self.min, self.max)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// The negotiated version; everything after `WELCOME` uses it.
    pub protocol: u32,
    pub core_version: String,
    pub core_build: String,
    pub pid: u32,
    /// The H1 versions of the holders the core is driving.
    #[serde(default)]
    pub holder_protocols: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// The session ends when its client closes it or is gone.
    #[default]
    Kill,
    /// The session keeps running unattached when its client is gone.
    Detach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawn {
    pub req: RequestId,
    /// `None` runs the user's login shell.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The program's complete environment; the server adds nothing.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub cell_w: u16,
    #[serde(default)]
    pub cell_h: u16,
    /// The emulator's scrollback limit, in lines.
    #[serde(default)]
    pub scrollback: u32,
    #[serde(default)]
    pub policy: Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawned {
    pub req: RequestId,
    pub session: SessionId,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resize {
    pub session: SessionId,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub cell_w: u16,
    #[serde(default)]
    pub cell_h: u16,
}

/// A message that names only a session: `CLOSE`, `DETACH`, `REPLAY_END`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub session: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    pub session: SessionId,
    /// `None` when the program was ended by a signal.
    #[serde(default)]
    pub code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attach {
    pub req: RequestId,
    pub session: SessionId,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayStart {
    /// The `ATTACH` this answers; `None` for a reconnect's replays.
    #[serde(default)]
    pub req: Option<RequestId>,
    pub session: SessionId,
    pub pid: u32,
}

/// A request with nothing but its id: `LIST`, `STATUS`, `STOP`, `STEP_DOWN`,
/// and the `OK` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub req: RequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillTarget {
    Session(SessionId),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kill {
    pub req: RequestId,
    pub target: KillTarget,
}

/// Reserved in protocol 1 for Windows; a Linux core answers with `ERROR`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reparent {
    pub session: SessionId,
    pub hwnd: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMessage {
    #[serde(default)]
    pub req: Option<RequestId>,
    #[serde(default)]
    pub session: Option<SessionId>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session: SessionId,
    pub pid: u32,
    #[serde(default)]
    pub attached: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub cols: u16,
    #[serde(default)]
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sessions {
    pub req: RequestId,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderStatus {
    pub pid: u32,
    #[serde(default)]
    pub build: String,
    /// The H1 version the core drives this holder in.
    #[serde(default)]
    pub protocol: u32,
    #[serde(default)]
    pub sessions: u32,
    #[serde(default)]
    pub draining: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub req: RequestId,
    pub core_version: String,
    #[serde(default)]
    pub core_build: String,
    /// The client protocol versions the core speaks.
    pub protocols: VersionRange,
    /// The H1 versions the core speaks.
    pub holder_protocols: VersionRange,
    pub pid: u32,
    #[serde(default)]
    pub uptime_secs: u64,
    #[serde(default)]
    pub clients: u32,
    #[serde(default)]
    pub holders: Vec<HolderStatus>,
}

/// Messages from a client to the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToCore {
    Hello(Hello),
    Spawn(Spawn),
    /// Input for a session.
    Data(Data),
    Resize(Resize),
    Close(SessionRef),
    Attach(Attach),
    Detach(SessionRef),
    List(Request),
    Kill(Kill),
    Status(Request),
    Stop(Request),
    StepDown(Request),
    Reparent(Reparent),
}

impl ToCore {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Hello(m) => wire::json(MSG_HELLO, m),
            Self::Spawn(m) => wire::json(MSG_SPAWN, m),
            Self::Data(m) => wire::session_bytes(MSG_DATA, m.session, &m.bytes),
            Self::Resize(m) => wire::json(MSG_RESIZE, m),
            Self::Close(m) => wire::json(MSG_CLOSE, m),
            Self::Attach(m) => wire::json(MSG_ATTACH, m),
            Self::Detach(m) => wire::json(MSG_DETACH, m),
            Self::List(m) => wire::json(MSG_LIST, m),
            Self::Kill(m) => wire::json(MSG_KILL, m),
            Self::Status(m) => wire::json(MSG_STATUS, m),
            Self::Stop(m) => wire::json(MSG_STOP, m),
            Self::StepDown(m) => wire::json(MSG_STEP_DOWN, m),
            Self::Reparent(m) => wire::json(MSG_REPARENT, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_HELLO => Self::Hello(wire::from_json("HELLO", frame)?),
            MSG_SPAWN => Self::Spawn(wire::from_json("SPAWN", frame)?),
            MSG_DATA => Self::Data(decode_data(frame)?),
            MSG_RESIZE => Self::Resize(wire::from_json("RESIZE", frame)?),
            MSG_CLOSE => Self::Close(wire::from_json("CLOSE", frame)?),
            MSG_ATTACH => Self::Attach(wire::from_json("ATTACH", frame)?),
            MSG_DETACH => Self::Detach(wire::from_json("DETACH", frame)?),
            MSG_LIST => Self::List(wire::from_json("LIST", frame)?),
            MSG_KILL => Self::Kill(wire::from_json("KILL", frame)?),
            MSG_STATUS => Self::Status(wire::from_json("STATUS", frame)?),
            MSG_STOP => Self::Stop(wire::from_json("STOP", frame)?),
            MSG_STEP_DOWN => Self::StepDown(wire::from_json("STEP_DOWN", frame)?),
            MSG_REPARENT => Self::Reparent(wire::from_json("REPARENT", frame)?),
            other => return Err(wire::unknown_type("client-to-core", other)),
        })
    }
}

/// Messages from the core to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToClient {
    Welcome(Welcome),
    Error(ErrorMessage),
    Spawned(Spawned),
    /// Output from a session, live or within a replay.
    Data(Data),
    Exit(Exit),
    ReplayStart(ReplayStart),
    ReplayEnd(SessionRef),
    Sessions(Sessions),
    ServerStatus(ServerStatus),
    Ok(Request),
}

impl ToClient {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Welcome(m) => wire::json(MSG_WELCOME, m),
            Self::Error(m) => wire::json(MSG_ERROR, m),
            Self::Spawned(m) => wire::json(MSG_SPAWNED, m),
            Self::Data(m) => wire::session_bytes(MSG_DATA, m.session, &m.bytes),
            Self::Exit(m) => wire::json(MSG_EXIT, m),
            Self::ReplayStart(m) => wire::json(MSG_REPLAY_START, m),
            Self::ReplayEnd(m) => wire::json(MSG_REPLAY_END, m),
            Self::Sessions(m) => wire::json(MSG_SESSIONS, m),
            Self::ServerStatus(m) => wire::json(MSG_SERVER_STATUS, m),
            Self::Ok(m) => wire::json(MSG_OK, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_WELCOME => Self::Welcome(wire::from_json("WELCOME", frame)?),
            MSG_ERROR => Self::Error(wire::from_json("ERROR", frame)?),
            MSG_SPAWNED => Self::Spawned(wire::from_json("SPAWNED", frame)?),
            MSG_DATA => Self::Data(decode_data(frame)?),
            MSG_EXIT => Self::Exit(wire::from_json("EXIT", frame)?),
            MSG_REPLAY_START => Self::ReplayStart(wire::from_json("REPLAY_START", frame)?),
            MSG_REPLAY_END => Self::ReplayEnd(wire::from_json("REPLAY_END", frame)?),
            MSG_SESSIONS => Self::Sessions(wire::from_json("SESSIONS", frame)?),
            MSG_SERVER_STATUS => Self::ServerStatus(wire::from_json("SERVER_STATUS", frame)?),
            MSG_OK => Self::Ok(wire::from_json("OK", frame)?),
            other => return Err(wire::unknown_type("core-to-client", other)),
        })
    }
}

fn decode_data(frame: &Frame) -> io::Result<Data> {
    let (session, bytes) = wire::split_session("DATA", frame)?;
    Ok(Data {
        session,
        bytes: bytes.to_vec(),
    })
}

/// The core's answer to a `HELLO`: the version both speak, or the `ERROR`
/// to send back (naming both ranges) before closing the connection.
pub fn answer_hello(hello: &Hello, supported: VersionRange) -> Result<u32, ErrorMessage> {
    negotiate(hello.range(), supported).map_err(|error| ErrorMessage {
        req: None,
        session: None,
        message: error.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod samples {
    use super::*;

    pub fn to_core() -> Vec<ToCore> {
        vec![
            ToCore::Hello(Hello {
                min: 1,
                max: 1,
                client_version: "0.3.0".into(),
                client_token: "4f1c9a".into(),
            }),
            ToCore::Spawn(Spawn {
                req: 1,
                command: Some("/bin/bash".into()),
                args: vec!["-l".into()],
                cwd: Some("/home/u".into()),
                env: BTreeMap::from([("TERM".into(), "xterm-256color".into())]),
                cols: 80,
                rows: 24,
                cell_w: 9,
                cell_h: 18,
                scrollback: 10_000,
                policy: Policy::Detach,
            }),
            ToCore::Data(Data {
                session: 7,
                bytes: b"ls\r\0\xff".to_vec(),
            }),
            ToCore::Data(Data {
                session: u32::MAX,
                bytes: Vec::new(),
            }),
            ToCore::Resize(Resize {
                session: 7,
                cols: 120,
                rows: 40,
                cell_w: 10,
                cell_h: 20,
            }),
            ToCore::Close(SessionRef { session: 7 }),
            ToCore::Attach(Attach {
                req: 2,
                session: 3,
                cols: 100,
                rows: 30,
            }),
            ToCore::Detach(SessionRef { session: 3 }),
            ToCore::List(Request { req: 3 }),
            ToCore::Kill(Kill {
                req: 4,
                target: KillTarget::Session(3),
            }),
            ToCore::Kill(Kill {
                req: 5,
                target: KillTarget::All,
            }),
            ToCore::Status(Request { req: 6 }),
            ToCore::Stop(Request { req: 7 }),
            ToCore::StepDown(Request { req: 8 }),
            ToCore::Reparent(Reparent {
                session: 7,
                hwnd: 0x0001_0a2c,
            }),
        ]
    }

    pub fn to_client() -> Vec<ToClient> {
        vec![
            ToClient::Welcome(Welcome {
                protocol: 1,
                core_version: "0.3.0".into(),
                core_build: "abc123".into(),
                pid: 4242,
                holder_protocols: vec![1],
            }),
            ToClient::Error(ErrorMessage {
                req: Some(2),
                session: Some(3),
                message: "session 3 is attached".into(),
            }),
            ToClient::Error(ErrorMessage {
                req: None,
                session: None,
                message: "no common protocol version".into(),
            }),
            ToClient::Spawned(Spawned {
                req: 1,
                session: 7,
                pid: 5000,
            }),
            ToClient::Data(Data {
                session: 7,
                bytes: b"\x1b[1mhi\x1b[m\r\n".to_vec(),
            }),
            ToClient::Exit(Exit {
                session: 7,
                code: Some(3),
            }),
            ToClient::Exit(Exit {
                session: 8,
                code: None,
            }),
            ToClient::ReplayStart(ReplayStart {
                req: Some(2),
                session: 3,
                pid: 5001,
            }),
            ToClient::ReplayEnd(SessionRef { session: 3 }),
            ToClient::Sessions(Sessions {
                req: 3,
                sessions: vec![SessionInfo {
                    session: 3,
                    pid: 5001,
                    attached: false,
                    command: "htop".into(),
                    cwd: Some("/tmp".into()),
                    policy: Policy::Detach,
                    cols: 100,
                    rows: 30,
                }],
            }),
            ToClient::ServerStatus(ServerStatus {
                req: 6,
                core_version: "0.3.0".into(),
                core_build: "abc123".into(),
                protocols: PROTOCOLS,
                holder_protocols: VersionRange::new(1, 1),
                pid: 4242,
                uptime_secs: 61,
                clients: 2,
                holders: vec![HolderStatus {
                    pid: 4243,
                    build: "abc123".into(),
                    protocol: 1,
                    sessions: 2,
                    draining: false,
                }],
            }),
            ToClient::Ok(Request { req: 7 }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::samples::*;
    use super::*;

    fn roundtrip_wire<T>(message: &T, encode: impl Fn(&T) -> io::Result<Frame>) -> Frame {
        let frame = encode(message).unwrap();
        let mut bytes = Vec::new();
        frame.write_to(&mut bytes).unwrap();
        Frame::read_from(&mut bytes.as_slice()).unwrap()
    }

    #[test]
    fn every_client_message_roundtrips() {
        let messages = to_core();
        let mut types: Vec<u8> = Vec::new();
        for message in &messages {
            let frame = roundtrip_wire(message, ToCore::encode);
            types.push(frame.msg_type);
            assert_eq!(&ToCore::decode(&frame).unwrap(), message);
        }
        for expected in MSG_DATA..=MSG_REPARENT {
            assert!(types.contains(&expected), "no sample of type {expected}");
        }
    }

    #[test]
    fn every_core_message_roundtrips() {
        let mut types: Vec<u8> = Vec::new();
        for message in &to_client() {
            let frame = roundtrip_wire(message, ToClient::encode);
            types.push(frame.msg_type);
            assert_eq!(&ToClient::decode(&frame).unwrap(), message);
        }
        assert!(types.contains(&MSG_DATA));
        for expected in MSG_WELCOME..=MSG_OK {
            assert!(types.contains(&expected), "no sample of type {expected}");
        }
    }

    #[test]
    fn data_starts_with_the_session_id() {
        let frame = ToClient::Data(Data {
            session: 0x0000_0102,
            bytes: b"ok".to_vec(),
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, MSG_DATA);
        assert_eq!(frame.payload, [0, 0, 1, 2, b'o', b'k']);

        let short = Frame::new(MSG_DATA, vec![0, 0, 1]).unwrap();
        assert_eq!(
            ToCore::decode(&short).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(ToClient::decode(&short).is_err());
    }

    #[test]
    fn json_messages_use_the_documented_field_names() {
        let frame = ToCore::Kill(Kill {
            req: 4,
            target: KillTarget::Session(3),
        })
        .encode()
        .unwrap();
        assert_eq!(frame.payload, br#"{"req":4,"target":{"session":3}}"#);
        let frame = ToCore::Kill(Kill {
            req: 5,
            target: KillTarget::All,
        })
        .encode()
        .unwrap();
        assert_eq!(frame.payload, br#"{"req":5,"target":"all"}"#);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let frame = Frame::new(
            MSG_HELLO,
            br#"{"min":1,"max":2,"client_version":"9","client_token":"t","future":{"x":[1]}}"#
                .to_vec(),
        )
        .unwrap();
        assert_eq!(
            ToCore::decode(&frame).unwrap(),
            ToCore::Hello(Hello {
                min: 1,
                max: 2,
                client_version: "9".into(),
                client_token: "t".into(),
            })
        );
        let frame = Frame::new(MSG_OK, br#"{"req":9,"note":"later"}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::Ok(Request { req: 9 })
        );
    }

    #[test]
    fn missing_optional_fields_take_defaults() {
        let frame = Frame::new(MSG_SPAWN, br#"{"req":1,"cols":80,"rows":24}"#.to_vec()).unwrap();
        assert_eq!(
            ToCore::decode(&frame).unwrap(),
            ToCore::Spawn(Spawn {
                req: 1,
                command: None,
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
                cols: 80,
                rows: 24,
                cell_w: 0,
                cell_h: 0,
                scrollback: 0,
                policy: Policy::Kill,
            })
        );
        let frame = Frame::new(MSG_ERROR, br#"{"message":"m"}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::Error(ErrorMessage {
                req: None,
                session: None,
                message: "m".into(),
            })
        );
        let frame = Frame::new(MSG_REPLAY_START, br#"{"session":2,"pid":9}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::ReplayStart(ReplayStart {
                req: None,
                session: 2,
                pid: 9,
            })
        );
    }

    #[test]
    fn required_fields_and_bad_json_are_invalid_data() {
        for (msg_type, payload) in [
            (MSG_SPAWN, &br#"{"cols":80,"rows":24}"#[..]),
            (MSG_RESIZE, br#"{"cols":80,"rows":24}"#),
            (MSG_KILL, br#"{"req":1,"target":"some"}"#),
            (MSG_HELLO, br#"{"min":-1,"max":1,"client_token":"t"}"#),
            (MSG_LIST, b""),
            (MSG_LIST, b"\"req\""),
            (MSG_LIST, br#"{"req":"1"}"#),
            (MSG_STOP, b"{\"req\":1"),
        ] {
            let frame = Frame::new(msg_type, payload.to_vec()).unwrap();
            let error = ToCore::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        }
    }

    #[test]
    fn unknown_and_wrong_direction_types_are_invalid_data() {
        for msg_type in [0, 14, 31, MSG_WELCOME, MSG_OK, 41, 255] {
            let frame = Frame::new(msg_type, b"{}".to_vec()).unwrap();
            let error = ToCore::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        for msg_type in [0, MSG_HELLO, MSG_REPARENT, 31, 41, 255] {
            let frame = Frame::new(msg_type, b"{}".to_vec()).unwrap();
            assert!(ToClient::decode(&frame).is_err());
        }
    }

    #[test]
    fn hello_is_answered_with_the_highest_common_version() {
        let hello = |min, max| Hello {
            min,
            max,
            client_version: String::new(),
            client_token: "t".into(),
        };
        assert_eq!(answer_hello(&hello(1, 1), PROTOCOLS), Ok(1));
        assert_eq!(answer_hello(&hello(1, 4), VersionRange::new(1, 3)), Ok(3));
        let error = answer_hello(&hello(2, 3), PROTOCOLS).unwrap_err();
        assert!(error.message.contains("2–3"), "{}", error.message);
        assert!(error.message.contains("supported 1"), "{}", error.message);
        assert_eq!(error.req, None);
    }
}
