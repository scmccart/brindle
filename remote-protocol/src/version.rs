//! Protocol version ranges and their negotiation, shared by the client
//! protocol and H1.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The protocol versions one side speaks, `min..=max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRange {
    pub min: u32,
    pub max: u32,
}

impl VersionRange {
    pub const fn new(min: u32, max: u32) -> Self {
        Self { min, max }
    }

    pub const fn contains(&self, version: u32) -> bool {
        self.min <= version && version <= self.max
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.min == self.max {
            write!(f, "{}", self.min)
        } else {
            write!(f, "{}–{}", self.min, self.max)
        }
    }
}

/// Why two sides can't agree on a version. Its message names both ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiationError {
    /// The range of the side that asked (a client's or a core's `HELLO`).
    pub offered: VersionRange,
    /// The range of the side that answers (the core, or a holder).
    pub supported: VersionRange,
}

impl fmt::Display for NegotiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let malformed = |range: &VersionRange| range.min > range.max;
        if malformed(&self.offered) || malformed(&self.supported) {
            write!(f, "invalid protocol range: ")?;
        } else {
            write!(f, "no common protocol version: ")?;
        }
        write!(f, "offered {}, supported {}", self.offered, self.supported)
    }
}

impl std::error::Error for NegotiationError {}

impl From<NegotiationError> for std::io::Error {
    fn from(error: NegotiationError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, error)
    }
}

/// Picks the highest version both ranges contain: `offered` is the range in
/// the asking side's `HELLO`, `supported` the answering side's. A range with
/// `min > max` shares nothing.
pub fn negotiate(offered: VersionRange, supported: VersionRange) -> Result<u32, NegotiationError> {
    let low = offered.min.max(supported.min);
    let high = offered.max.min(supported.max);
    if offered.min > offered.max || supported.min > supported.max || low > high {
        return Err(NegotiationError { offered, supported });
    }
    Ok(high)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn r(min: u32, max: u32) -> VersionRange {
        VersionRange::new(min, max)
    }

    #[test]
    fn picks_the_highest_common_version() {
        assert_eq!(negotiate(r(1, 1), r(1, 1)), Ok(1));
        assert_eq!(negotiate(r(1, 3), r(2, 5)), Ok(3));
        assert_eq!(negotiate(r(2, 5), r(1, 3)), Ok(3));
        assert_eq!(negotiate(r(1, 9), r(4, 4)), Ok(4));
        assert_eq!(negotiate(r(3, 3), r(1, 7)), Ok(3));
        // Touching at one end.
        assert_eq!(negotiate(r(1, 2), r(2, 4)), Ok(2));
    }

    #[test]
    fn disjoint_ranges_name_both_sides() {
        let error = negotiate(r(1, 2), r(3, 4)).unwrap_err();
        assert_eq!(error.offered, r(1, 2));
        assert_eq!(error.supported, r(3, 4));
        let text = error.to_string();
        assert!(text.contains("1–2") && text.contains("3–4"), "{text}");
        assert!(text.contains("no common"), "{text}");

        let error = negotiate(r(5, 5), r(1, 1)).unwrap_err();
        let text = error.to_string();
        assert!(text.ends_with("offered 5, supported 1"), "{text}");
    }

    #[test]
    fn inverted_ranges_never_match() {
        let error = negotiate(r(3, 1), r(1, 3)).unwrap_err();
        assert!(error.to_string().starts_with("invalid"), "{error}");
        assert!(negotiate(r(1, 3), r(2, 1)).is_err());
        assert_eq!(
            std::io::Error::from(error).kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
