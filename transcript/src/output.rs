use serde::{Deserialize, Serialize};

use crate::event::{is_false, is_zero_u64};
use crate::{Exit, ITEM_LIMIT};

/// One command's output, as plain text. `omitted` counts the bytes dropped
/// from its start past [`ITEM_LIMIT`]; `partial` says the kept text starts
/// inside a line whose start was dropped, which the fold needs to keep
/// whole lines the same however the output was split.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub omitted: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
    /// How the command ended; `None` while it runs, or when it never said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<Exit>,
}

impl Output {
    /// Appends `text` and keeps the longest suffix within [`ITEM_LIMIT`]
    /// (design decision 8): whole lines are dropped from the start, and a
    /// single line longer than the limit keeps its last bytes, cut at a
    /// character boundary. Appending `a` then `b` gives what appending
    /// `a + b` does. Returns the bytes dropped, for the entry's size.
    pub(crate) fn append(&mut self, text: &str) -> String {
        self.text.push_str(text);
        let cut = self.cut();
        if cut == 0 {
            return String::new();
        }
        let partial = self.text.as_bytes().get(cut - 1) != Some(&b'\n');
        let dropped: String = self.text.drain(..cut).collect();
        self.omitted = self.omitted.saturating_add(dropped.len() as u64);
        self.partial = partial;
        dropped
    }

    /// Where the kept text starts: the first line start at or after the
    /// limit's reach, or failing one the first character boundary there.
    /// The text's own start is a line start unless it is `partial`.
    fn cut(&self) -> usize {
        let text = &self.text;
        let low = text.len().saturating_sub(ITEM_LIMIT);
        if low == 0 && !self.partial {
            return 0;
        }
        if low > 0 && text.as_bytes().get(low - 1) == Some(&b'\n') {
            return low;
        }
        // The first character boundary from `low`: a line break is one
        // byte, so none is skipped.
        let boundary = (low..=text.len())
            .find(|&at| text.is_char_boundary(at))
            .unwrap_or(text.len());
        match text.get(boundary..).and_then(|rest| rest.find('\n')) {
            Some(at) if boundary + at + 1 < text.len() => boundary + at + 1,
            Some(_) | None => boundary,
        }
    }
}

/// The encoded length of an output's fields that `ToolOutput` changes other
/// than its text: `omitted`, `partial` and `exit`, each with its comma,
/// none when unset (every one follows the item's tag and its text).
pub(crate) fn output_fields_len(output: &Output) -> usize {
    let omitted = if output.omitted == 0 {
        0
    } else {
        ",\"omitted\":".len() + output.omitted.to_string().len()
    };
    let partial = if output.partial {
        ",\"partial\":true".len()
    } else {
        0
    };
    let exit = output.exit.as_ref().map_or(0, |exit| {
        ",\"exit\":".len() + serde_json::to_vec(exit).map_or(0, |bytes| bytes.len())
    });
    omitted + partial + exit
}

#[cfg(test)]
mod tests {
    use crate::fuzz::XorShift;
    use crate::harness::*;
    use crate::*;

    #[test]
    fn output_appends_and_keeps_whole_lines_within_the_limit() {
        let mut transcript = fold(&[
            added(1, tool("c1")),
            items(1, 0, vec![text_item("t"), output_item(Some("seq"))]),
        ])
        .unwrap();
        transcript.apply(&output(1, 1, "1\n2\n", None)).unwrap();
        transcript
            .apply(&output(1, 1, "3\n", Some(Exit::Code { code: 0 })))
            .unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[1] else {
            panic!()
        };
        assert_eq!(
            (kept.text.as_str(), kept.omitted, kept.partial, &kept.exit),
            ("1\n2\n3\n", 0, false, &Some(Exit::Code { code: 0 }))
        );
        assert_eq!(transcript.encoded_len(), measured(&transcript));

        // Past the limit: whole lines go from the start.
        let line = format!("{}\n", "x".repeat(99));
        let mut transcript = fold(&[added(1, tool("c1")), items(1, 0, vec![output_item(None)])])
            .unwrap();
        let lines = ITEM_LIMIT / 100 + 10;
        transcript
            .apply(&output(1, 0, &line.repeat(lines), None))
            .unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[0] else {
            panic!()
        };
        assert_eq!(kept.text, line.repeat(ITEM_LIMIT / 100));
        assert_eq!(kept.omitted, (line.len() * (lines - ITEM_LIMIT / 100)) as u64);
        assert!(!kept.partial);
        assert_eq!(transcript.encoded_len(), measured(&transcript));

        // A single line past the limit keeps its last bytes, at a
        // character boundary.
        let mut transcript = fold(&[added(1, tool("c1")), items(1, 0, vec![output_item(None)])])
            .unwrap();
        let long = format!("a{}", "é".repeat(ITEM_LIMIT));
        transcript.apply(&output(1, 0, &long, None)).unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[0] else {
            panic!()
        };
        assert_eq!(kept.text, "é".repeat(ITEM_LIMIT / 2));
        assert_eq!(kept.omitted as usize, long.len() - ITEM_LIMIT);
        assert!(kept.partial);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
        // Its line ends, and the next one drops what was left of it.
        transcript.apply(&output(1, 0, "\nnext", None)).unwrap();
        let ToolItem::Output(kept) = &content_of(&transcript, 1)[0] else {
            panic!()
        };
        assert_eq!(kept.text, "next");
        assert!(!kept.partial);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
    }

    /// Folding `ToolOutput(a)` then `ToolOutput(b)` equals folding
    /// `ToolOutput(a + b)` (design decision 8), which the checkpoint's
    /// merge relies on: random texts, split at random points.
    #[test]
    fn output_appended_in_pieces_folds_as_appended_whole() {
        const PIECES: &[&str] = &["a", "bc", "\n", "é", "🦀", "\"", "\\", "\t", "line\n"];
        let mut rng = XorShift::new(0x7e57_00a1);
        for round in 0..1000 {
            // Some texts reach past the limit, with long lines or none.
            let target = if round % 3 == 0 {
                ITEM_LIMIT + rng.below(2 * ITEM_LIMIT)
            } else {
                rng.below(300)
            };
            let newlines = rng.below(4) > 0;
            let mut text = String::new();
            while text.len() < target {
                let piece = PIECES[rng.below(PIECES.len())];
                if piece.contains('\n') && !newlines {
                    continue;
                }
                text.push_str(&piece.repeat(1 + rng.below(if target > 300 { 200 } else { 2 })));
                // Long runs without a line break.
                if rng.below(50) == 0 {
                    text.push_str(&"z".repeat(rng.below(ITEM_LIMIT + 64)));
                }
            }
            let start = [added(1, tool("c1")), items(1, 0, vec![output_item(None)])];
            let whole = {
                let mut transcript = fold(&start).unwrap();
                transcript.apply(&output(1, 0, &text, None)).unwrap();
                transcript
            };
            let mut pieces = fold(&start).unwrap();
            let mut rest = text.as_str();
            while !rest.is_empty() {
                let mut cut = 1 + rng.below(rest.len());
                while !rest.is_char_boundary(cut) {
                    cut += 1;
                }
                let (piece, after) = rest.split_at(cut);
                pieces.apply(&output(1, 0, piece, None)).unwrap();
                rest = after;
            }
            assert_eq!(pieces, whole, "round {round}");
            assert_eq!(whole.encoded_len(), measured(&whole));
        }
    }
}
