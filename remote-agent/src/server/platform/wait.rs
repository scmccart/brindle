//! A wait's timeout as Win32 takes it: whole milliseconds, never
//! `INFINITE`.
//!
//! Pure, so it is tested on every platform; the Windows `event_loop`,
//! `pipe` and `listen` call it for their waits.

use std::time::Duration;

/// Win32's `INFINITE` timeout, which [`millis`] never gives. The Windows
/// event loop's tests check it against the `windows` crate's constant.
pub const INFINITE: u32 = u32::MAX;

/// A timeout in milliseconds for a Win32 wait, rounded up so that a short
/// one doesn't spin at 0 ms, and never `INFINITE`.
pub fn millis(timeout: Duration) -> u32 {
    timeout
        .as_nanos()
        .div_ceil(1_000_000)
        .min(u128::from(INFINITE - 1)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_time_is_zero() {
        assert_eq!(millis(Duration::ZERO), 0);
    }

    /// Any part of a millisecond counts as a whole one, so a short wait
    /// never becomes a 0 ms one that returns at once.
    #[test]
    fn a_part_of_a_millisecond_rounds_up() {
        assert_eq!(millis(Duration::from_nanos(1)), 1);
        assert_eq!(millis(Duration::from_micros(999)), 1);
        assert_eq!(millis(Duration::from_millis(1)), 1);
        assert_eq!(millis(Duration::from_millis(1) + Duration::from_nanos(1)), 2);
        assert_eq!(millis(Duration::from_micros(1500)), 2);
        assert_eq!(millis(Duration::from_secs(10)), 10_000);
    }

    /// A timeout too long for a `u32` stops one short of `INFINITE`, which
    /// would wait forever.
    #[test]
    fn a_long_timeout_stops_short_of_infinite() {
        let longest = INFINITE - 1;
        assert_eq!(millis(Duration::from_millis(u64::from(longest) - 1)), longest - 1);
        assert_eq!(millis(Duration::from_millis(u64::from(longest))), longest);
        assert_eq!(millis(Duration::from_millis(u64::from(INFINITE))), longest);
        assert_eq!(
            millis(Duration::from_millis(u64::from(longest)) + Duration::from_nanos(1)),
            longest
        );
        assert_eq!(millis(Duration::from_secs(u64::MAX)), longest);
        assert_eq!(millis(Duration::MAX), longest);
    }
}
