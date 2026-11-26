//! An agent's stdout split into lines (design decisions 11 and 13): ACP is
//! one JSON-RPC message per line, and the bytes come in chunks that cut
//! anywhere, inside a UTF-8 sequence or a `\r\n` included. [`Lines`] keeps
//! the partial line between chunks, bounds a line at [`MAX_LINE`] (a longer
//! one is dropped to its end and reported, never buffered), resyncs at the
//! next newline after the front of the output was cut (a truncated tail),
//! and counts every byte it was fed ([`Lines::covers`]), which a checkpoint's
//! `covers` reports.
//!
//! A line is bytes: [`super::wire::decode`] replaces invalid UTF-8.

/// The longest line kept, without its line end: 8 MiB.
pub const MAX_LINE: usize = 8 * 1024 * 1024;

/// What a fed chunk completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// A whole line, without its `\n` or `\r\n`.
    Line(Vec<u8>),
    /// A line longer than [`MAX_LINE`], dropped; `len` is its length
    /// without the `\n`.
    TooLong { len: u64 },
    /// The bytes dropped after a cut front, up to and without the first
    /// `\n`, the partial line held before the cut included.
    Lost { len: u64 },
}

/// The line in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Partial {
    /// Bytes of a line not yet ended; at most [`MAX_LINE`] + 1 (room for a
    /// `\r` before the `\n`).
    Collecting(Vec<u8>),
    /// A line already past the bound, dropped as it comes; `len` bytes so far.
    TooLong { len: u64 },
    /// The front was cut: dropping up to the next newline; `len` bytes so far.
    Resyncing { len: u64 },
}

/// Splits an agent's stdout into bounded lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lines {
    partial: Partial,
    covers: u64,
    limit: usize,
}

impl Default for Lines {
    fn default() -> Self {
        Self::new()
    }
}

impl Lines {
    /// Before the agent's first byte.
    pub fn new() -> Self {
        Self::restore(Partial::Collecting(Vec::new()), 0)
    }

    /// As a checkpoint left it: `partial` and the bytes fed so far. A
    /// collected partial line already past the bound becomes
    /// [`Partial::TooLong`].
    pub fn restore(partial: Partial, covers: u64) -> Self {
        Self::bounded(partial, covers, MAX_LINE)
    }

    fn bounded(partial: Partial, covers: u64, limit: usize) -> Self {
        let partial = match partial {
            Partial::Collecting(bytes) if bytes.len() > limit + 1 => Partial::TooLong {
                len: len(bytes.len()),
            },
            kept @ (Partial::Collecting(_)
            | Partial::TooLong { .. }
            | Partial::Resyncing { .. }) => kept,
        };
        Self {
            partial,
            covers,
            limit,
        }
    }

    /// The line in progress.
    pub fn partial(&self) -> &Partial {
        &self.partial
    }

    /// Every byte fed so far, the partial line's included.
    pub fn covers(&self) -> u64 {
        self.covers
    }

    /// The bytes fed next follow a gap (the holder's tail was truncated):
    /// the partial line can't be finished, so it is dropped with the bytes
    /// up to the next newline, and reported as [`Piece::Lost`] there.
    pub fn cut_front(&mut self) {
        let held = match &self.partial {
            Partial::Collecting(bytes) => len(bytes.len()),
            Partial::TooLong { len } | Partial::Resyncing { len } => *len,
        };
        self.partial = Partial::Resyncing { len: held };
    }

    /// Feeds a chunk of stdout and returns what it completed, in order.
    pub fn feed(&mut self, mut bytes: &[u8]) -> Vec<Piece> {
        self.covers = self.covers.saturating_add(len(bytes.len()));
        let mut pieces = Vec::new();
        while !bytes.is_empty() {
            let end = bytes.iter().position(|byte| *byte == b'\n');
            let (head, rest) = match end {
                Some(end) => (&bytes[..end], Some(&bytes[end + 1..])),
                None => (bytes, None),
            };
            self.extend(head);
            let Some(rest) = rest else {
                break;
            };
            pieces.push(self.finish());
            bytes = rest;
        }
        pieces
    }

    /// Adds bytes with no newline to the line in progress.
    fn extend(&mut self, head: &[u8]) {
        match &mut self.partial {
            Partial::Collecting(line) => {
                if line.len() + head.len() > self.limit + 1 {
                    self.partial = Partial::TooLong {
                        len: len(line.len() + head.len()),
                    };
                } else {
                    line.extend_from_slice(head);
                }
            }
            Partial::TooLong { len: dropped } | Partial::Resyncing { len: dropped } => {
                *dropped = dropped.saturating_add(len(head.len()));
            }
        }
    }

    /// Ends the line in progress at a newline.
    fn finish(&mut self) -> Piece {
        match std::mem::replace(&mut self.partial, Partial::Collecting(Vec::new())) {
            Partial::Collecting(mut line) => {
                let whole = line.len();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.len() > self.limit {
                    Piece::TooLong { len: len(whole) }
                } else {
                    Piece::Line(line)
                }
            }
            Partial::TooLong { len } => Piece::TooLong { len },
            Partial::Resyncing { len } => Piece::Lost { len },
        }
    }
}

fn len(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Piece {
        Piece::Line(text.as_bytes().to_vec())
    }

    #[test]
    fn a_chunk_of_whole_lines_gives_each_line() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(b"one\ntwo\n"), [line("one"), line("two")]);
        assert_eq!(lines.partial(), &Partial::Collecting(Vec::new()));
    }

    #[test]
    fn a_partial_line_is_kept_until_its_newline() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(b"{\"a\":"), []);
        assert_eq!(lines.partial(), &Partial::Collecting(b"{\"a\":".to_vec()));
        assert_eq!(lines.feed(b"1}\n{"), [line("{\"a\":1}")]);
        assert_eq!(lines.partial(), &Partial::Collecting(b"{".to_vec()));
    }

    #[test]
    fn a_chunk_boundary_inside_a_utf8_sequence_joins_the_sequence() {
        let text = "é€🦀\n".as_bytes();
        for cut in 1..text.len() {
            let mut lines = Lines::new();
            let mut pieces = lines.feed(&text[..cut]);
            pieces.extend(lines.feed(&text[cut..]));
            assert_eq!(pieces, [line("é€🦀")], "cut at {cut}");
        }
    }

    #[test]
    fn a_crlf_line_end_is_removed_even_split_between_chunks() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(b"a\r"), []);
        assert_eq!(lines.feed(b"\nb\r\n"), [line("a"), line("b")]);
    }

    #[test]
    fn a_carriage_return_inside_a_line_is_kept() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(b"a\rb\n"), [line("a\rb")]);
    }

    #[test]
    fn an_empty_line_is_a_line() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(b"\n\r\n"), [line(""), line("")]);
    }

    #[test]
    fn a_line_of_exactly_8_mib_is_kept() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(&vec![b'x'; MAX_LINE]), []);
        assert_eq!(lines.feed(b"\r"), []);
        let pieces = lines.feed(b"\n");
        assert_eq!(pieces.len(), 1);
        assert!(matches!(&pieces[0], Piece::Line(kept) if kept.len() == MAX_LINE));
        assert_eq!(lines.covers(), len(MAX_LINE + 2));
    }

    #[test]
    fn a_line_over_8_mib_is_dropped_to_its_end_and_reported() {
        let mut lines = Lines::new();
        let text = vec![b'x'; MAX_LINE + 1];
        assert_eq!(lines.feed(&text), []);
        assert_eq!(
            lines.feed(b"\nnext\n"),
            [
                Piece::TooLong {
                    len: len(MAX_LINE + 1)
                },
                line("next"),
            ]
        );
    }

    #[test]
    fn a_long_line_is_not_buffered_past_the_bound() {
        let mut lines = Lines::new();
        for _ in 0..3 {
            assert_eq!(lines.feed(&vec![b'x'; MAX_LINE / 2 + 1]), []);
        }
        let dropped = len(3 * (MAX_LINE / 2 + 1));
        assert_eq!(lines.partial(), &Partial::TooLong { len: dropped });
        assert_eq!(lines.feed(b"\n"), [Piece::TooLong { len: dropped }]);
    }

    #[test]
    fn a_cut_front_drops_the_partial_line_and_resyncs_at_the_next_newline() {
        let mut lines = Lines::new();
        assert_eq!(lines.feed(b"{\"jsonrpc\""), []);
        lines.cut_front();
        assert_eq!(
            lines.feed(b"ult\":{}}\n{\"a\":1}\n"),
            [Piece::Lost { len: 10 + 8 }, line("{\"a\":1}"),]
        );
    }

    #[test]
    fn a_cut_front_at_a_line_start_still_drops_to_the_first_newline() {
        let mut lines = Lines::new();
        lines.cut_front();
        assert_eq!(
            lines.feed(b"whole\nnext\n"),
            [Piece::Lost { len: 5 }, line("next")]
        );
    }

    #[test]
    fn covers_counts_every_byte_fed_partial_line_included() {
        let mut lines = Lines::new();
        lines.feed(b"a\nbc");
        lines.cut_front();
        lines.feed(b"d");
        assert_eq!(lines.covers(), 5);
    }

    #[test]
    fn a_restored_lines_continues_its_partial_line() {
        let mut lines = Lines::restore(Partial::Collecting(b"ab".to_vec()), 40);
        assert_eq!(lines.feed(b"c\n"), [line("abc")]);
        assert_eq!(lines.covers(), 42);
        let mut lines = Lines::restore(Partial::TooLong { len: 9 }, 0);
        assert_eq!(lines.feed(b"x\n"), [Piece::TooLong { len: 10 }]);
    }

    #[test]
    fn a_restored_partial_line_past_the_bound_is_dropped() {
        let partial = Partial::Collecting(vec![b'x'; MAX_LINE + 2]);
        let lines = Lines::restore(partial, 0);
        assert_eq!(
            lines.partial(),
            &Partial::TooLong {
                len: len(MAX_LINE + 2)
            }
        );
    }

    /// A fixed-seed xorshift, so the fuzz run is the same every time.
    struct XorShift(u64);

    impl XorShift {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as usize
        }
    }

    /// What a stream splits into, by a plain reading of the rules, with a
    /// line bound of `limit`.
    fn expected(stream: &[u8], limit: usize) -> (Vec<Piece>, Vec<u8>) {
        let mut parts: Vec<&[u8]> = stream.split(|b| *b == b'\n').collect();
        let rest = parts.pop().unwrap_or_default().to_vec();
        let pieces = parts
            .into_iter()
            .map(|part| {
                let content = part.strip_suffix(b"\r").unwrap_or(part);
                if content.len() > limit {
                    Piece::TooLong {
                        len: len(part.len()),
                    }
                } else {
                    Piece::Line(content.to_vec())
                }
            })
            .collect();
        (pieces, rest)
    }

    #[test]
    fn any_chunking_of_a_stream_gives_the_same_lines() {
        let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
        let alphabet = b"ab\r\n\n{\xc3\xa9";
        for _ in 0..2_000 {
            let limit = 1 + rng.below(12);
            let stream: Vec<u8> = (0..rng.below(80))
                .map(|_| alphabet[rng.below(alphabet.len())])
                .collect();
            let mut lines = Lines::bounded(Partial::Collecting(Vec::new()), 0, limit);
            let mut pieces = Vec::new();
            let mut rest = stream.as_slice();
            while !rest.is_empty() {
                let (chunk, tail) = rest.split_at(1 + rng.below(rest.len()));
                pieces.extend(lines.feed(chunk));
                match lines.partial() {
                    Partial::Collecting(held) => assert!(held.len() <= limit + 1),
                    Partial::TooLong { .. } => {}
                    Partial::Resyncing { .. } => panic!("never cut"),
                }
                rest = tail;
            }
            let (want, held) = expected(&stream, limit);
            assert_eq!(pieces, want, "{stream:?} limit {limit}");
            assert_eq!(lines.covers(), len(stream.len()));
            match lines.partial() {
                Partial::Collecting(partial) => assert_eq!(partial, &held),
                Partial::TooLong { len: dropped } => {
                    assert!(held.len() > limit + 1);
                    assert_eq!(*dropped, len(held.len()));
                }
                Partial::Resyncing { .. } => panic!("never cut"),
            }
        }
    }
}
