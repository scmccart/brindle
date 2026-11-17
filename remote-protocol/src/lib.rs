//! Framing shared by Brindle and its `brindle-remote` helper.
//!
//! Each frame has a five-byte header followed by an opaque payload:
//! `message_type: u8` and `payload_length: u32` (big endian). The terminal
//! server's protocols carry their own message numbers in it: [`client`]
//! (client ↔ core, protocols 1–3) and [`holder`] (core ↔ holder, H1).
//! [`relay`] adds the frames of the link between Brindle and a
//! `brindle-remote relay`, which pipes the client protocol, after the
//! [`FRAME_STREAM_MARKER`].

pub mod access;
pub mod build_id;
pub mod client;
pub mod endpoint;
pub mod holder;
pub mod ready;
pub mod relay;
#[cfg(unix)]
pub mod scope;
pub mod server_dir;
pub mod upgrade;
pub mod version;
#[cfg(windows)]
pub mod win;
mod wire;

#[cfg(test)]
mod fuzz;

/// Re-exported for the helper's `--json` output, so it needs no dependency
/// of its own.
pub use serde_json;
pub use wire::{RequestId, SessionId};

use std::io::{self, Read, Write};
use std::time::Duration;

use serde::Serialize;

pub const FRAME_HEADER_LEN: usize = 5;
/// The largest payload accepted from the wire or written to it.
///
/// Session data is emitted in much smaller chunks. This upper bound mainly
/// leaves room for spawn requests with sizeable environments while preventing
/// an untrusted peer from causing an unbounded allocation.
pub const MAX_FRAME_PAYLOAD_SIZE: usize = 1024 * 1024;

/// How long either side of a relay link goes without writing before it
/// sends a `RELAY_HEARTBEAT` ([`relay`]), so the other can tell a quiet
/// link from a dead one.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// Written by `brindle-remote relay` before its first frame. Over SSH
/// the remote command runs through the user's shell, whose startup files may
/// print to stdout first; the client discards everything up to this marker.
pub const FRAME_STREAM_MARKER: &[u8] = b"\0BRINDLE-FRAMES1\n";
/// The most shell startup output the client discards before giving up.
pub const MAX_PREAMBLE_NOISE: usize = 64 * 1024;

/// Reads `reader` up to and including [`FRAME_STREAM_MARKER`], returning the
/// bytes that preceded it. Reads one byte at a time so nothing past the
/// marker is consumed and the frame stream starts exactly where `reader` is
/// left; call it on an unbuffered reader or on the buffered reader used for
/// all later reads. Fails when the stream ends or more than
/// [`MAX_PREAMBLE_NOISE`] bytes pass before the marker appears, so it never
/// reads more than that bound plus the marker.
pub fn skip_preamble(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut seen: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    preamble_message("the stream ended before the frame marker", &seen),
                ));
            }
            Ok(_) => {
                seen.push(byte[0]);
                if seen.ends_with(FRAME_STREAM_MARKER) {
                    seen.truncate(seen.len() - FRAME_STREAM_MARKER.len());
                    return Ok(seen);
                }
                // A marker ending later would follow more noise than allowed.
                if seen.len() >= MAX_PREAMBLE_NOISE + FRAME_STREAM_MARKER.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        preamble_message(
                            "no frame marker within the first 64 KiB of output",
                            &seen,
                        ),
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn preamble_message(problem: &str, seen: &[u8]) -> String {
    let shown = &seen[..seen.len().min(512)];
    let text = String::from_utf8_lossy(shown);
    let more = if seen.len() > shown.len() {
        " (truncated)"
    } else {
        ""
    };
    format!(
        "{problem}; shell startup output or a failed remote helper is the likely cause. \
         Output received before it: {:?}{more}",
        text.trim_end()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub msg_type: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(msg_type: u8, payload: Vec<u8>) -> io::Result<Self> {
        validate_outgoing_length(payload.len())?;
        Ok(Self { msg_type, payload })
    }

    pub fn json<T: Serialize>(msg_type: u8, value: &T) -> io::Result<Self> {
        let payload = serde_json::to_vec(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Self::new(msg_type, payload)
    }

    /// Writes exactly one frame, header and payload in a single write, and
    /// flushes it so stdio transports deliver it without waiting for another
    /// packet.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        validate_outgoing_length(self.payload.len())?;

        let mut bytes = Vec::with_capacity(FRAME_HEADER_LEN + self.payload.len());
        bytes.push(self.msg_type);
        bytes.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        writer.write_all(&bytes)?;
        writer.flush()
    }

    /// Reads exactly one frame. A clean EOF before the next header is reported
    /// as `UnexpectedEof`; callers that treat it as normal should use
    /// [`Frame::read_from_or_eof`].
    pub fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        Self::read_from_or_eof(reader)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "EOF before frame header"))
    }

    /// Reads exactly one frame, returning `None` only when EOF occurs before
    /// any byte of a new header. A partial header or payload is always an
    /// `UnexpectedEof` error.
    pub fn read_from_or_eof<R: Read>(reader: &mut R) -> io::Result<Option<Self>> {
        let mut header = [0; FRAME_HEADER_LEN];
        loop {
            match reader.read(&mut header[..1]) {
                Ok(0) => return Ok(None),
                // The buffer holds one byte, so any other count is one.
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        reader.read_exact(&mut header[1..])?;

        let msg_type = header[0];
        let payload_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if payload_len > MAX_FRAME_PAYLOAD_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "remote frame payload is {payload_len} bytes; maximum is {MAX_FRAME_PAYLOAD_SIZE}"
                ),
            ));
        }

        let mut payload = vec![0; payload_len];
        reader.read_exact(&mut payload)?;
        Ok(Some(Self { msg_type, payload }))
    }
}

fn validate_outgoing_length(length: usize) -> io::Result<()> {
    if length > MAX_FRAME_PAYLOAD_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("remote frame payload is {length} bytes; maximum is {MAX_FRAME_PAYLOAD_SIZE}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// A message type: framing is the same whatever the protocol.
    const ANY: u8 = 1;

    struct ShortIo {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
        chunk_size: usize,
    }

    impl Read for ShortIo {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let length = buffer.len().min(self.chunk_size);
            self.input.read(&mut buffer[..length])
        }
    }

    impl Write for ShortIo {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let length = buffer.len().min(self.chunk_size);
            self.output.extend_from_slice(&buffer[..length]);
            Ok(length)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn preamble_skips_noise_and_leaves_the_frame_stream_untouched() {
        let frame = Frame::new(ANY, b"payload".to_vec()).unwrap();
        let mut bytes = b"motd\n\0BRINDLE-almost\n".to_vec();
        bytes.extend_from_slice(FRAME_STREAM_MARKER);
        frame.write_to(&mut bytes).unwrap();
        let mut reader = ShortIo {
            input: Cursor::new(bytes),
            output: Vec::new(),
            chunk_size: 3,
        };
        let noise = skip_preamble(&mut reader).unwrap();
        assert_eq!(noise, b"motd\n\0BRINDLE-almost\n");
        assert_eq!(Frame::read_from(&mut reader).unwrap(), frame);
    }

    #[test]
    fn preamble_without_noise_is_empty() {
        let mut bytes = FRAME_STREAM_MARKER.to_vec();
        bytes.push(7);
        let mut reader = Cursor::new(bytes);
        assert!(skip_preamble(&mut reader).unwrap().is_empty());
        assert_eq!(reader.position() as usize, FRAME_STREAM_MARKER.len());
    }

    #[test]
    fn preamble_eof_and_bound_report_the_discarded_text() {
        let error = skip_preamble(&mut Cursor::new(b"Welcome to host\n".to_vec())).unwrap_err();
        assert!(error.to_string().contains("Welcome to host"), "{error}");
        assert!(
            error.to_string().contains("shell startup output"),
            "{error}"
        );

        let flood = vec![b'x'; MAX_PREAMBLE_NOISE + 4096];
        let error = skip_preamble(&mut Cursor::new(flood)).unwrap_err();
        assert!(error.to_string().contains("64 KiB"), "{error}");
    }

    #[test]
    fn a_marker_after_one_byte_more_than_the_noise_limit_is_refused_within_the_bound() {
        let bound = MAX_PREAMBLE_NOISE + FRAME_STREAM_MARKER.len();
        let after = |noise: usize| {
            let mut bytes = vec![b'x'; noise];
            bytes.extend_from_slice(FRAME_STREAM_MARKER);
            bytes.push(7);
            Cursor::new(bytes)
        };
        let mut reader = after(MAX_PREAMBLE_NOISE);
        assert_eq!(
            skip_preamble(&mut reader).unwrap().len(),
            MAX_PREAMBLE_NOISE
        );
        assert_eq!(reader.position() as usize, bound);

        let mut reader = after(MAX_PREAMBLE_NOISE + 1);
        let error = skip_preamble(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        assert_eq!(reader.position() as usize, bound, "read past the bound");

        // Exactly the bound's worth with no marker is too much noise, not
        // an early end.
        let mut reader = Cursor::new(vec![b'x'; bound]);
        let error = skip_preamble(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    }

    #[test]
    fn frame_roundtrip_survives_short_reads_and_writes() {
        let original = Frame::new(ANY, b"hello\0remote\xffterminal".to_vec()).unwrap();
        let mut writer = ShortIo {
            input: Cursor::new(Vec::new()),
            output: Vec::new(),
            chunk_size: 2,
        };
        original.write_to(&mut writer).unwrap();

        let mut reader = ShortIo {
            input: Cursor::new(writer.output),
            output: Vec::new(),
            chunk_size: 3,
        };
        assert_eq!(Frame::read_from(&mut reader).unwrap(), original);
    }

    #[test]
    fn clean_eof_is_distinct_from_a_truncated_frame() {
        assert_eq!(
            Frame::read_from_or_eof(&mut Cursor::new(Vec::new())).unwrap(),
            None
        );

        let error = Frame::read_from_or_eof(&mut Cursor::new(vec![0; 4])).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        let truncated_payload = vec![ANY, 0, 0, 0, 2, b'x'];
        let error = Frame::read_from(&mut truncated_payload.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn oversized_payloads_are_rejected_before_reading_or_writing_them() {
        let oversized = (MAX_FRAME_PAYLOAD_SIZE as u32 + 1).to_be_bytes();
        let mut encoded = vec![ANY];
        encoded.extend_from_slice(&oversized);
        let error = Frame::read_from(&mut encoded.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let error =
            Frame::new(ANY, vec![0; MAX_FRAME_PAYLOAD_SIZE.saturating_add(1)]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        let oversized_frame = Frame {
            msg_type: ANY,
            payload: vec![0; MAX_FRAME_PAYLOAD_SIZE.saturating_add(1)],
        };
        let mut output = Vec::new();
        let error = oversized_frame.write_to(&mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(output.is_empty());
    }
}
