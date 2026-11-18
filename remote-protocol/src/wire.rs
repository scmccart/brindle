//! Encoding helpers shared by the client protocol and H1.
//!
//! Both put two kinds of payload in the common frame header:
//! - **binary, session-scoped:** a `u32` big-endian session id, then raw bytes;
//! - **JSON control messages:** one object, decoded with unknown fields
//!   ignored and missing optional fields defaulted.

use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Frame;

/// A session's id, allocated by the core.
pub type SessionId = u32;
/// A request id chosen by the sender, echoed in the reply.
pub type RequestId = u64;

pub(crate) const SESSION_PREFIX_LEN: usize = 4;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub(crate) fn unknown_type(protocol: &str, msg_type: u8) -> io::Error {
    invalid(format!("unknown {protocol} message type {msg_type}"))
}

pub(crate) fn json<T: Serialize>(msg_type: u8, value: &T) -> io::Result<Frame> {
    Frame::json(msg_type, value)
}

pub(crate) fn from_json<T: DeserializeOwned>(name: &str, frame: &Frame) -> io::Result<T> {
    serde_json::from_slice(&frame.payload).map_err(|error| invalid(format!("bad {name}: {error}")))
}

/// A frame whose payload is `session` (u32 BE) followed by `bytes`.
pub(crate) fn session_bytes(msg_type: u8, session: SessionId, bytes: &[u8]) -> io::Result<Frame> {
    let mut payload = Vec::with_capacity(SESSION_PREFIX_LEN + bytes.len());
    payload.extend_from_slice(&session.to_be_bytes());
    payload.extend_from_slice(bytes);
    Frame::new(msg_type, payload)
}

/// Splits a session-prefixed payload into its session id and the rest.
pub(crate) fn split_session<'a>(name: &str, frame: &'a Frame) -> io::Result<(SessionId, &'a [u8])> {
    let Some((prefix, rest)) = frame.payload.split_first_chunk::<SESSION_PREFIX_LEN>() else {
        return Err(invalid(format!(
            "{name} is {} bytes; it needs a {SESSION_PREFIX_LEN}-byte session id",
            frame.payload.len()
        )));
    };
    Ok((u32::from_be_bytes(*prefix), rest))
}

/// Decodes a session-prefixed payload that must hold exactly `N` more bytes.
pub(crate) fn session_fixed<const N: usize>(
    name: &str,
    frame: &Frame,
) -> io::Result<(SessionId, [u8; N])> {
    let (session, rest) = split_session(name, frame)?;
    let fixed: [u8; N] = rest.try_into().map_err(|_| {
        invalid(format!(
            "{name} carries {} bytes after its session id, not {N}",
            rest.len()
        ))
    })?;
    Ok((session, fixed))
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixed_payloads_reject_other_lengths() {
        let frame = Frame::new(1, vec![0, 0, 0, 5, 9, 9]).unwrap();
        assert_eq!(session_fixed::<2>("x", &frame).unwrap(), (5, [9, 9]));
        assert!(session_fixed::<1>("x", &frame).is_err());
        assert!(session_fixed::<3>("x", &frame).is_err());
        let frame = Frame::new(1, vec![0, 0, 0, 5]).unwrap();
        assert_eq!(session_fixed::<0>("x", &frame).unwrap(), (5, []));
    }

    use super::*;

    #[test]
    fn the_session_prefix_is_four_big_endian_bytes() {
        let frame = session_bytes(9, 0x0102_0304, b"hi").unwrap();
        assert_eq!(frame.payload, [1, 2, 3, 4, b'h', b'i']);
        assert_eq!(
            split_session("x", &frame).unwrap(),
            (0x0102_0304, &b"hi"[..])
        );

        let empty = session_bytes(9, 7, b"").unwrap();
        assert_eq!(split_session("x", &empty).unwrap(), (7, &b""[..]));

        for short in 0..SESSION_PREFIX_LEN {
            let frame = Frame::new(9, vec![0; short]).unwrap();
            let error = split_session("DATA", &frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("DATA"), "{error}");
        }
    }

    #[test]
    fn the_largest_session_payload_still_fits_a_frame() {
        let bytes = vec![0; crate::MAX_FRAME_PAYLOAD_SIZE - SESSION_PREFIX_LEN];
        assert!(session_bytes(1, 1, &bytes).is_ok());
        let bytes = vec![0; crate::MAX_FRAME_PAYLOAD_SIZE - SESSION_PREFIX_LEN + 1];
        assert_eq!(
            session_bytes(1, 1, &bytes).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    /// A control message as the protocols' messages are: a required field
    /// and a defaulted one.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        name: String,
        #[serde(default)]
        count: u32,
    }

    fn frame(payload: &[u8]) -> Frame {
        Frame::new(1, payload.to_vec()).unwrap()
    }

    #[test]
    fn a_control_message_is_one_json_object_in_its_type() {
        let sample = Sample {
            name: "a".into(),
            count: 2,
        };
        let frame = json(12, &sample).unwrap();
        assert_eq!(frame.msg_type, 12);
        assert_eq!(frame.payload, br#"{"name":"a","count":2}"#);
        assert_eq!(from_json::<Sample>("SAMPLE", &frame).unwrap(), sample);
    }

    /// Fields a later version adds are ignored, and optional fields an
    /// earlier version leaves out take their defaults (`process R5`).
    #[test]
    fn decoding_ignores_unknown_fields_and_defaults_missing_ones() {
        let later = frame(br#"{"name":"a","count":2,"added":{"x":[1]}}"#);
        let sample = Sample {
            name: "a".into(),
            count: 2,
        };
        assert_eq!(from_json::<Sample>("SAMPLE", &later).unwrap(), sample);
        let earlier = frame(br#"{"name":"b"}"#);
        let sample = Sample {
            name: "b".into(),
            count: 0,
        };
        assert_eq!(from_json::<Sample>("SAMPLE", &earlier).unwrap(), sample);
    }

    /// Anything that isn't the message, malformed or missing a required
    /// field, is `InvalidData` naming the message.
    #[test]
    fn a_bad_message_is_invalid_data_naming_it() {
        for payload in [
            &b""[..],
            b"{",
            b"[]",
            b"null",
            b"\xff\xfe",
            br#"{"count":1}"#,
            br#"{"name":5}"#,
            br#"{"name":"a"} trailing"#,
        ] {
            let error = from_json::<Sample>("SAMPLE", &frame(payload)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{payload:?}");
            assert!(
                error.to_string().starts_with("bad SAMPLE: "),
                "{payload:?}: {error}"
            );
        }
    }

    #[test]
    fn an_unknown_type_is_invalid_data_naming_the_protocol_and_number() {
        let error = unknown_type("core-to-client", 200);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "unknown core-to-client message type 200");
    }
}
