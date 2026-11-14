//! A dependency-free decoder fuzz test: a fixed-seed xorshift generator
//! feeds random, truncated and mutated frames to every decoder, which must
//! return `Ok` or `Err` and never panic.

use std::io;

use crate::{Frame, client, holder, relay};

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

#[test]
fn the_assembler_never_panics_on_random_sequences() {
    let mut rng = XorShift::new(0x00a5_5e4b_1e00_0003);
    for _ in 0..2000 {
        let mut assembler = holder::Assembler::new(rng.below(64) as u64);
        for _ in 0..rng.below(24) {
            let session = rng.below(3) as u32;
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
        }
    }
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
