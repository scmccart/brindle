//! A path on the agent's host as a `file://` URI (OpenSpec change
//! `agents-prompt-context`, design decision 5), for a prompt's links and
//! the kept texts' resources.
//!
//! Every byte outside RFC 3986's unreserved set (`A-Z a-z 0-9 - . _ ~`)
//! and `/` is percent-encoded, so a space, `%`, `#`, `?` and non-ASCII
//! text all survive. Which kind of path it is comes as a value (`windows`),
//! never from the platform this runs on, so both are tested everywhere
//! (`fcis R3`, `R7`): a Windows path is read as text, never as a `Path`.
//!
//! A whole pure module (`fcis` R2, R3; `PURE_MODULES`).

/// `path` as a `file://` URI. `windows` reads it as a Windows path:
/// backslashes are separators, `C:\…` becomes `file:///C:/…` and a UNC
/// path `\\server\share\…` becomes `file://server/share/…`. A path that
/// isn't absolute (which a resolved link never is) is made so with a
/// leading `/` rather than refused.
pub fn file(path: &str, windows: bool) -> String {
    if !windows {
        return format!("file://{}{}", lead(path), encode(path));
    }
    let path = path.replace('\\', "/");
    if let Some(unc) = path.strip_prefix("//") {
        let (host, rest) = unc.split_once('/').unwrap_or((unc, ""));
        return format!("file://{}/{}", encode(host), encode(rest));
    }
    match drive(&path) {
        Some((letter, rest)) => format!("file:///{letter}:{}{}", lead(rest), encode(rest)),
        None => format!("file://{}{}", lead(&path), encode(&path)),
    }
}

/// A `/` to put before `rest` when it doesn't start with one.
fn lead(rest: &str) -> &'static str {
    if rest.starts_with('/') { "" } else { "/" }
}

/// `C:` and what follows it, for a path that starts with a drive letter.
fn drive(path: &str) -> Option<(char, &str)> {
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':')).then(|| (letter, &path[2..]))
}

/// Every byte but the unreserved ones and `/` as `%XX`.
fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(char::from(byte));
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unix_path_is_a_file_uri_with_its_odd_bytes_encoded() {
        assert_eq!(file("/home/u/a.rs", false), "file:///home/u/a.rs");
        assert_eq!(file("/home/u/a b.rs", false), "file:///home/u/a%20b.rs");
        assert_eq!(file("/t/100%#1?.log", false), "file:///t/100%25%231%3F.log");
        assert_eq!(file("/t/ñ€.txt", false), "file:///t/%C3%B1%E2%82%AC.txt");
        assert_eq!(file("/t/a:b\\c", false), "file:///t/a%3Ab%5Cc");
        // Already odd: relative, empty, doubled separators.
        assert_eq!(file("a.rs", false), "file:///a.rs");
        assert_eq!(file("", false), "file:///");
        assert_eq!(file("//srv//x", false), "file:////srv//x");
    }

    #[test]
    fn a_windows_drive_path_keeps_its_drive_and_turns_its_separators() {
        assert_eq!(file("C:\\Users\\u\\a.rs", true), "file:///C:/Users/u/a.rs");
        assert_eq!(file("c:/Users/u/a b.rs", true), "file:///c:/Users/u/a%20b.rs");
        assert_eq!(file("D:\\t\\100%#1?.log", true), "file:///D:/t/100%25%231%3F.log");
        assert_eq!(file("C:\\ñ.txt", true), "file:///C:/%C3%B1.txt");
        // A drive with no separator after it, and a bare drive.
        assert_eq!(file("C:a.rs", true), "file:///C:/a.rs");
        assert_eq!(file("C:", true), "file:///C:/");
    }

    #[test]
    fn a_unc_path_names_its_server_as_the_host() {
        assert_eq!(file("\\\\srv\\share\\a b.rs", true), "file://srv/share/a%20b.rs");
        assert_eq!(file("//srv/share", true), "file://srv/share");
        assert_eq!(file("\\\\srv", true), "file://srv/");
    }

    #[test]
    fn a_windows_path_that_isnt_absolute_is_made_so() {
        assert_eq!(file("src\\a.rs", true), "file:///src/a.rs");
        assert_eq!(file("\\src\\a.rs", true), "file:///src/a.rs");
        // A path that only looks like a drive on Unix stays a path there.
        assert_eq!(file("C:\\a", false), "file:///C%3A%5Ca");
    }
}
