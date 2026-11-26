//! A file change's hunks, from its old and new text (design decision 4 of
//! `agents-tool-content`): a line diff by the `similar` crate's Myers, which
//! bounds its own work, with no deadline or timeout, so the same texts give
//! the same hunks in every core (`only_the_diff_module_names_similar`).
//! The only file naming `similar` (`ports R3`).

use brindle_transcript::{DiffLine, Hunk, ITEM_LIMIT};
use similar::algorithms::{Capture, Replace, myers};
use similar::{DiffTag, group_diff_ops};

/// Lines of context around each change.
const CONTEXT: usize = 3;

/// Texts with more lines than this, together, aren't compared: the change
/// is one hunk removing the old lines and adding the new (a guard on
/// memory).
pub const MAX_DIFF_LINES: usize = 200_000;

/// A change's hunks, and how many of its lines were left out past
/// [`ITEM_LIMIT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diffed {
    pub hunks: Vec<Hunk>,
    pub omitted: u32,
}

/// The hunks that turn `old` into `new`, three lines of context around
/// each change, starts counted from 1 and lines without their `\n` (a
/// trailing `\r` stays). A text with a final line break and one without
/// differ in their last line. Lines are kept until their text together
/// would pass [`ITEM_LIMIT`]; the rest are counted in `omitted`.
pub fn hunks(old: &str, new: &str) -> Diffed {
    let mut kept = Keep::default();
    if lines(old).count().saturating_add(lines(new).count()) > MAX_DIFF_LINES {
        let hunk = kept.hunk(1, 1);
        for line in lines(old) {
            kept.push(hunk, DiffLine::Removed(line.to_owned()));
        }
        for line in lines(new) {
            kept.push(hunk, DiffLine::Added(line.to_owned()));
        }
        return kept.done();
    }
    // Lines are compared with their `\n`, so a final line break counts.
    let old: Vec<&str> = old.split_inclusive('\n').collect();
    let new: Vec<&str> = new.split_inclusive('\n').collect();
    // Myers alone, rather than `TextDiff`, whose choice of algorithm at
    // run time builds every algorithm into the helper.
    // `Replace` orders a change's removed lines before its added ones,
    // as `TextDiff` does.
    let mut capture = Replace::new(Capture::new());
    match myers::diff(&mut capture, &old[..], 0..old.len(), &new[..], 0..new.len()) {
        Ok(()) => {}
        Err(never) => match never {},
    }
    for group in group_diff_ops(capture.into_inner().into_ops(), CONTEXT) {
        let Some(first) = group.first() else {
            continue;
        };
        let hunk = kept.hunk(first.old_range().start + 1, first.new_range().start + 1);
        for op in &group {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            let line = |text: &str| strip(text).to_owned();
            let (context, removed, added) = match tag {
                DiffTag::Equal => (old_range, 0..0, 0..0),
                DiffTag::Delete => (0..0, old_range, 0..0),
                DiffTag::Insert => (0..0, 0..0, new_range),
                DiffTag::Replace => (0..0, old_range, new_range),
            };
            for at in context {
                kept.push(hunk, DiffLine::Context(line(old[at])));
            }
            for at in removed {
                kept.push(hunk, DiffLine::Removed(line(old[at])));
            }
            for at in added {
                kept.push(hunk, DiffLine::Added(line(new[at])));
            }
        }
    }
    kept.done()
}

/// `text`'s lines, each without its `\n`.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive('\n').map(strip)
}

fn strip(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

/// The hunks so far, and what was left out once the limit was reached.
#[derive(Default)]
struct Keep {
    hunks: Vec<Hunk>,
    len: usize,
    full: bool,
    omitted: u32,
}

impl Keep {
    /// Starts a hunk, returning its index; `None` once the limit is
    /// reached (its lines are all left out).
    fn hunk(&mut self, old_start: usize, new_start: usize) -> Option<usize> {
        if self.full {
            return None;
        }
        self.hunks.push(Hunk {
            old_start: u32::try_from(old_start).unwrap_or(u32::MAX),
            new_start: u32::try_from(new_start).unwrap_or(u32::MAX),
            lines: Vec::new(),
        });
        Some(self.hunks.len() - 1)
    }

    fn push(&mut self, hunk: Option<usize>, line: DiffLine) {
        let len = self.len.saturating_add(line.text().len());
        match hunk.and_then(|at| self.hunks.get_mut(at)) {
            Some(hunk) if !self.full && len <= ITEM_LIMIT => {
                hunk.lines.push(line);
                self.len = len;
            }
            Some(_) | None => {
                self.full = true;
                self.omitted = self.omitted.saturating_add(1);
            }
        }
    }

    fn done(mut self) -> Diffed {
        self.hunks.retain(|hunk| !hunk.lines.is_empty());
        Diffed {
            hunks: self.hunks,
            omitted: self.omitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `old`'s lines with `hunks` applied, as lines.
    fn apply(old: &str, hunks: &[Hunk]) -> Vec<String> {
        let old: Vec<&str> = lines(old).collect();
        let mut out = Vec::new();
        let mut at = 0;
        for hunk in hunks {
            let start = hunk.old_start as usize - 1;
            assert!(start >= at, "hunks overlap or go backwards");
            out.extend(old[at..start].iter().map(|line| (*line).to_owned()));
            at = start;
            for line in &hunk.lines {
                match line {
                    DiffLine::Context(text) => {
                        assert_eq!(old[at], text, "context differs at {at}");
                        out.push(text.clone());
                        at += 1;
                    }
                    DiffLine::Removed(text) => {
                        assert_eq!(old[at], text, "removed line differs at {at}");
                        at += 1;
                    }
                    DiffLine::Added(text) => out.push(text.clone()),
                }
            }
        }
        out.extend(old[at..].iter().map(|line| (*line).to_owned()));
        out
    }

    fn check(old: &str, new: &str) -> Diffed {
        let diffed = hunks(old, new);
        assert_eq!(diffed.omitted, 0, "{old:?} -> {new:?}");
        assert_eq!(
            apply(old, &diffed.hunks),
            lines(new).collect::<Vec<_>>(),
            "{old:?} -> {new:?}: {diffed:?}"
        );
        diffed
    }

    #[test]
    fn applying_the_hunks_gives_the_new_text() {
        let table = [
            ("", ""),
            ("a\n", "a\n"),
            ("", "a\nb\n"),
            ("a\nb\n", ""),
            ("a\nb\nc\n", "a\nB\nc\n"),
            (
                "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
                "1\n2\n3\nfour\n5\n6\n7\n8\n9\nten\n",
            ),
            ("a\r\nb\r\n", "a\r\nc\r\n"),
            ("an apple a day", "an banana a day"),
            ("x\ny\n", "y\nx\n"),
        ];
        for (old, new) in table {
            check(old, new);
        }
    }

    #[test]
    fn line_numbers_count_from_1_and_match_the_texts() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n";
        let new = "1\n2\n3\n4\n5\n6\nseven\n8\n9\n10\n11\n12\n";
        let diffed = check(old, new);
        assert_eq!(diffed.hunks.len(), 1);
        let hunk = &diffed.hunks[0];
        // Three lines of context before line 7.
        assert_eq!((hunk.old_start, hunk.new_start), (4, 4));
        assert_eq!(hunk.lines[0], DiffLine::Context("4".into()));
        assert_eq!(hunk.lines[3], DiffLine::Removed("7".into()));
        assert_eq!(hunk.lines[4], DiffLine::Added("seven".into()));
        assert_eq!(hunk.lines.len(), 8);
        // Two changes far apart are two hunks.
        let far = "one\n2\n3\n4\n5\n6\nseven\n8\n9\n10\n11\ntwelve\n";
        let diffed = check(new, far);
        assert_eq!(diffed.hunks.len(), 2);
        assert_eq!(diffed.hunks[1].old_start, 9);
    }

    #[test]
    fn an_added_file_is_one_hunk_of_added_lines() {
        let diffed = check("", "a\nb\nc");
        assert_eq!(
            diffed.hunks,
            [Hunk {
                old_start: 1,
                new_start: 1,
                lines: vec![
                    DiffLine::Added("a".into()),
                    DiffLine::Added("b".into()),
                    DiffLine::Added("c".into()),
                ],
            }]
        );
    }

    #[test]
    fn a_final_line_break_counts_as_a_change_of_the_last_line() {
        let with = check("a\nb", "a\nb\n");
        assert_eq!(
            with.hunks[0].lines,
            [
                DiffLine::Context("a".into()),
                DiffLine::Removed("b".into()),
                DiffLine::Added("b".into()),
            ]
        );
        check("a\nb\n", "a\nb");
        assert!(check("a\nb", "a\nb").hunks.is_empty());
    }

    struct XorShift(u64);

    impl XorShift {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }

        fn text(&mut self) -> String {
            let words = ["a", "b", "c", "dd", "", "e\r"];
            let n = self.below(12);
            let mut text = String::new();
            for _ in 0..n {
                text.push_str(words[self.below(words.len() as u64) as usize]);
                text.push('\n');
            }
            if self.below(3) == 0 {
                text.push_str("tail");
            }
            text
        }

        fn edit(&mut self, old: &str) -> String {
            let mut lines: Vec<String> = old.split_inclusive('\n').map(str::to_owned).collect();
            for _ in 0..self.below(4) {
                let at = self.below(lines.len() as u64 + 1) as usize;
                match self.below(3) {
                    0 if at < lines.len() => {
                        lines.remove(at);
                    }
                    1 => lines.insert(at, "new\n".into()),
                    _ => {
                        if let Some(line) = lines.get_mut(at) {
                            *line = "changed\n".into();
                        }
                    }
                }
            }
            lines.concat()
        }
    }

    #[test]
    fn random_pairs_apply_to_their_new_text() {
        let mut rng = XorShift(0x5eed_d1ff);
        for _ in 0..10_000 {
            let old = rng.text();
            let new = if rng.below(2) == 0 {
                rng.edit(&old)
            } else {
                rng.text()
            };
            check(&old, &new);
        }
    }

    #[test]
    fn unrelated_large_texts_finish_and_are_cut_at_the_item_limit() {
        let old: String = (0..50_000).map(|n| format!("old line {n}\n")).collect();
        let new: String = (0..50_000).map(|n| format!("new line {n}\n")).collect();
        let diffed = hunks(&old, &new);
        let kept: usize = diffed
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .map(|line| line.text().len())
            .sum();
        assert!(kept <= ITEM_LIMIT);
        assert!(kept > ITEM_LIMIT - 64, "{kept}");
        let lines: usize = diffed.hunks.iter().map(|hunk| hunk.lines.len()).sum();
        assert_eq!(lines + diffed.omitted as usize, 100_000);
    }

    #[test]
    fn past_the_line_guard_the_change_is_one_replacing_hunk() {
        let old = "x\n".repeat(MAX_DIFF_LINES / 2);
        let new = format!("{old}y\n");
        let diffed = hunks(&old, &new);
        assert_eq!(diffed.hunks.len(), 1);
        let hunk = &diffed.hunks[0];
        assert_eq!((hunk.old_start, hunk.new_start), (1, 1));
        assert!(
            hunk.lines
                .iter()
                .all(|line| *line == DiffLine::Removed("x".into()))
        );
        assert_eq!(
            hunk.lines.len() + diffed.omitted as usize,
            MAX_DIFF_LINES + 1
        );
        // At the guard, the texts are still compared.
        let at = "x\n".repeat(MAX_DIFF_LINES / 2 - 1);
        let diffed = hunks(&at, &format!("{at}y\n"));
        assert_eq!(
            diffed.hunks[0].lines.last(),
            Some(&DiffLine::Added("y".into()))
        );
        assert_eq!(diffed.omitted, 0);
    }

    #[test]
    fn the_same_texts_give_the_same_hunks() {
        let mut rng = XorShift(0xd17f);
        for _ in 0..200 {
            let (old, new) = (rng.text(), rng.text());
            assert_eq!(hunks(&old, &new), hunks(&old, &new));
        }
        let old: String = (0..5_000).map(|n| format!("{}\n", n % 7)).collect();
        let new: String = (0..5_000).map(|n| format!("{}\n", n % 11)).collect();
        assert_eq!(hunks(&old, &new), hunks(&old, &new));
    }
}
