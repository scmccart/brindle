//! A dependency-free decoder fuzz test: a fixed-seed xorshift generator
//! feeds random, truncated and mutated frames to every decoder, which must
//! return `Ok` or `Err` and never panic. The other parsers of outside bytes
//! (the relay's preamble, a held session's meta, versions, holder lock
//! names) get the same, with a property of what they return.

use std::io;
use std::time::{Duration, SystemTime};

use crate::{
    FRAME_STREAM_MARKER, Frame, MAX_PREAMBLE_NOISE, client, holder, relay, server_dir, upgrade,
};

pub(crate) struct XorShift(u64);

impl XorShift {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

type Decoder = (&'static str, fn(&Frame) -> io::Result<()>);

fn decoders() -> Vec<Decoder> {
    vec![
        ("client::ToCore", |f| client::ToCore::decode(f).map(drop)),
        ("client::ToClient", |f| {
            client::ToClient::decode(f).map(drop)
        }),
        ("holder::ToHolder", |f| {
            holder::ToHolder::decode(f).map(drop)
        }),
        ("holder::ToCore", |f| holder::ToCore::decode(f).map(drop)),
        ("relay::RelayMessage", |f| {
            relay::RelayMessage::decode(f).map(drop)
        }),
    ]
}

fn seeds() -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::new();
    frames.extend(
        client::samples::to_core()
            .iter()
            .map(|m| m.encode().unwrap()),
    );
    frames.extend(
        client::samples::to_client()
            .iter()
            .map(|m| m.encode().unwrap()),
    );
    frames.extend(
        relay::samples::messages()
            .iter()
            .map(|m| m.encode().unwrap()),
    );
    frames.extend(
        holder::samples::to_holder()
            .iter()
            .map(|m| m.encode().unwrap()),
    );
    frames.extend(
        holder::samples::to_core()
            .iter()
            .map(|m| m.encode().unwrap()),
    );
    frames
}

const JSONISH: &[u8] = b"{}[]\":,-01239aeflnrstu\\ ";

/// One random frame: noise, a truncated sample, or a mutated sample.
fn random_frame(rng: &mut XorShift, seeds: &[Frame]) -> Frame {
    let seed = &seeds[rng.below(seeds.len())];
    match rng.below(4) {
        0 => {
            let len = rng.below(96);
            Frame {
                msg_type: rng.next() as u8,
                payload: rng.bytes(len),
            }
        }
        1 => {
            let cut = rng.below(seed.payload.len() + 1);
            Frame {
                msg_type: seed.msg_type,
                payload: seed.payload[..cut].to_vec(),
            }
        }
        2 => {
            let mut payload = seed.payload.clone();
            for _ in 0..1 + rng.below(4) {
                if payload.is_empty() {
                    break;
                }
                let at = rng.below(payload.len());
                payload[at] = if rng.below(2) == 0 {
                    rng.next() as u8
                } else {
                    JSONISH[rng.below(JSONISH.len())]
                };
            }
            Frame {
                msg_type: seed.msg_type,
                payload,
            }
        }
        _ => {
            // A sample's payload under another type.
            Frame {
                msg_type: seeds[rng.below(seeds.len())].msg_type,
                payload: seed.payload.clone(),
            }
        }
    }
}

#[test]
fn decoders_never_panic_on_random_frames() {
    let seeds = seeds();
    let decoders = decoders();
    let mut rng = XorShift::new(0x5eed_b41d_1e00_0001);
    let mut errors = 0usize;
    let iterations = 5000;
    for _ in 0..iterations {
        let frame = random_frame(&mut rng, &seeds);
        for (_, decode) in &decoders {
            if let Err(error) = decode(&frame) {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
                errors += 1;
            }
        }
    }
    // The generator is meant to hit the error paths most of the time.
    assert!(errors > iterations, "only {errors} errors");
}

#[test]
fn the_frame_reader_never_panics_on_random_streams() {
    let seeds = seeds();
    let decoders = decoders();
    let mut rng = XorShift::new(0x0bad_cafe_f00d_0002);
    for _ in 0..2000 {
        // A stream of encoded samples, cut short or with noise spliced in.
        let mut stream = Vec::new();
        for _ in 0..1 + rng.below(4) {
            seeds[rng.below(seeds.len())].write_to(&mut stream).unwrap();
        }
        match rng.below(3) {
            0 => stream.truncate(rng.below(stream.len() + 1)),
            1 => {
                let at = rng.below(stream.len());
                stream[at] = rng.next() as u8;
            }
            _ => {
                let len = rng.below(64);
                stream = rng.bytes(len);
            }
        }
        let mut reader = stream.as_slice();
        while let Ok(Some(frame)) = Frame::read_from_or_eof(&mut reader) {
            for (_, decode) in &decoders {
                let _ = decode(&frame);
            }
        }
    }
}

/// Session ids range past [`holder::MAX_OPEN_BLOBS`], so a sequence can try
/// to open more blobs than the bound allows; it never holds more.
#[test]
fn the_assembler_never_panics_on_random_sequences() {
    let mut rng = XorShift::new(0x00a5_5e4b_1e00_0003);
    let mut at_the_bound = 0usize;
    for _ in 0..2000 {
        let mut assembler = holder::Assembler::new(rng.below(64) as u64);
        for _ in 0..rng.below(160) {
            let session = rng.below(64) as u32;
            let _ = match rng.below(4) {
                0 => assembler.begin(holder::BlobBegin {
                    session,
                    len: rng.below(80) as u64,
                }),
                1 | 2 => {
                    let len = rng.below(24);
                    assembler.chunk(&holder::Data {
                        session,
                        bytes: rng.bytes(len),
                    })
                }
                _ => assembler.end(holder::BlobEnd { session }).map(drop),
            };
            let open = assembler.open_blobs();
            assert!(open <= holder::MAX_OPEN_BLOBS, "{open} blobs open");
            at_the_bound += usize::from(open == holder::MAX_OPEN_BLOBS);
        }
    }
    // The generator does reach the bound.
    assert!(at_the_bound > 0, "never {} blobs open", holder::MAX_OPEN_BLOBS);
}

/// The fields added in protocol 2 (`HELLO`'s `linger_secs` and
/// `process_info`, `SessionInfo`'s `owned` and `detached_secs`, `PROCESS`)
/// and the relay's frames, with every value of the wrong type, out of range
/// or hostile: each is `Ok` or an `InvalidData` error.
#[test]
fn the_protocol_2_fields_and_relay_frames_reject_bad_values_without_panicking() {
    let bad = [
        "null",
        "true",
        "-1",
        "4294967296",
        "18446744073709551616",
        "1.5",
        "\"7\"",
        "[]",
        "{}",
        "[1,[2,[3]]]",
        "\"\\ud800\"",
    ];
    let mut cases: Vec<(u8, String)> = Vec::new();
    for value in bad {
        cases.push((
            client::MSG_HELLO,
            format!(r#"{{"min":1,"max":2,"client_token":"t","linger_secs":{value},"process_info":{value}}}"#),
        ));
        cases.push((
            client::MSG_SESSIONS,
            format!(r#"{{"req":1,"sessions":[{{"session":1,"pid":2,"owned":{value},"detached_secs":{value}}}]}}"#),
        ));
        cases.push((
            client::MSG_PROCESS,
            format!(r#"{{"session":{value},"name":{value},"cwd":{value}}}"#),
        ));
        cases.push((
            relay::MSG_RELAY_HOST,
            format!(r#"{{"version":{value},"env":{value},"shell":{value},"core_pid":{value}}}"#),
        ));
        cases.push((
            relay::MSG_RELAY_HOST,
            format!(r#"{{"login":{value}}}"#),
        ));
        cases.push((
            relay::MSG_RELAY_HOST,
            format!(r#"{{"login":{{"captured":{value}}}}}"#),
        ));
        cases.push((
            relay::MSG_RELAY_HOST,
            format!(r#"{{"login":{{"failed":{value}}}}}"#),
        ));
        cases.push((relay::MSG_RELAY_ERROR, format!(r#"{{"message":{value}}}"#)));
    }
    cases.push((relay::MSG_RELAY_HEARTBEAT, "{}".into()));
    let decoders = decoders();
    for (msg_type, payload) in cases {
        let frame = Frame {
            msg_type,
            payload: payload.into_bytes(),
        };
        for (_, decode) in &decoders {
            if let Err(error) = decode(&frame) {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
            }
        }
    }
    // The largest accepted values are accepted.
    let frame = Frame {
        msg_type: client::MSG_HELLO,
        payload: br#"{"min":1,"max":2,"client_token":"t","linger_secs":4294967295,"process_info":true}"#.to_vec(),
    };
    assert!(client::ToCore::decode(&frame).is_ok());
}

/// Client protocol 4's and H1 version 3's messages and fields with every
/// value of the wrong type, out of range or hostile: each is `Ok` or an
/// `InvalidData` error. A `TRANSCRIPT` event that isn't an object and a
/// `STDERR_TAIL` past its limit are errors, whatever else the frame holds.
#[test]
fn the_agent_messages_reject_bad_values_without_panicking() {
    let bad = [
        "null",
        "true",
        "-1",
        "4294967296",
        "18446744073709551616",
        "1.5",
        "\"7\"",
        "[]",
        "{}",
        "[1,[2,[3]]]",
        "\"\\ud800\"",
    ];
    let mut cases: Vec<(u8, String)> = Vec::new();
    for value in bad {
        cases.push((
            client::MSG_SPAWN_AGENT,
            format!(r#"{{"req":{value},"command":{value},"args":{value},"cwd":{value},"env":{value},"policy":{value},"profile":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_PROMPT,
            format!(r#"{{"req":1,"session":{value},"text":{value}}}"#),
        ));
        cases.push((client::MSG_AGENT_CANCEL, format!(r#"{{"session":{value}}}"#)));
        cases.push((
            client::MSG_AGENT_PERMISSION,
            format!(r#"{{"req":1,"session":1,"entry":{value},"option":{value}}}"#),
        ));
        cases.push((client::MSG_AGENT_UNQUEUE, format!(r#"{{"req":{value},"session":1}}"#)));
        cases.push((client::MSG_AGENT_RESEND, format!(r#"{{"req":1,"session":{value}}}"#)));
        cases.push((
            client::MSG_SPAWN_AGENT,
            format!(r#"{{"req":1,"command":"a","conversation":{value}}}"#),
        ));
        cases.push((
            client::MSG_SPAWN_AGENT,
            format!(r#"{{"req":1,"command":"a","conversation":{{"id":{value},"cwd":{value}}}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_RESTART,
            format!(r#"{{"req":{value},"session":{value},"command":{value},"args":{value},"env":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_RESTART,
            format!(r#"{{"req":1,"session":1,"command":"a","args":[{value}],"env":{{"K":{value}}}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_LOGIN,
            format!(r#"{{"req":{value},"session":{value},"method":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_LOGIN,
            format!(r#"{{"req":1,"session":1,"method":{value}}}"#),
        ));
        cases.push((
            client::MSG_TRANSCRIPT,
            format!(r#"{{"session":{value},"events":{value}}}"#),
        ));
        cases.push((
            client::MSG_SESSIONS,
            format!(r#"{{"req":1,"sessions":[{{"session":1,"pid":2,"agent":{value}}}]}}"#),
        ));
        cases.push((
            client::MSG_SESSIONS,
            format!(r#"{{"req":1,"sessions":[{{"session":1,"pid":2,"agent":{{"activity":{value},"name":{value}}}}}]}}"#),
        ));
        cases.push((
            holder::MSG_SPAWN_PIPES,
            format!(r#"{{"session":1,"command":{value},"args":{value},"cwd":{value},"env":{value},"meta":{value}}}"#),
        ));
        cases.push((
            holder::MSG_WELCOME,
            format!(r#"{{"protocol":3,"pid":1,"sessions":[{{"session":1,"pid":2,"cols":0,"rows":0,"mode":{value}}}]}}"#),
        ));
        // An event of every wrong kind among objects.
        cases.push((
            client::MSG_TRANSCRIPT,
            format!(r#"{{"session":1,"events":[{{"event":"turn"}},{value}]}}"#),
        ));
        // Paging (`agents-log-paging`).
        cases.push((
            client::MSG_ATTACH,
            format!(r#"{{"req":1,"session":1,"cols":0,"rows":0,"page":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_FETCH,
            format!(r#"{{"req":{value},"session":{value},"before":{value},"max_bytes":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_FETCH,
            format!(r#"{{"req":1,"session":1,"before":1,"max_bytes":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_PAGE,
            format!(r#"{{"req":{value},"session":{value},"events":{value},"last":{value}}}"#),
        ));
        cases.push((
            client::MSG_AGENT_PAGE,
            format!(r#"{{"req":1,"session":1,"events":[{{"event":"turn"}},{value}],"last":{{"more":{value}}}}}"#),
        ));
    }
    let decoders = decoders();
    for (msg_type, payload) in cases {
        let frame = Frame {
            msg_type,
            payload: payload.into_bytes(),
        };
        for (_, decode) in &decoders {
            if let Err(error) = decode(&frame) {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
            }
        }
    }

    // An event that isn't an object is refused, wherever it is.
    let mut rng = XorShift::new(0xa6e7_0000_0008);
    for _ in 0..500 {
        let mut events: Vec<serde_json::Value> = (0..rng.below(5))
            .map(|_| serde_json::json!({"event": "turn", "n": rng.below(9)}))
            .collect();
        let odd = random_json(&mut rng, 2);
        let at = rng.below(events.len() + 1);
        events.insert(at, odd.clone());
        let payload = serde_json::to_vec(&serde_json::json!({"session": 1, "events": events}))
            .unwrap();
        let frame = Frame {
            msg_type: client::MSG_TRANSCRIPT,
            payload,
        };
        let decoded = client::ToClient::decode(&frame);
        assert_eq!(decoded.is_ok(), odd.is_object(), "{odd} at {at}: {decoded:?}");
    }

    // A STDERR_TAIL is refused past its limit, at any session id.
    for _ in 0..200 {
        let len = holder::MAX_STDERR_TAIL - 2 + rng.below(5);
        let mut payload = (rng.next() as u32).to_be_bytes().to_vec();
        payload.extend(rng.bytes(len));
        let frame = Frame {
            msg_type: holder::MSG_STDERR_TAIL,
            payload,
        };
        match holder::ToCore::decode(&frame) {
            Ok(holder::ToCore::StderrTail(tail)) => {
                assert!(tail.bytes.len() <= holder::MAX_STDERR_TAIL)
            }
            Ok(other) => panic!("decoded {other:?}"),
            Err(error) => {
                assert!(len > holder::MAX_STDERR_TAIL, "{len} bytes refused: {error}");
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            }
        }
    }
}

/// An in-memory stream that hands out short reads and is now and then
/// interrupted, as a pipe from `ssh` can be.
struct Choppy<'a> {
    bytes: &'a [u8],
    at: usize,
    rng: XorShift,
}

impl io::Read for Choppy<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.rng.below(4) == 0 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let len = buffer
            .len()
            .min(1 + self.rng.below(7))
            .min(self.bytes.len() - self.at);
        buffer[..len].copy_from_slice(&self.bytes[self.at..self.at + len]);
        self.at += len;
        Ok(len)
    }
}

/// Shell output before the frame marker: random bytes, line ends, NULs and
/// pieces of the marker that never complete it on their own.
fn preamble_noise(rng: &mut XorShift, len: usize) -> Vec<u8> {
    let mut noise = Vec::with_capacity(len + FRAME_STREAM_MARKER.len());
    while noise.len() < len {
        match rng.below(4) {
            0 => noise
                .extend_from_slice(&FRAME_STREAM_MARKER[..rng.below(FRAME_STREAM_MARKER.len())]),
            1 => noise.push(*b"\n\0\r"[..].get(rng.below(3)).unwrap()),
            _ => {
                let count = 1 + rng.below(16);
                noise.extend(rng.bytes(count));
            }
        }
    }
    noise.truncate(len);
    // A piece of the marker and a line end can make a whole one: break it
    // at its NUL, which can't start another.
    while let Some(at) = noise
        .windows(FRAME_STREAM_MARKER.len())
        .position(|window| window == FRAME_STREAM_MARKER)
    {
        noise[at] = b'x';
    }
    noise
}

/// A relay's stdout: noise, then the marker whole, partial, broken,
/// repeated or missing, then whatever follows.
fn preamble_stream(rng: &mut XorShift) -> Vec<u8> {
    let marker = FRAME_STREAM_MARKER;
    // Near the bound now and then, to check it to the byte.
    let noise_len = match rng.below(8) {
        0 => MAX_PREAMBLE_NOISE - 2 + rng.below(5),
        _ => rng.below(200),
    };
    let mut stream = preamble_noise(rng, noise_len);
    match rng.below(6) {
        0 => stream.extend_from_slice(marker),
        1 => {}
        2 => stream.extend_from_slice(&marker[..rng.below(marker.len())]),
        3 => {
            // A byte of it changed, or noise spliced into it.
            let mut broken = marker.to_vec();
            let at = rng.below(broken.len());
            if rng.below(2) == 0 {
                broken[at] = rng.next() as u8;
            } else {
                let count = 1 + rng.below(4);
                broken.splice(at..at, rng.bytes(count));
            }
            stream.extend(broken);
        }
        _ => {
            stream.extend_from_slice(marker);
            stream.extend_from_slice(marker);
        }
    }
    let after = rng.below(32);
    stream.extend(rng.bytes(after));
    stream
}

#[test]
fn skip_preamble_finds_the_first_marker_or_fails_within_its_bound() {
    let marker = FRAME_STREAM_MARKER;
    let bound = MAX_PREAMBLE_NOISE + marker.len();
    let mut rng = XorShift::new(0x9e3a_1b5c_0d00_0004);
    let mut found = 0usize;
    for _ in 0..600 {
        let stream = preamble_stream(&mut rng);
        let mut reader = Choppy {
            bytes: &stream,
            at: 0,
            rng: XorShift::new(rng.next()),
        };
        let result = crate::skip_preamble(&mut reader);
        assert!(
            reader.at <= bound,
            "read {} bytes; the bound is {bound}",
            reader.at
        );
        let first = stream
            .windows(marker.len())
            .position(|window| window == marker);
        match (result, first) {
            (Ok(noise), Some(at)) if at <= MAX_PREAMBLE_NOISE => {
                assert_eq!(
                    noise,
                    stream[..at],
                    "the noise is what precedes the first marker"
                );
                assert_eq!(reader.at, at + marker.len(), "left just past the marker");
                found += 1;
            }
            (Ok(noise), first) => {
                panic!(
                    "Ok with {} bytes of noise; the first marker is at {first:?}",
                    noise.len()
                )
            }
            (Err(error), first) => {
                let within = first.is_some_and(|at| at <= MAX_PREAMBLE_NOISE);
                assert!(!within, "{error}, though a marker is at {first:?}");
                let expected = if stream.len() < bound {
                    io::ErrorKind::UnexpectedEof
                } else {
                    io::ErrorKind::InvalidData
                };
                assert_eq!(error.kind(), expected, "{error}");
                assert_eq!(reader.at, stream.len().min(bound), "read up to the bound");
            }
        }
    }
    assert!(found > 0, "the generator never placed a marker");
}

/// `SessionMeta`'s field names, so random objects hit its parsers.
const META_FIELDS: [&str; 9] = [
    "command",
    "cwd",
    "key",
    "layout",
    "owner",
    "policy",
    "profile",
    "scrollback",
    "started",
];

fn random_string(rng: &mut XorShift) -> String {
    match rng.below(4) {
        0 => ["kill", "detach", "Kill", "", "/home/u", "bash -l"][rng.below(6)].to_string(),
        1 => META_FIELDS[rng.below(META_FIELDS.len())].to_string(),
        _ => {
            let len = rng.below(12);
            String::from_utf8_lossy(&rng.bytes(len)).into_owned()
        }
    }
}

fn random_number(rng: &mut XorShift) -> serde_json::Value {
    match rng.below(8) {
        0 => serde_json::json!(-1 - rng.below(1 << 20) as i64),
        1 => serde_json::json!(u64::MAX - rng.below(2) as u64),
        2 => serde_json::json!(u64::from(u32::MAX) + rng.below(2) as u64),
        3 => serde_json::json!(i64::MIN),
        4 => {
            let float = [1.5, -0.0, 1e300, -2.5e-7][rng.below(4)];
            serde_json::json!(float)
        }
        _ => serde_json::json!(rng.below(100_000)),
    }
}

/// A random JSON value at most `depth` levels deep, whose object keys are
/// `SessionMeta`'s field names more often than not.
fn random_json(rng: &mut XorShift, depth: usize) -> serde_json::Value {
    use serde_json::Value;
    match rng.below(if depth == 0 { 4 } else { 6 }) {
        0 => Value::Null,
        1 => Value::Bool(rng.below(2) == 0),
        2 => random_number(rng),
        3 => Value::String(random_string(rng)),
        4 => Value::Array(
            (0..rng.below(4))
                .map(|_| random_json(rng, depth - 1))
                .collect(),
        ),
        _ => Value::Object(
            (0..rng.below(6))
                .map(|_| (random_string(rng), random_json(rng, depth - 1)))
                .collect(),
        ),
    }
}

/// `holder::samples::meta()` with fields replaced, dropped or added.
fn mutated_meta(rng: &mut XorShift) -> serde_json::Value {
    let mut meta = holder::samples::meta();
    let object = meta.as_object_mut().expect("the sample is an object");
    for _ in 0..1 + rng.below(4) {
        let field = META_FIELDS[rng.below(META_FIELDS.len())].to_string();
        match rng.below(3) {
            0 => {
                object.remove(&field);
            }
            1 => {
                let key = random_string(rng);
                object.insert(key, random_json(rng, 2));
            }
            _ => {
                object.insert(field, random_json(rng, 2));
            }
        }
    }
    meta
}

#[test]
fn session_meta_reads_any_value_and_round_trips_what_it_returns() {
    let mut rng = XorShift::new(0x3e7a_0b1d_c0de_0005);
    let mut kept = 0usize;
    let mut refused = 0usize;
    for _ in 0..3000 {
        let value = match rng.below(3) {
            0 => random_json(&mut rng, 3),
            1 => mutated_meta(&mut rng),
            _ => serde_json::Value::Object(
                (0..rng.below(8))
                    .map(|_| {
                        let field = META_FIELDS[rng.below(META_FIELDS.len())].to_string();
                        (field, random_json(&mut rng, 2))
                    })
                    .collect(),
            ),
        };
        // Read, or refused with what was well formed: never a panic.
        let meta = match holder::SessionMeta::from_value(&value) {
            Ok(meta) => meta,
            Err(fault) => {
                assert!(!fault.malformed.is_empty(), "a fault names nothing: {value}");
                refused += 1;
                fault.read
            }
        };
        assert_eq!(
            holder::SessionMeta::from_value(&meta.to_value()),
            Ok(meta.clone()),
            "read from {value}"
        );
        kept += usize::from(meta != holder::SessionMeta::default());
    }
    // It reaches both answers.
    assert!(refused > 0 && refused < 3000, "{refused} of 3000 refused");
    // The generator reaches the fields' parsers, not only the defaults.
    assert!(kept > 0, "every meta read was the default");
}

/// Random version text: digits, dots, `-`, `+`, whitespace, a pre-release
/// word and random bytes.
fn random_version(rng: &mut XorShift) -> String {
    const VERSIONISH: &[u8] = b"0123456789...-+ \t\nrc";
    match rng.below(4) {
        0 => {
            let len = rng.below(14);
            (0..len)
                .map(|_| char::from(VERSIONISH[rng.below(VERSIONISH.len())]))
                .collect()
        }
        1 => {
            let suffix = ["", "-rc.1", "+b1", "-", " ", "\n", ".4"][rng.below(7)];
            format!("{}.{}.{}{suffix}", rng.below(3), rng.below(3), rng.below(3))
        }
        2 => {
            // Past u64.
            let len = 18 + rng.below(4);
            let digits: String = (0..len)
                .map(|_| char::from(b'0' + rng.below(10) as u8))
                .collect();
            format!("{digits}.0.1")
        }
        _ => {
            let len = rng.below(12);
            String::from_utf8_lossy(&rng.bytes(len)).into_owned()
        }
    }
}

#[test]
fn versions_parse_without_panicking_and_newer_is_never_mutual() {
    let mut rng = XorShift::new(0x0e75_10a5_0000_0006);
    let mut parsed = 0usize;
    for _ in 0..5000 {
        let version = random_version(&mut rng);
        if let Some((a, b, c)) = upgrade::semver(&version) {
            assert_eq!(
                upgrade::semver(&format!("{a}.{b}.{c}")),
                Some((a, b, c)),
                "{version:?}"
            );
            parsed += 1;
        }
    }
    assert!(parsed > 0, "no version parsed");

    let build = |rng: &mut XorShift| upgrade::Build {
        version: match rng.below(3) {
            0 => random_version(rng),
            _ => ["0.3.0", "0.3.0-rc.1", "0.3.1", "1.0.0", "dev"][rng.below(5)].to_string(),
        },
        build: ["a", "b", ""][rng.below(3)].to_string(),
        mtime: match rng.below(4) {
            0 => None,
            _ => Some(SystemTime::UNIX_EPOCH + Duration::from_secs(rng.below(3) as u64)),
        },
    };
    let mut newer = 0usize;
    for _ in 0..5000 {
        let (x, y) = (build(&mut rng), build(&mut rng));
        let (forward, back) = (upgrade::is_newer(&x, &y), upgrade::is_newer(&y, &x));
        assert!(
            !(forward && back),
            "each newer than the other: {x:?}, {y:?}"
        );
        assert!(!upgrade::is_newer(&x, &x), "newer than itself: {x:?}");
        newer += usize::from(forward || back);
    }
    assert!(newer > 0, "no pair had a newer build");
}

/// Random lock file names: pieces of `holder-<build>-<n>.lock` and random
/// bytes.
fn random_lock_name(rng: &mut XorShift) -> String {
    const PIECES: [&str; 8] = ["holder-", ".lock", "-", "0", "42", "b1", ".", "4294967296"];
    let mut name = String::new();
    for _ in 0..rng.below(7) {
        if rng.below(4) == 0 {
            let len = 1 + rng.below(4);
            name.push_str(&String::from_utf8_lossy(&rng.bytes(len)));
        } else {
            name.push_str(PIECES[rng.below(PIECES.len())]);
        }
    }
    name
}

#[test]
fn holder_lock_names_parse_without_panicking_and_round_trip() {
    let mut rng = XorShift::new(0x10c4_f11e_0000_0007);
    let mut parsed = 0usize;
    for _ in 0..5000 {
        let name = random_lock_name(&mut rng);
        if let Some((build, n)) = server_dir::parse_holder_lock(&name) {
            let rendered = format!("holder-{build}-{n}.lock");
            assert_eq!(
                server_dir::parse_holder_lock(&rendered),
                Some((build, n)),
                "{name:?}"
            );
            parsed += 1;
        }
    }
    assert!(parsed > 0, "no random name parsed");

    for _ in 0..5000 {
        // Any non-empty build: dashes, dots, digits and random text included.
        let mut build = random_lock_name(&mut rng);
        if build.is_empty() {
            build.push('b');
        }
        let n = rng.next() as u32;
        let name = format!("holder-{build}-{n}.lock");
        assert_eq!(
            server_dir::parse_holder_lock(&name),
            Some((build.clone(), n)),
            "{name:?}"
        );
    }
}
