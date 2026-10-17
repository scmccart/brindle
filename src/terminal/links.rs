//! Links in terminal text: finding URLs in a line of characters, checking
//! OSC 8 URIs, and handing a link to the desktop's opener.
//!
//! Finding is pure so it can be tested without a terminal; the grid side
//! (joining wrapped rows, OSC 8 cells) lives in [`super::link_at`].

use std::ops::Range;
use std::process::{Command, Stdio};

/// Schemes detected in plain text, with the separator that must follow.
const SCHEMES: &[(&str, &str)] = &[
    ("https", "://"),
    ("http", "://"),
    ("ftp", "://"),
    ("file", "://"),
    ("ssh", "://"),
    ("git", "://"),
    ("gemini", "://"),
    ("gopher", "://"),
    ("ipfs", "://"),
    ("ipns", "://"),
    ("mailto", ":"),
    ("news", ":"),
    ("magnet", ":"),
];

/// Environment variable naming the program that opens links.
pub const OPENER_ENV: &str = "BRINDLE_LINK_OPENER";
const DEFAULT_OPENER: &str = "xdg-open";

/// Characters a URL can't contain. Box drawing and block elements end a
/// URL too, since TUIs draw them right next to text.
fn is_url_char(c: char) -> bool {
    !(c.is_whitespace()
        || c.is_control()
        || matches!(c, '<' | '>' | '"' | '`' | '{' | '}' | '|' | '\\' | '^')
        || ('\u{2500}'..='\u{259f}').contains(&c))
}

/// If a known scheme starts at `i` (at a word start within the run that
/// begins at `run_start`), the index just past its separator.
fn scheme_at(text: &[char], i: usize, run_start: usize) -> Option<usize> {
    if i > run_start {
        let before = text[i - 1];
        if before.is_alphanumeric() || matches!(before, '+' | '-' | '.') {
            return None;
        }
    }
    SCHEMES.iter().find_map(|(name, sep)| {
        let end = i + name.len() + sep.len();
        if end > text.len() {
            return None;
        }
        let name_ok = text[i..i + name.len()].iter().zip(name.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b));
        let sep_ok = text[i + name.len()..end].iter().copied().eq(sep.chars());
        (name_ok && sep_ok).then_some(end)
    })
}

/// The URL in `text` that covers index `at`, as a char range.
pub fn find_url(text: &[char], at: usize) -> Option<Range<usize>> {
    if at >= text.len() || !is_url_char(text[at]) {
        return None;
    }
    let mut run_start = at;
    while run_start > 0 && is_url_char(text[run_start - 1]) {
        run_start -= 1;
    }
    let mut run_end = at + 1;
    while run_end < text.len() && is_url_char(text[run_end]) {
        run_end += 1;
    }

    // The last scheme that starts at or before `at`.
    let (start, body) = (run_start..=at).rev().find_map(|i| Some((i, scheme_at(text, i, run_start)?)))?;

    let mut end = run_end;
    while end > body {
        let url = &text[start..end];
        let unmatched = |open: char, close: char| {
            url.iter().filter(|&&c| c == close).count() > url.iter().filter(|&&c| c == open).count()
        };
        let drop = match text[end - 1] {
            '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*' => true,
            ')' => unmatched('(', ')'),
            ']' => unmatched('[', ']'),
            _ => false,
        };
        if !drop {
            break;
        }
        end -= 1;
    }
    (end > body && at < end).then_some(start..end)
}

/// Whether `uri` starts with a URI scheme (`ALPHA *( ALPHA / DIGIT / "+" /
/// "-" / "." ) ":"`) and has no control characters, so it is safe to hand
/// to the opener as one argument (it can't look like an option).
pub fn has_scheme(uri: &str) -> bool {
    let Some((scheme, _)) = uri.split_once(':') else { return false };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !uri.chars().any(char::is_control)
}

/// Opens `uri` with `$BRINDLE_LINK_OPENER`, or `xdg-open`, without waiting.
pub fn open(uri: &str) {
    let program = std::env::var(OPENER_ENV).ok().filter(|p| !p.is_empty()).unwrap_or_else(|| DEFAULT_OPENER.into());
    open_with(program, uri.to_string());
}

/// Runs `program uri` on its own thread (spawning and reaping it there), so
/// the UI never waits for the opener and it never lingers as a zombie.
fn open_with(program: String, uri: String) -> Option<std::thread::JoinHandle<()>> {
    if !has_scheme(&uri) {
        log::warn!("not opening {uri:?}: it has no URI scheme");
        return None;
    }
    let spawned = std::thread::Builder::new().name("link-opener".into()).spawn(move || {
        log::info!("opening {uri} with {program}");
        let mut command = Command::new(&program);
        command.arg(&uri).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let child = command.spawn();
        match child.and_then(|mut child| child.wait()) {
            Ok(status) if !status.success() => log::warn!("{program} {uri}: {status}"),
            Ok(_) => {}
            Err(err) => log::warn!("could not run {program} to open {uri}: {err}"),
        }
    });
    spawned.map_err(|err| log::warn!("could not start the link opener thread: {err}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The URL found when pointing at the first occurrence of `needle`.
    fn url_at(text: &str, needle: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        let byte = text.find(needle).expect("needle in text");
        let at = text[..byte].chars().count();
        find_url(&chars, at).map(|r| chars[r].iter().collect())
    }

    #[test]
    fn url_in_a_sentence() {
        let text = "see https://example.com/docs for details";
        assert_eq!(url_at(text, "example").as_deref(), Some("https://example.com/docs"));
        assert_eq!(url_at(text, "https").as_deref(), Some("https://example.com/docs"));
        assert_eq!(url_at(text, "docs").as_deref(), Some("https://example.com/docs"));
        assert_eq!(url_at(text, "see"), None);
        assert_eq!(url_at(text, "details"), None);
    }

    #[test]
    fn trailing_punctuation_is_dropped() {
        assert_eq!(url_at("Visit https://example.com/a.", "ex").as_deref(), Some("https://example.com/a"));
        assert_eq!(url_at("at https://example.com/a, then", "ex").as_deref(), Some("https://example.com/a"));
        assert_eq!(url_at("really https://example.com/?!", "ex").as_deref(), Some("https://example.com/"));
        assert_eq!(url_at("go: https://example.com/a:", "ex").as_deref(), Some("https://example.com/a"));
        // Inner punctuation stays.
        assert_eq!(url_at("https://example.com/a.b?c=1;d", "ex").as_deref(), Some("https://example.com/a.b?c=1;d"));
    }

    #[test]
    fn brackets() {
        assert_eq!(url_at("(see https://example.com/a)", "ex").as_deref(), Some("https://example.com/a"));
        assert_eq!(
            url_at("https://en.wikipedia.org/wiki/Rust_(programming_language)", "wiki").as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(programming_language)")
        );
        assert_eq!(
            url_at("(https://en.wikipedia.org/wiki/Rust_(lang))", "wiki").as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(lang)")
        );
        assert_eq!(url_at("[docs](https://example.com/x)", "ex").as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at("[https://example.com/x]", "ex").as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at("<https://example.com/x>", "ex").as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at("{https://example.com/x}", "ex").as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at("(https://example.com/x).", "ex").as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn quotes() {
        assert_eq!(url_at("\"https://example.com/x\"", "ex").as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at("'https://example.com/x'", "ex").as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at("`https://example.com/x`", "ex").as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn schemes() {
        assert_eq!(url_at("mail mailto:dev@example.com now", "dev").as_deref(), Some("mailto:dev@example.com"));
        assert_eq!(url_at("file:///etc/hosts", "etc").as_deref(), Some("file:///etc/hosts"));
        assert_eq!(url_at("HTTP://EXAMPLE.COM", "EX").as_deref(), Some("HTTP://EXAMPLE.COM"));
        assert_eq!(url_at("ssh://git@host/repo", "host").as_deref(), Some("ssh://git@host/repo"));
        assert_eq!(url_at("foo://bar", "bar"), None);
        assert_eq!(url_at("https://", "https"), None);
        assert_eq!(url_at("https://.", "https"), None);
        assert_eq!(url_at("mailto:", "mailto"), None);
        assert_eq!(url_at("www.example.com", "example"), None);
        assert_eq!(url_at("/usr/share/doc", "share"), None);
    }

    #[test]
    fn scheme_must_start_a_word() {
        assert_eq!(url_at("xhttps://example.com", "example"), None);
        assert_eq!(url_at("git+ssh://example.com", "example"), None);
        assert_eq!(url_at("url=https://example.com", "example").as_deref(), Some("https://example.com"));
        assert_eq!(url_at("a:https://example.com", "example").as_deref(), Some("https://example.com"));
    }

    #[test]
    fn ends_at_box_drawing_and_unicode_is_kept() {
        assert_eq!(url_at("│https://example.com│", "ex").as_deref(), Some("https://example.com"));
        assert_eq!(url_at("https://example.com/naïve café", "ex").as_deref(), Some("https://example.com/naïve"));
    }

    #[test]
    fn last_scheme_before_the_pointer_wins() {
        let text = "https://a.example/?u=https://b.example";
        assert_eq!(url_at(text, "b.example").as_deref(), Some("https://b.example"));
        assert_eq!(url_at(text, "a.example").as_deref(), Some(text));
    }

    #[test]
    fn pointer_past_the_trimmed_end() {
        // Pointing at the dropped full stop finds nothing.
        let chars: Vec<char> = "https://example.com.".chars().collect();
        assert_eq!(find_url(&chars, chars.len() - 1), None);
        assert_eq!(find_url(&chars, chars.len()), None);
    }

    #[test]
    fn uri_schemes() {
        assert!(has_scheme("https://example.com"));
        assert!(has_scheme("vscode://file/x"));
        assert!(has_scheme("man:ls(1)"));
        assert!(has_scheme("git+ssh://h/r"));
        assert!(!has_scheme("-n"));
        assert!(!has_scheme("/etc/hosts"));
        assert!(!has_scheme("1http://x"));
        assert!(!has_scheme(":x"));
        assert!(!has_scheme("https://example.com/\nrm"));
    }

    #[cfg(unix)]
    #[test]
    fn opener_gets_the_uri_as_one_argument() {
        let dir = std::env::temp_dir().join(format!("brindle-opener-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("opened");
        let script = dir.join("record");
        std::fs::write(&script, format!("#!/bin/sh\nprintf '%s|%s' \"$#\" \"$1\" > '{}'\n", out.display())).unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let uri = "https://example.com/a b?c=1&d='e'";
        open_with(script.display().to_string(), uri.into()).unwrap().join().unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("1|{uri}"));

        assert!(open_with(script.display().to_string(), "-rf".into()).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
