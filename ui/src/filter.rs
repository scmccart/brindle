//! The match rule for filterable lists: a case-insensitive subsequence.

/// Case-insensitive subsequence match.
pub fn fuzzy_match(query: &str, candidate: &str) -> bool {
    let mut chars = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .all(|q| chars.any(|c| c == q))
}

/// Indices of the labels matching `query`, in their original order.
pub fn filter<'a>(query: &str, labels: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    labels
        .into_iter()
        .enumerate()
        .filter(|(_, label)| fuzzy_match(query, label))
        .map(|(ix, _)| ix)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy() {
        assert!(fuzzy_match("", "Shell"));
        assert!(fuzzy_match("de", "devbox (container)"));
        assert!(fuzzy_match("DC", "devbox (container)"));
        assert!(fuzzy_match("devbox co", "devbox (container)"));
        assert!(!fuzzy_match("zsh", "Shell"));
    }

    #[test]
    fn filtering_keeps_order() {
        let labels = ["Reload Config", "Clear Scrollback", "Close Tab", "Split Right"];
        assert_eq!(filter("", labels), vec![0, 1, 2, 3]);
        assert_eq!(filter("c", labels), vec![0, 1, 2]);
        assert_eq!(filter("clrsc", labels), vec![1]);
        assert!(filter("zzz", labels).is_empty());
    }
}
