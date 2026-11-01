//! Framing and control messages shared by Brindle and its remote agent.
//!
//! Each frame has a nine-byte header followed by an opaque payload:
//! `channel_id: u32` (big endian), `message_type: u8`, and
//! `payload_length: u32` (big endian). Channel zero is reserved for agent
//! control messages; non-zero channels carry session traffic.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const CHANNEL_CONTROL: u32 = 0;
pub const FRAME_HEADER_LEN: usize = 9;
/// The largest payload accepted from the wire or written to it.
///
/// Session data is emitted in much smaller chunks. This upper bound mainly
/// leaves room for spawn requests with sizeable environments while preventing
/// an untrusted peer from causing an unbounded allocation.
pub const MAX_FRAME_PAYLOAD_SIZE: usize = 1024 * 1024;

pub const MSG_DATA: u8 = 1;
pub const MSG_RESIZE: u8 = 2;
pub const MSG_EXIT: u8 = 3;
pub const MSG_QUERY_PROCESS: u8 = 4;
pub const MSG_PROCESS_INFO: u8 = 5;
#[allow(dead_code)] // Used by the standalone agent's --server mode.
pub const MSG_SPAWN: u8 = 6;
pub const MSG_SPAWN_RESPONSE: u8 = 7;
pub const MSG_ERROR: u8 = 8;
#[allow(dead_code)] // Reserved for agent keepalive.
pub const MSG_PING: u8 = 9;
#[allow(dead_code)]
pub const MSG_PONG: u8 = 10;

/// Written by the agent in `--server` mode before its first frame. Over SSH
/// the remote command runs through the user's shell, whose startup files may
/// print to stdout first; the client discards everything up to this marker.
#[allow(dead_code)] // Used by the standalone agent's --server mode.
pub const FRAME_STREAM_MARKER: &[u8] = b"\0BRINDLE-FRAMES1\n";
/// The most shell startup output the client discards before giving up.
#[allow(dead_code)] // Used by the client only.
pub const MAX_PREAMBLE_NOISE: usize = 64 * 1024;

/// Reads `reader` up to and including [`FRAME_STREAM_MARKER`], returning the
/// bytes that preceded it. Reads one byte at a time so nothing past the
/// marker is consumed and the frame stream starts exactly where `reader` is
/// left; call it on an unbuffered reader or on the buffered reader used for
/// all later reads. Fails when the stream ends or more than
/// [`MAX_PREAMBLE_NOISE`] bytes pass before the marker appears.
#[allow(dead_code)] // Used by the client only.
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
                if seen.len() > MAX_PREAMBLE_NOISE + FRAME_STREAM_MARKER.len() {
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
    let more = if seen.len() > shown.len() { " (truncated)" } else { "" };
    format!(
        "{problem}; shell startup output or a failed remote helper is the likely cause. \
         Output received before it: {:?}{more}",
        text.trim_end()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel_id: u32,
    pub msg_type: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(channel_id: u32, msg_type: u8, payload: Vec<u8>) -> io::Result<Self> {
        validate_outgoing_length(payload.len())?;
        Ok(Self {
            channel_id,
            msg_type,
            payload,
        })
    }

    pub fn data(channel_id: u32, bytes: &[u8]) -> io::Result<Self> {
        Self::new(channel_id, MSG_DATA, bytes.to_vec())
    }

    pub fn json<T: Serialize>(channel_id: u32, msg_type: u8, value: &T) -> io::Result<Self> {
        let payload = serde_json::to_vec(value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Self::new(channel_id, msg_type, payload)
    }

    pub fn decode_json<T: DeserializeOwned>(&self) -> io::Result<T> {
        serde_json::from_slice(&self.payload)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn resize(
        channel_id: u32,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> io::Result<Self> {
        Self::json(
            channel_id,
            MSG_RESIZE,
            &ResizeRequest {
                cols,
                rows,
                cell_width,
                cell_height,
            },
        )
    }

    pub fn exit(channel_id: u32, code: Option<i32>) -> io::Result<Self> {
        Self::json(channel_id, MSG_EXIT, &ExitMessage { code })
    }

    pub fn query_process(channel_id: u32) -> io::Result<Self> {
        Self::new(channel_id, MSG_QUERY_PROCESS, Vec::new())
    }

    #[allow(dead_code)] // Used by the standalone agent's --server mode.
    pub fn spawn(request: &SpawnRequest) -> io::Result<Self> {
        Self::json(CHANNEL_CONTROL, MSG_SPAWN, request)
    }

    #[allow(dead_code)] // Used by the standalone agent binary.
    pub fn spawn_response(response: &SpawnResponse) -> io::Result<Self> {
        Self::json(CHANNEL_CONTROL, MSG_SPAWN_RESPONSE, response)
    }

    #[allow(dead_code)] // Used by the standalone agent binary.
    pub fn process_info(info: &ProcessInfo) -> io::Result<Self> {
        Self::json(info.channel_id, MSG_PROCESS_INFO, info)
    }

    #[allow(dead_code)] // Used by the standalone agent binary.
    pub fn error(channel_id: u32, message: impl Into<String>) -> io::Result<Self> {
        Self::json(
            channel_id,
            MSG_ERROR,
            &ErrorMessage {
                message: message.into(),
            },
        )
    }

    /// Writes exactly one frame and flushes it so stdio transports deliver it
    /// without waiting for another packet.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        validate_outgoing_length(self.payload.len())?;

        let mut header = [0; FRAME_HEADER_LEN];
        header[..4].copy_from_slice(&self.channel_id.to_be_bytes());
        header[4] = self.msg_type;
        header[5..].copy_from_slice(&(self.payload.len() as u32).to_be_bytes());
        writer.write_all(&header)?;
        writer.write_all(&self.payload)?;
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
                Ok(1) => break,
                Ok(_) => {
                    return Err(io::Error::other(
                        "reader returned more bytes than its buffer can hold",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        reader.read_exact(&mut header[1..])?;

        let channel_id = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let msg_type = header[4];
        let payload_len = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;
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
        Ok(Some(Self {
            channel_id,
            msg_type,
            payload,
        }))
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResizeRequest {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExitMessage {
    pub code: Option<i32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionKind {
    Pty,
    Tmux,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpawnRequest {
    pub channel_id: u32,
    pub kind: SessionKind,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpawnResponse {
    pub channel_id: u32,
    pub success: bool,
    pub error: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessInfo {
    pub channel_id: u32,
    pub pid: u32,
    pub foreground_name: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorMessage {
    pub message: String,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

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
        let frame = Frame::data(1, b"payload").unwrap();
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
        assert!(error.to_string().contains("shell startup output"), "{error}");

        let flood = vec![b'x'; MAX_PREAMBLE_NOISE + 4096];
        let error = skip_preamble(&mut Cursor::new(flood)).unwrap_err();
        assert!(error.to_string().contains("64 KiB"), "{error}");
    }

    #[test]
    fn frame_roundtrip_survives_short_reads_and_writes() {
        let original = Frame::data(42, b"hello\0remote\xffterminal").unwrap();
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

        let truncated_payload = vec![0, 0, 0, 1, MSG_DATA, 0, 0, 0, 2, b'x'];
        let error = Frame::read_from(&mut truncated_payload.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn oversized_payloads_are_rejected_before_reading_or_writing_them() {
        let oversized = (MAX_FRAME_PAYLOAD_SIZE as u32 + 1).to_be_bytes();
        let mut encoded = vec![0, 0, 0, 1, MSG_DATA];
        encoded.extend_from_slice(&oversized);
        let error = Frame::read_from(&mut encoded.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let error = Frame::new(
            1,
            MSG_DATA,
            vec![0; MAX_FRAME_PAYLOAD_SIZE.saturating_add(1)],
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        let oversized_frame = Frame {
            channel_id: 1,
            msg_type: MSG_DATA,
            payload: vec![0; MAX_FRAME_PAYLOAD_SIZE.saturating_add(1)],
        };
        let mut output = Vec::new();
        let error = oversized_frame.write_to(&mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(output.is_empty());
    }

    #[test]
    fn control_payloads_roundtrip() {
        let spawn = SpawnRequest {
            channel_id: 7,
            kind: SessionKind::Pty,
            command: Some("/bin/bash".into()),
            args: vec!["-l".into()],
            cwd: Some("/home/user".into()),
            env: BTreeMap::from([("TERM".into(), "xterm-256color".into())]),
            cols: 80,
            rows: 24,
            cell_width: 9,
            cell_height: 18,
        };
        let spawn_frame = Frame::spawn(&spawn).unwrap();
        assert_eq!(spawn_frame.channel_id, CHANNEL_CONTROL);
        assert_eq!(spawn_frame.msg_type, MSG_SPAWN);
        assert_eq!(spawn_frame.decode_json::<SpawnRequest>().unwrap(), spawn);

        let response = SpawnResponse {
            channel_id: 7,
            success: true,
            error: None,
            pid: Some(1234),
        };
        let response_frame = Frame::spawn_response(&response).unwrap();
        assert_eq!(response_frame.channel_id, CHANNEL_CONTROL);
        assert_eq!(response_frame.msg_type, MSG_SPAWN_RESPONSE);
        assert_eq!(
            response_frame.decode_json::<SpawnResponse>().unwrap(),
            response
        );

        let resize = ResizeRequest {
            cols: 120,
            rows: 40,
            cell_width: 10,
            cell_height: 20,
        };
        let resize_frame = Frame::json(7, MSG_RESIZE, &resize).unwrap();
        assert_eq!(resize_frame.decode_json::<ResizeRequest>().unwrap(), resize);

        let info = ProcessInfo {
            channel_id: 7,
            pid: 1234,
            foreground_name: Some("vim".into()),
            cwd: Some("/work".into()),
        };
        let info_frame = Frame::process_info(&info).unwrap();
        assert_eq!(info_frame.decode_json::<ProcessInfo>().unwrap(), info);

        let exit_frame = Frame::exit(7, Some(17)).unwrap();
        assert_eq!(
            exit_frame.decode_json::<ExitMessage>().unwrap(),
            ExitMessage { code: Some(17) }
        );

        let error_frame = Frame::error(7, "bad request").unwrap();
        assert_eq!(
            error_frame.decode_json::<ErrorMessage>().unwrap(),
            ErrorMessage {
                message: "bad request".into()
            }
        );
    }
}
