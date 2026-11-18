//! What bounds the input waiting to reach one session (`process R9`): its
//! budget, counted from when Brindle takes the bytes until the connection's
//! writer has written them (or skipped them), across the session's start
//! and any time its link is down or slow.
//!
//! A call is taken whole while less than the high mark waits, so a paste
//! into a connected tab goes through however big it is, and waiting input
//! is bounded by the high mark plus one call. Past the high mark input is
//! refused until less than the low mark waits. The first refusal of each
//! crossing is [`Refusal::First`], which the tab says once; the rest are
//! [`Refusal::Again`].

use crate::queue_marks::QueueMarks;

/// The marks on a session's waiting input, in bytes: the high one is the
/// holder's own input cap, so the server never drops input Brindle took for
/// a live session.
pub const INPUT_MARKS: QueueMarks = QueueMarks::new(4 << 20, 1 << 20);

/// How much of a session's input waits, and whether that is over the marks.
#[derive(Debug, Default)]
pub struct InputBudget {
    waiting: usize,
    over: bool,
}

/// What [`InputBudget::offer`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Send it; it is counted until [`InputBudget::handled`].
    Take,
    Refuse(Refusal),
}

/// Which refusal of a crossing this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The one that found the budget going over: the tab says so.
    First,
    /// Any later one, until the budget is back under its low mark.
    Again,
}

/// What became of input given to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// It waits to be written.
    Queued,
    Refused(Refusal),
    /// The session is no longer the tab's: nothing is counted or sent.
    Discarded,
}

impl InputBudget {
    /// Input of `len` bytes for the session: taken and counted while it is
    /// under the marks, refused while it is over them.
    pub fn offer(&mut self, len: usize) -> Offer {
        let over = INPUT_MARKS.over(self.over, self.waiting);
        let going_over = over && !self.over;
        self.over = over;
        if !over {
            self.waiting += len;
            Offer::Take
        } else if going_over {
            Offer::Refuse(Refusal::First)
        } else {
            Offer::Refuse(Refusal::Again)
        }
    }

    /// `len` bytes of taken input were written, or skipped: they no longer
    /// wait. The next offer finds the budget back under once less than the
    /// low mark waits.
    pub fn handled(&mut self, len: usize) {
        self.waiting = self.waiting.saturating_sub(len);
    }

    /// How many taken bytes wait.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.waiting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1 << 20;
    const HIGH: usize = 4 * MIB;
    const LOW: usize = MIB;

    #[test]
    fn input_is_taken_while_under_the_high_mark() {
        let mut budget = InputBudget::default();
        for _ in 0..3 {
            assert_eq!(budget.offer(MIB), Offer::Take);
        }
        assert_eq!(budget.offer(MIB - 1), Offer::Take);
        // One byte under the high mark waits: still taken.
        assert_eq!(budget.offer(64 * 1024), Offer::Take);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::First));
    }

    #[test]
    fn the_first_refusal_after_going_over_is_first_and_the_rest_again() {
        let mut budget = InputBudget::default();
        assert_eq!(budget.offer(HIGH), Offer::Take);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::First));
        for _ in 0..5 {
            assert_eq!(budget.offer(1), Offer::Refuse(Refusal::Again));
        }
        // Back under, then over again: a new crossing, a new first.
        budget.handled(HIGH);
        assert_eq!(budget.offer(HIGH), Offer::Take);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::First));
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::Again));
    }

    #[test]
    fn input_is_taken_again_once_under_the_low_mark() {
        let mut budget = InputBudget::default();
        assert_eq!(budget.offer(HIGH), Offer::Take);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::First));
        // Under the high mark isn't enough, nor is the low mark itself.
        budget.handled(HIGH - LOW);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::Again));
        // Under the low mark is.
        budget.handled(1);
        assert_eq!(budget.offer(10), Offer::Take);
        assert_eq!(budget.offer(10), Offer::Take);
    }

    #[test]
    fn one_call_past_the_high_mark_is_taken_whole() {
        let mut budget = InputBudget::default();
        assert_eq!(budget.offer(6 * MIB), Offer::Take);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::First));
        // Written in pieces, it frees the budget only once under the low mark.
        budget.handled(5 * MIB);
        assert_eq!(budget.offer(1), Offer::Refuse(Refusal::Again));
        budget.handled(1);
        assert_eq!(budget.offer(1), Offer::Take);
    }

    #[test]
    fn handled_never_goes_below_zero() {
        let mut budget = InputBudget::default();
        budget.handled(10);
        assert_eq!(budget.offer(HIGH - 1), Offer::Take);
        assert_eq!(budget.offer(1), Offer::Take);
        assert_eq!(
            budget.offer(1),
            Offer::Refuse(Refusal::First),
            "{HIGH} bytes wait, no more"
        );
        budget.handled(usize::MAX);
        assert_eq!(budget.offer(1), Offer::Take);
    }
}
