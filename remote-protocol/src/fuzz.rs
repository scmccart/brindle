//! A dependency-free decoder fuzz test: a fixed-seed xorshift generator
//! feeds random, truncated and mutated frames to every decoder, which must
//! return `Ok` or `Err` and never panic.

use std::io;

use crate::{Frame, client, holder};

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
