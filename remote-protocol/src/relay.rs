//! The link between Brindle and a `brindle-remote relay` on a remote host.
//!
//! The relay pipes the client protocol ([`crate::client`]) between its stdio
//! and its host's core, so the link carries protocol 1 and 2 frames as they
//! are. It adds three frame types of its own, on numbers in a high range that
//! no other protocol uses, and handles them itself rather than passing them
//! on:
//!
//! | type | name              | direction        | payload                                                         |
//! |-----:|-------------------|------------------|-----------------------------------------------------------------|
//! | 0xF0 | `RELAY_HOST`      | relay → Brindle  | `version, build, env, shell, home, server_dir, core_pid`        |
//! | 0xF1 | `RELAY_HEARTBEAT` | both             | empty                                                           |
//! | 0xF2 | `RELAY_ERROR`     | relay → Brindle  | `message`                                                       |
//!
//! `RELAY_HOST` is the relay's first frame once it reached the core, or
//! `RELAY_ERROR` if it could not (then the relay exits). Either side sends
//! `RELAY_HEARTBEAT` after [`crate::HEARTBEAT_INTERVAL`] without writing, and
//! both end the link after a longer silence. JSON fields follow the client
//! protocol's rule: unknown fields are ignored and missing optional ones take
//! their defaults.

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

use crate::Frame;
use crate::wire;

pub const MSG_RELAY_HOST: u8 = 0xF0;
pub const MSG_RELAY_HEARTBEAT: u8 = 0xF1;
pub const MSG_RELAY_ERROR: u8 = 0xF2;

/// Whether `msg_type` is one of the relay's own frames, to handle rather than
/// pass between the link and the core.
pub const fn is_relay_type(msg_type: u8) -> bool {
    matches!(msg_type, MSG_RELAY_HOST..=MSG_RELAY_ERROR)
}

/// What the relay knows about its host, sent once the core is reached.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayHost {
    /// The relay's release.
    #[serde(default)]
    pub version: String,
    /// The relay's build id.
    #[serde(default)]
    pub build: String,
    /// The relay's own environment: a fresh login's, which Brindle builds
    /// a remote program's environment from.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// The login shell, from `SHELL`, else the passwd entry.
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub home: String,
    /// The server directory the core runs in.
    #[serde(default)]
    pub server_dir: String,
    /// The core's process id on the host.
    #[serde(default)]
    pub core_pid: u32,
}

/// The relay could not reach or start the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayError {
    /// What went wrong, with the core log's path if there is one.
    pub message: String,
}

/// A frame of the relay's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayMessage {
    Host(RelayHost),
    Heartbeat,
    Error(RelayError),
}

impl RelayMessage {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Host(m) => wire::json(MSG_RELAY_HOST, m),
            Self::Heartbeat => Frame::new(MSG_RELAY_HEARTBEAT, Vec::new()),
            Self::Error(m) => wire::json(MSG_RELAY_ERROR, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_RELAY_HOST => Self::Host(wire::from_json("RELAY_HOST", frame)?),
            MSG_RELAY_HEARTBEAT => {
                if !frame.payload.is_empty() {
                    return Err(wire::invalid(format!(
                        "RELAY_HEARTBEAT carries {} bytes, not none",
                        frame.payload.len()
                    )));
                }
                Self::Heartbeat
            }
            MSG_RELAY_ERROR => Self::Error(wire::from_json("RELAY_ERROR", frame)?),
            other => return Err(wire::unknown_type("relay", other)),
        })
    }
}

#[cfg(test)]
pub(crate) mod samples {
    use super::*;

    pub fn messages() -> Vec<RelayMessage> {
        vec![
            RelayMessage::Host(RelayHost {
                version: "0.3.0".into(),
                build: "b1d0".into(),
                env: BTreeMap::from([
                    ("HOME".into(), "/home/u".into()),
                    ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                ]),
                shell: "/bin/bash".into(),
                home: "/home/u".into(),
                server_dir: "/run/user/1000/brindle".into(),
                core_pid: 4242,
            }),
            RelayMessage::Heartbeat,
            RelayMessage::Error(RelayError {
                message: "no server directory: $HOME is unset; core log /tmp/brindle/core.log".into(),
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::samples::*;
    use super::*;
    use crate::client;

    #[test]
    fn every_relay_message_roundtrips() {
        let mut types = Vec::new();
        for message in messages() {
            let frame = message.encode().unwrap();
            let mut bytes = Vec::new();
            frame.write_to(&mut bytes).unwrap();
            let frame = Frame::read_from(&mut bytes.as_slice()).unwrap();
            types.push(frame.msg_type);
            assert_eq!(RelayMessage::decode(&frame).unwrap(), message);
        }
        for expected in MSG_RELAY_HOST..=MSG_RELAY_ERROR {
            assert!(types.contains(&expected), "no sample of type {expected:#x}");
        }
    }

    #[test]
    fn relay_numbers_collide_with_no_other_protocol() {
        let relay = [MSG_RELAY_HOST, MSG_RELAY_HEARTBEAT, MSG_RELAY_ERROR];
        let stdio = [
            crate::MSG_DATA,
            crate::MSG_RESIZE,
            crate::MSG_EXIT,
            crate::MSG_QUERY_PROCESS,
            crate::MSG_PROCESS_INFO,
            crate::MSG_SPAWN,
            crate::MSG_SPAWN_RESPONSE,
            crate::MSG_ERROR,
            crate::MSG_HEARTBEAT,
        ];
        // Everything the client protocol uses: 1-13 and 32-41 (`PROCESS`).
        let client_numbers =
            (client::MSG_DATA..=client::MSG_REPARENT).chain(client::MSG_WELCOME..=client::MSG_PROCESS);
        for number in client_numbers.chain(stdio) {
            assert!(!relay.contains(&number), "type {number:#x} is used twice");
            assert!(!is_relay_type(number));
        }
        // Far from the client protocol's ranges, so it has room to grow.
        assert!(relay.iter().all(|&number| number >= 0xF0));
        // The client protocol's decoders refuse them.
        for number in relay {
            let frame = Frame::new(number, b"{}".to_vec()).unwrap();
            assert!(client::ToCore::decode(&frame).is_err());
            assert!(client::ToClient::decode(&frame).is_err());
        }
        assert!(relay.iter().all(|&number| is_relay_type(number)));
        assert!(!is_relay_type(MSG_RELAY_HOST - 1));
        assert!(!is_relay_type(MSG_RELAY_ERROR + 1));
    }

    #[test]
    fn host_uses_the_documented_field_names() {
        let frame = RelayMessage::Host(RelayHost {
            version: "0.3.0".into(),
            build: "b1d0".into(),
            env: BTreeMap::from([("A".into(), "1".into())]),
            shell: "/bin/sh".into(),
            home: "/root".into(),
            server_dir: "/tmp/b".into(),
            core_pid: 7,
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, 0xF0);
        assert_eq!(
            frame.payload,
            br#"{"version":"0.3.0","build":"b1d0","env":{"A":"1"},"shell":"/bin/sh","home":"/root","server_dir":"/tmp/b","core_pid":7}"#
        );
        let frame = RelayMessage::Error(RelayError { message: "m".into() }).encode().unwrap();
        assert_eq!((frame.msg_type, &frame.payload[..]), (0xF2, &br#"{"message":"m"}"#[..]));
        let frame = RelayMessage::Heartbeat.encode().unwrap();
        assert_eq!((frame.msg_type, frame.payload.len()), (0xF1, 0));
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_ones_default() {
        let frame = Frame::new(MSG_RELAY_HOST, br#"{"version":"9","later":[1]}"#.to_vec()).unwrap();
        assert_eq!(
            RelayMessage::decode(&frame).unwrap(),
            RelayMessage::Host(RelayHost {
                version: "9".into(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn malformed_relay_frames_are_invalid_data() {
        for (msg_type, payload) in [
            (MSG_RELAY_HOST, &b""[..]),
            (MSG_RELAY_HOST, br#"{"core_pid":-1}"#),
            (MSG_RELAY_HOST, br#"{"env":{"a":1}}"#),
            (MSG_RELAY_HEARTBEAT, b"x"),
            (MSG_RELAY_ERROR, b"{}"),
            (MSG_RELAY_ERROR, br#"{"message":"#),
            (client::MSG_OK, br#"{"req":1}"#),
            (0, b""),
        ] {
            let frame = Frame::new(msg_type, payload.to_vec()).unwrap();
            let error = RelayMessage::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        }
    }
}
