//! Windows environment blocks for `CreateProcessW`, shared by both
//! binaries: the holder passes one to each ConPTY program it starts, and
//! Brindle passes one to a server core it starts for a given endpoint.
//!
//! Pure, so it builds and is tested on every platform.

/// The environment block for `vars`: `name=value` entries in UTF-16, each
/// NUL-terminated, sorted by name case-insensitively as `CreateProcessW`
/// requires, and ended by one more NUL. The variables are used as given:
/// the caller has already merged names that differ only in case.
pub fn environment_block<'a>(vars: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<u16> {
    let mut vars: Vec<(&str, &str)> = vars.into_iter().collect();
    vars.sort_by_cached_key(|(name, _)| name.to_uppercase());
    let mut block = Vec::new();
    for (name, value) in vars {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        // An empty block is still two NULs.
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(block: &[u16]) -> Vec<String> {
        assert_eq!(block.last(), Some(&0), "the block ends with a NUL");
        let body = &block[..block.len() - 1];
        if body == [0] {
            return Vec::new();
        }
        assert_eq!(body.last(), Some(&0), "each entry ends with a NUL");
        body[..body.len() - 1]
            .split(|unit| *unit == 0)
            .map(|entry| String::from_utf16(entry).unwrap())
            .collect()
    }

    #[test]
    fn environment_is_sorted_case_insensitively() {
        let block = environment_block([
            ("windir", "C:\\Windows"),
            ("ALLUSERSPROFILE", "C:\\ProgramData"),
            ("=C:", "C:\\src"),
            ("Path", "C:\\tools;C:\\Windows"),
            ("TERM", "xterm-256color"),
        ]);
        assert_eq!(
            entries(&block),
            [
                "=C:=C:\\src",
                "ALLUSERSPROFILE=C:\\ProgramData",
                "Path=C:\\tools;C:\\Windows",
                "TERM=xterm-256color",
                "windir=C:\\Windows",
            ]
        );
    }

    #[test]
    fn environment_values_keep_non_ascii_text() {
        let block = environment_block([("NAME", "caf\u{e9} \u{1f600}")]);
        assert_eq!(entries(&block), ["NAME=caf\u{e9} \u{1f600}"]);
    }

    #[test]
    fn an_empty_environment_is_two_nuls() {
        assert_eq!(environment_block([]), [0, 0]);
    }
}
