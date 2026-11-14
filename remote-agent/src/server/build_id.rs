//! The running binary's build id ([`brindle_remote_protocol::build_id`]).
//! It is computed once at startup from the running binary itself, which an
//! install may since have replaced on disk.

use std::io;

use brindle_remote_protocol::build_id::of_binary;

/// The build id of the running binary, as lower-case hex.
pub fn current() -> io::Result<String> {
    let bytes = super::platform::sys::read_own_binary()?;
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
