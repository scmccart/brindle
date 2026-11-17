//! The Win32 code inside an `HRESULT`.
//!
//! The `windows` crate reports a failed call as an `HRESULT`, and one of
//! facility 7 (`FACILITY_WIN32`, `0x8007xxxx`) wraps a Win32 error code,
//! which is what `io::Error` knows the kind of. The rule is plain
//! arithmetic, so it is tested on every platform; each caller keeps its own
//! fallback for an `HRESULT` of another facility.

/// The Win32 error code `hresult` wraps, when it is one of facility 7
/// (`0x8007xxxx`); `None` for any other `HRESULT`, and for a plain code.
pub fn win32_code(hresult: u32) -> Option<u32> {
    (hresult >> 16 == 0x8007).then_some(hresult & 0xffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_win32_hresult_gives_its_code() {
        assert_eq!(win32_code(0x8007_0005), Some(5));
        assert_eq!(win32_code(0x8007_00E7), Some(231));
    }

    #[test]
    fn another_facility_gives_none() {
        // FACILITY_ITF, and E_FAIL (FACILITY_NULL).
        assert_eq!(win32_code(0x8004_0005), None);
        assert_eq!(win32_code(0x8000_4005), None);
    }

    #[test]
    fn a_plain_code_gives_none() {
        assert_eq!(win32_code(0), None);
        assert_eq!(win32_code(5), None);
    }
}
