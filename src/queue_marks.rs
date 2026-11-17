//! The hysteresis rule every bounded queue between Brindle's threads uses
//! (`process R9`): a queue goes over at its high mark and stays over until
//! it falls under its low one, so the producer isn't switched on and off at
//! every entry.

/// A queue's high and low marks, in whatever unit the queue counts
/// (entries, bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueMarks {
    high: usize,
    low: usize,
}

impl QueueMarks {
    /// Marks with `low < high`; anything else is a programming error, caught
    /// when the constant is built.
    pub const fn new(high: usize, low: usize) -> Self {
        assert!(low < high, "a queue's low mark must be under its high mark");
        Self { high, low }
    }

    /// Whether a queue at `level` is over, given whether it was: one that
    /// wasn't goes over at the high mark, and one that was stays over until
    /// it is under the low mark.
    pub fn over(self, was_over: bool, level: usize) -> bool {
        if was_over {
            level >= self.low
        } else {
            level >= self.high
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKS: QueueMarks = QueueMarks::new(10, 4);

    #[test]
    fn a_queue_goes_over_at_the_high_mark() {
        assert!(!MARKS.over(false, 0));
        assert!(!MARKS.over(false, 9));
        assert!(MARKS.over(false, 10));
        assert!(MARKS.over(false, 11));
    }

    #[test]
    fn it_stays_over_until_under_the_low_mark() {
        let mut over = false;
        for level in [10, 9, 6, 5] {
            over = MARKS.over(over, level);
            assert!(over, "at {level}");
        }
        over = MARKS.over(over, 3);
        assert!(!over, "under the low mark");
        // Back under, it takes the high mark again to go over.
        for level in [4, 5, 9] {
            over = MARKS.over(over, level);
            assert!(!over, "at {level}");
        }
        assert!(MARKS.over(over, 10));
    }

    #[test]
    fn the_marks_themselves_are_over() {
        // Reaching the high mark goes over; sitting on the low one stays over.
        assert!(MARKS.over(false, 10));
        assert!(MARKS.over(true, 4));
        assert!(!MARKS.over(true, 3));
        assert!(!MARKS.over(false, 4));
    }

    #[test]
    #[should_panic(expected = "low mark")]
    fn marks_must_be_apart() {
        let _ = QueueMarks::new(4, 4);
    }
}
