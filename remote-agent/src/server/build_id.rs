//! The running binary's build id ([`brindle_remote_protocol::build_id`]).
//! It is computed once at startup from `/proc/self/exe`, which still names
//! the running binary after an install replaces the file.

use std::fs;
use std::io;

use brindle_remote_protocol::build_id::of_binary;

/// The build id of `/proc/self/exe`, as lower-case hex.
pub fn current() -> io::Result<String> {
    let bytes = fs::read("/proc/self/exe")?;
    Ok(of_binary(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_test_binary_has_a_stable_id() {
        let id = current().unwrap();
        assert!(
            id.len() >= 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "{id}"
        );
        assert_eq!(current().unwrap(), id);
    }
}
