//! Links in terminal text: finding URLs in a line of characters, deciding
//! what a clicked link does ([`plan`]), and handing it to the desktop's
//! opener.
//!
//! Finding and planning are pure so they can be tested without a terminal
//! or a desktop; the grid side (joining wrapped rows, OSC 8 cells) lives
//! in [`super::link_at`]. A link is whatever a program printed, so only
//! the schemes Brindle detects are opened, a `file:` link only on this
//! machine, and on Windows a file is shown in Explorer, never run.

use std::fmt::Write as _;
use std::ops::Range;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use super::shell_integration::{PathStyle, is_absolute_path};

/// The schemes Brindle detects in plain text and opens, each with the
/// separator that must follow it in text. One list for both, so what is
/// underlined and what a click opens can't drift apart.
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
#[cfg(not(windows))]
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

/// Why Brindle doesn't open a link at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// It has no URI scheme, or holds a control character.
    NoScheme,
    /// Its scheme, as written, isn't one Brindle opens ([`SCHEMES`]).
    Scheme(String),
    /// A `file:` link that names another host than this machine: as its
    /// host, or on Windows as its path's (`\\host\share`).
    OtherHost(String),
    /// A `file:` link on Windows whose path isn't a plain one on a drive
    /// letter: no file's path, but whatever else the shell would make of
    /// it.
    NoDrive,
}

/// What a click on a link does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The opener gets the link, with each double quote and whitespace
    /// character percent-encoded.
    Open(String),
    /// A `file:` link on Windows: its path, a plain one on a drive letter
    /// (`is_absolute_path`), to show selected in Explorer (a directory is
    /// opened there). It is never handed to the shell to open, which would
    /// run a program.
    Reveal(String),
    Refuse(Refusal),
}

/// What a click on `uri` does: a detected URL or an OSC 8 hyperlink's URI,
/// which is whatever a program printed.
///
/// - Only a link whose scheme is one Brindle detects ([`SCHEMES`], compared
///   without case) is opened, so a click never reaches a handler or a
///   program that an arbitrary scheme names (`ms-settings:`, or the drive
///   of `C:\tool.exe`).
/// - A `file:` link is taken only when its host is empty, `localhost` or
///   `this_host`, this machine's name.
/// - With `windows`, a `file:` link's path is shown in Explorer, and
///   Explorer is given nothing but a plain path on a drive letter: `X:\`
///   and then no `:`, no `"` and no control character, which is the rule
///   a reported directory follows ([`is_absolute_path`]). The path is
///   checked as Windows will read it, percent-decoded and with every `/`
///   a `\`. A network path (`\\host\share`) is refused as another host,
///   and any other as no file's path: a device or rooted path (`\\?\…`,
///   `\??\…`, `\Device\…`), one with no drive or none from its root
///   (`C:x`), a stream of a file, `shell:…`. The shell reads more than
///   paths, and a quote would end the argument Explorer is given.
pub fn plan(uri: &str, this_host: &str, windows: bool) -> Plan {
    let Some((scheme, rest)) = uri.split_once(':').filter(|_| has_scheme(uri)) else {
        return Plan::Refuse(Refusal::NoScheme);
    };
    if !SCHEMES.iter().any(|(name, _)| scheme.eq_ignore_ascii_case(name)) {
        return Plan::Refuse(Refusal::Scheme(scheme.to_string()));
    }
    if !scheme.eq_ignore_ascii_case("file") {
        return Plan::Open(encoded(uri));
    }
    // `//host/path`; without the slashes there is no host.
    let (host, path) = match rest.strip_prefix("//") {
        Some(after) => after.find('/').map_or((after, ""), |slash| after.split_at(slash)),
        None => ("", rest),
    };
    let here = host.is_empty() || host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case(this_host);
    if !here {
        return Plan::Refuse(Refusal::OtherHost(host.to_string()));
    }
    if !windows {
        return Plan::Open(encoded(uri));
    }
    let path = windows_path(path);
    if let Some(host) = unc_host(&path) {
        return Plan::Refuse(Refusal::OtherHost(host.to_string()));
    }
    if !is_absolute_path(&path, PathStyle::Windows) {
        return Plan::Refuse(Refusal::NoDrive);
    }
    Plan::Reveal(path)
}

/// `link` with each double quote and each whitespace character
/// percent-encoded (as UTF-8), so it stays one argument however the
/// opener's command line is read. What is encoded already is left alone.
fn encoded(link: &str) -> String {
    let mut out = String::with_capacity(link.len());
    for c in link.chars() {
        if c == '"' || c.is_whitespace() {
            push_encoded(&mut out, c);
        } else {
            out.push(c);
        }
    }
    out
}

fn push_encoded(out: &mut String, c: char) {
    let mut utf8 = [0u8; 4];
    for byte in c.encode_utf8(&mut utf8).bytes() {
        // Writing to a `String` can't fail.
        let _ = write!(out, "%{byte:02X}");
    }
}

/// The path of a `file:` link as Windows reads it: without a query or
/// fragment, percent-decoded, `/C:…` without the slash before its drive,
/// and every `/` a `\`. Whether it is a path to show is [`plan`]'s to say,
/// of this form and no earlier one.
fn windows_path(path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let decoded = percent_decoded(path);
    let from_drive = match decoded.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => decoded.get(1..),
        _ => None,
    };
    from_drive.unwrap_or(&decoded).replace('/', "\\")
}

/// `text` with its `%XX` escapes decoded, as UTF-8. An escape that isn't
/// two hex digits stays as written, and so does all of `text` when the
/// decoded bytes aren't UTF-8.
fn percent_decoded(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let escaped = tail
            .get(..2)
            .filter(|hex| b == b'%' && hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                bytes.push(byte);
                rest = &tail[2..];
            }
            None => {
                bytes.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| text.to_string())
}

/// The host a Windows path names where a drive would be, when it is a
/// network path: the `host` of `\\host\share`. `None` for any other path,
/// a device path (`\\?\…`, `\\.\…`) included: what follows its two
/// separators is no host's name.
fn unc_host(path: &str) -> Option<&str> {
    let host = path.strip_prefix(r"\\")?.split('\\').find(|part| !part.is_empty())?;
    let named = host != "." && !host.chars().any(|c| matches!(c, '?' | '*' | ':' | '"' | '<' | '>' | '|') || c.is_control());
    named.then_some(host)
}

/// Why a link wasn't opened.
#[derive(Debug)]
pub enum OpenError {
    /// Refused before anything ran ([`plan`]).
    Refused(Refusal),
    /// The opener thread couldn't start.
    NoThread(std::io::Error),
    /// The opener program couldn't be run.
    NotStarted { program: String, error: std::io::Error },
    /// The opener ran and failed: its exit code, or none when a signal
    /// ended it.
    Failed { program: String, code: Option<i32> },
    /// `ShellExecuteW` returned `code`, which is 32 or less (Windows).
    #[cfg_attr(not(windows), allow(dead_code, reason = "only the Windows opener makes it; the text is tested everywhere"))]
    Shell { code: usize },
}

/// What a failed open says, as a notice on the config line of the window
/// where the link was clicked (`railway R8`: the action, then the cause).
pub fn failure_text(uri: &str, error: &OpenError) -> String {
    let why = match error {
        OpenError::Refused(Refusal::NoScheme) => "it has no URI scheme".to_string(),
        OpenError::Refused(Refusal::Scheme(scheme)) => format!("its scheme ({scheme}) isn't one Brindle opens"),
        OpenError::Refused(Refusal::OtherHost(host)) => format!("it names another host ({host})"),
        OpenError::Refused(Refusal::NoDrive) => "its path isn't on a drive letter".to_string(),
        OpenError::NoThread(error) => format!("the opener's thread couldn't start: {error}"),
        OpenError::NotStarted { program, error } => format!("couldn't run {program}: {error}"),
        OpenError::Failed { program, code: Some(code) } => format!("{program} exited with status {code}"),
        OpenError::Failed { program, code: None } => format!("{program} was ended by a signal"),
        OpenError::Shell { code } => format!("the shell couldn't open it (ShellExecute returned {code})"),
    };
    format!("couldn't open {uri}: {why}")
}

/// What opening a link answers: `Ok` once the opener has run and succeeded.
pub type Opened = futures::channel::oneshot::Receiver<Result<(), OpenError>>;

/// An answer already known.
fn answered(result: Result<(), OpenError>) -> Opened {
    let (tx, rx) = futures::channel::oneshot::channel();
    tx.send(result).ok();
    rx
}

/// Does what [`plan`] says of `uri`, without waiting: the receiver answers
/// when the opener has finished, or at once when the link is refused or
/// the opener can't run.
///
/// A link to open goes to `$BRINDLE_LINK_OPENER`, or else to `xdg-open`
/// (on Windows, the shell's handler for it). A Windows file is shown in
/// Explorer whatever the opener: it is never opened.
pub fn open(uri: &str) -> Opened {
    match plan_here(uri) {
        Plan::Refuse(refusal) => answered(Err(OpenError::Refused(refusal))),
        Plan::Reveal(path) => reveal(path),
        Plan::Open(link) => match std::env::var(OPENER_ENV).ok().filter(|p| !p.is_empty()) {
            Some(program) => open_with(program, link),
            None => open_default(link),
        },
    }
}

/// What [`open`] would do with `uri` on this machine: [`plan`] with this
/// machine's name and platform, the two facts the opener reads. Owners
/// that decide before opening (an agent's links ask first) plan with it.
pub fn plan_here(uri: &str) -> Plan {
    plan(uri, this_host(), cfg!(windows))
}

/// Whether a pointer press with `modifiers` acts on links: ctrl without
/// alt, whatever shift does (ctrl-alt-drag is a terminal's block
/// selection). Terminals and agent zones share the gesture.
pub fn wants_links(modifiers: &gpui::Modifiers) -> bool {
    modifiers.control && !modifiers.alt
}

/// This machine's name, which a `file:` link may give as its host. Read
/// once.
fn this_host() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(read_host)
}

/// The host name (`gethostname`); empty when it can't be read, and then
/// only a `file:` link with no host or `localhost` is this machine's.
#[cfg(unix)]
fn read_host() -> String {
    let mut name = [0u8; 256];
    // SAFETY: the buffer is writable for the length given.
    if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 {
        return String::new();
    }
    // A name that fills the buffer may come without its NUL.
    let len = name.iter().position(|&byte| byte == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..len]).into_owned()
}

/// The computer's name (`%COMPUTERNAME%`); empty when it isn't set, and
/// then only a `file:` link with no host or `localhost` is this machine's.
#[cfg(windows)]
fn read_host() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

#[cfg(not(any(unix, windows)))]
fn read_host() -> String {
    String::new()
}

#[cfg(not(windows))]
fn open_default(uri: String) -> Opened {
    open_with(DEFAULT_OPENER.into(), uri)
}

/// Only Windows shows a file in Explorer: [`plan`] gives a
/// [`Plan::Reveal`] nowhere else, and here its path goes to the opener as
/// a link does.
#[cfg(not(windows))]
fn reveal(path: String) -> Opened {
    open_default(path)
}

/// Has the shell open `uri` with its default handler (the browser, for a
/// web link), on its own thread: handlers can take a while to start.
#[cfg(windows)]
fn open_default(uri: String) -> Opened {
    on_opener_thread(move || {
        log::info!("opening {uri}");
        shell_execute(windows::core::w!("open"), &uri)
    })
}

/// Shows the file at `path` in Explorer, on the opener's thread: selected
/// in its folder, or the folder itself opened when `path` is a directory.
/// Neither runs a program, whatever `path` names: `explore` only takes a
/// folder, and `/select,` only selects.
#[cfg(windows)]
fn reveal(path: String) -> Opened {
    use std::os::windows::process::CommandExt as _;

    on_opener_thread(move || {
        log::info!("showing {path} in Explorer");
        if std::path::Path::new(&path).is_dir() {
            return shell_execute(windows::core::w!("explore"), &path);
        }
        // Explorer by its full path, so no other `explorer.exe` is found
        // first.
        let explorer = std::env::var_os("SystemRoot")
            .map(|root| std::path::PathBuf::from(root).join("explorer.exe"))
            .unwrap_or_else(|| "explorer.exe".into());
        let mut command = Command::new(&explorer);
        // One argument, quoted the way Explorer reads it. `plan` leaves no
        // double quote in the path, so nothing in it ends the quotes.
        command.raw_arg(format!("/select,\"{path}\""));
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // Explorer hands the request to the running shell and exits with
        // status 1 even when it worked, so only failing to start it counts.
        command
            .status()
            .map(|_| ())
            .map_err(|error| OpenError::NotStarted { program: explorer.display().to_string(), error })
    })
}

/// `ShellExecuteW` with `verb` on `target`, on the calling thread.
#[cfg(windows)]
fn shell_execute(verb: windows::core::PCWSTR, target: &str) -> Result<(), OpenError> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::HSTRING;

    // SAFETY: COM is initialized once for this short-lived thread, as
    // ShellExecute asks of its callers; the strings outlive the call.
    let result = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok().ok();
        ShellExecuteW(None, verb, &HSTRING::from(target), None, None, SW_SHOWNORMAL)
    };
    // Values up to 32 are errors.
    let code = result.0 as usize;
    if code <= 32 { Err(OpenError::Shell { code }) } else { Ok(()) }
}

/// Runs `work` on a thread of its own, so the UI never waits for the opener
/// and it never lingers as a zombie, and answers with its result; or with
/// [`OpenError::NoThread`] when the thread can't start.
fn on_opener_thread(work: impl FnOnce() -> Result<(), OpenError> + Send + 'static) -> Opened {
    use std::sync::Arc;

    let (tx, rx) = futures::channel::oneshot::channel();
    let slot = Arc::new(parking_lot::Mutex::new(Some(tx)));
    let for_thread = slot.clone();
    let spawned = std::thread::Builder::new().name("link-opener".into()).spawn(move || {
        let result = work();
        if let Some(tx) = for_thread.lock().take() {
            tx.send(result).ok();
        }
    });
    if let Err(error) = spawned
        && let Some(tx) = slot.lock().take()
    {
        tx.send(Err(OpenError::NoThread(error))).ok();
    }
    rx
}

/// Runs `program uri` on its own thread (spawning and reaping it there).
/// A uri with no scheme never reaches a program, whoever calls this: as
/// an argument it could read as an option.
fn open_with(program: String, uri: String) -> Opened {
    if !has_scheme(&uri) {
        return answered(Err(OpenError::Refused(Refusal::NoScheme)));
    }
    on_opener_thread(move || run_opener(&program, &uri))
}

/// Runs `program uri` and waits for it, with the uri as the one argument.
fn run_opener(program: &str, uri: &str) -> Result<(), OpenError> {
    log::info!("opening {uri} with {program}");
    let mut command = Command::new(program);
    command.arg(uri).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    crate::remote::hide_window(&mut command);
    let status = command
        .spawn()
        .and_then(|mut child| child.wait())
        .map_err(|error| OpenError::NotStarted { program: program.to_string(), error })?;
    if status.success() {
        Ok(())
    } else {
        Err(OpenError::Failed { program: program.to_string(), code: status.code() })
    }
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

    #[cfg(windows)]
    #[test]
    fn opener_gets_the_uri_as_one_argument_on_windows() {
        let dir = crate::test_support::TempDir::new("opener-test-windows");
        let out = dir.join("opened");
        let script = dir.join("record.cmd");
        // `%1` is the first argument as given (quotes and all), `%2` the second.
        std::fs::write(&script, format!("@echo %1^|%2> \"{}\"
", out.display())).unwrap();

        let uri = "https://example.com/a b?c=1&d='e'";
        run_opener(&script.display().to_string(), uri).unwrap();
        let recorded = std::fs::read_to_string(&out).unwrap();
        assert_eq!(recorded.trim_end(), format!("\"{uri}\"|"));

        let mut refused = open_with(script.display().to_string(), "-rf".into());
        assert!(matches!(refused.try_recv(), Ok(Some(Err(OpenError::Refused(Refusal::NoScheme))))));
    }

    #[cfg(unix)]
    #[test]
    fn opener_gets_the_uri_as_one_argument() {
        let dir = crate::test_support::TempDir::new("opener-test");
        let out = dir.join("opened");
        let script = dir.join("record");
        crate::test_support::write_executable(
            &script,
            &format!("#!/bin/sh\nprintf '%s|%s' \"$#\" \"$1\" > '{}'\n", out.display()),
        );

        let uri = "https://example.com/a b?c=1&d='e'";
        run_opener(&script.display().to_string(), uri).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("1|{uri}"));

        let mut refused = open_with(script.display().to_string(), "-rf".into());
        assert!(matches!(refused.try_recv(), Ok(Some(Err(OpenError::Refused(Refusal::NoScheme))))));
    }

    // ---- what a click does ----------------------------------------------------

    /// This machine's name in the plans below.
    const HERE: &str = "mybox";

    fn open_plan(link: &str) -> Plan {
        Plan::Open(link.to_string())
    }

    fn refused(refusal: Refusal) -> Plan {
        Plan::Refuse(refusal)
    }

    #[test]
    fn every_detected_scheme_opens_whatever_its_case() {
        for windows in [false, true] {
            for (scheme, separator) in SCHEMES.iter().filter(|(scheme, _)| *scheme != "file") {
                let link = format!("{scheme}{separator}example.com/a?b=1&c=2");
                assert_eq!(plan(&link, HERE, windows), open_plan(&link), "{link}");
                let upper = format!("{}{separator}EXAMPLE.COM", scheme.to_uppercase());
                assert_eq!(plan(&upper, HERE, windows), open_plan(&upper), "{upper}");
            }
            // The name decides, not what follows it: OSC 8 URIs come as
            // their program wrote them.
            assert_eq!(plan("MailTo:dev@example.com", HERE, windows), open_plan("MailTo:dev@example.com"));
            assert_eq!(plan("https:example.com", HERE, windows), open_plan("https:example.com"));
        }
    }

    #[test]
    fn a_scheme_brindle_does_not_detect_is_refused_by_name() {
        let cases = [
            ("ms-settings:privacy", "ms-settings"),
            ("search-ms:query=x", "search-ms"),
            ("vscode://file/x", "vscode"),
            ("javascript:alert(1)", "javascript"),
            ("man:ls(1)", "man"),
            ("git+ssh://h/r", "git+ssh"),
            // A Windows path reads as a link whose scheme is its drive.
            (r"C:\Windows\System32\calc.exe", "C"),
            ("c:/tools/x.exe", "c"),
            // Named as written.
            ("MS-Settings:privacy", "MS-Settings"),
        ];
        for windows in [false, true] {
            for (link, scheme) in cases {
                assert_eq!(plan(link, HERE, windows), refused(Refusal::Scheme(scheme.to_string())), "{link}");
            }
        }
    }

    #[test]
    fn a_link_with_no_scheme_or_with_a_control_character_is_refused() {
        for windows in [false, true] {
            for link in ["example.com/x", "-rf", "/etc/hosts", ":x", "1http://x", "", "https://example.com/\nrm", "file:///a\x1b[31m"] {
                assert_eq!(plan(link, HERE, windows), refused(Refusal::NoScheme), "{link:?}");
            }
        }
    }

    #[test]
    fn a_file_link_to_another_host_is_refused_naming_the_host() {
        let cases = [
            ("file://host/share/x.exe", "host"),
            ("FILE://Host/share/x.exe", "Host"),
            ("file://host", "host"),
            ("file://host.example.com/etc/hosts", "host.example.com"),
            // Not this machine's name, only like it.
            ("file://mybox.example.com/etc/hosts", "mybox.example.com"),
            ("file://mybox:80/etc/hosts", "mybox:80"),
            ("file://user@mybox/etc/hosts", "user@mybox"),
            // A drive where the host goes.
            ("file://C:/Users/me/x.txt", "C:"),
        ];
        for windows in [false, true] {
            for (link, host) in cases {
                assert_eq!(plan(link, HERE, windows), refused(Refusal::OtherHost(host.to_string())), "{link}");
            }
        }
        // With no name of its own, this machine is only `localhost`.
        assert_eq!(plan("file://mybox/etc/hosts", "", false), refused(Refusal::OtherHost("mybox".into())));
        assert_eq!(plan("file://localhost/etc/hosts", "", false), open_plan("file://localhost/etc/hosts"));
    }

    #[test]
    fn a_file_link_with_no_host_localhost_or_this_machines_name_opens() {
        for link in [
            "file:///etc/hosts",
            "file:/etc/hosts",
            "file://localhost/etc/hosts",
            "file://LOCALHOST/etc/hosts",
            "file://mybox/etc/hosts",
            "file://MyBox/etc/hosts",
            "FILE:///etc/hosts",
            // Elsewhere than Windows, a path is a path: nothing here names
            // a host.
            "file:////host/share/x",
            "file:///C:/Users/me/tool.exe",
        ] {
            assert_eq!(plan(link, HERE, false), open_plan(link), "{link}");
        }
    }

    /// On Windows the shell would run a program a `file:` link names, so
    /// the link is shown in Explorer: the plan carries its path.
    #[test]
    fn a_windows_file_link_is_revealed_by_its_path() {
        let cases = [
            ("file:///C:/Users/me/tool.exe", r"C:\Users\me\tool.exe"),
            ("file://localhost/C:/Users/me/tool.exe", r"C:\Users\me\tool.exe"),
            ("file://MYBOX/c:/Users/me/tool.exe", r"c:\Users\me\tool.exe"),
            ("FILE:///C:/Users/me/tool.exe", r"C:\Users\me\tool.exe"),
            ("file:///C:/My%20Files/caf%C3%A9.txt", r"C:\My Files\café.txt"),
            ("file:///C:/My Files/a.txt", r"C:\My Files\a.txt"),
            (r"file:///C:\Users\me\tool.exe", r"C:\Users\me\tool.exe"),
            ("file:C:/Users/me/tool.exe", r"C:\Users\me\tool.exe"),
            ("file:///C:/", r"C:\"),
            // A query or a fragment is no part of the path.
            ("file:///C:/docs/a.html?x=1#top", r"C:\docs\a.html"),
            // On a drive, whatever follows it.
            ("file:///C:/..//host/share", r"C:\..\\host\share"),
            // An escape that isn't one stays as written.
            ("file:///C:/100%/a%2", r"C:\100%\a%2"),
            ("file:///C:/a%+fb", r"C:\a%+fb"),
            ("file:///C:/%FF%FE", r"C:\%FF%FE"),
        ];
        for (link, path) in cases {
            assert_eq!(plan(link, HERE, true), Plan::Reveal(path.to_string()), "{link}");
        }
    }

    /// What Explorer is given goes inside one pair of quotes on its command
    /// line. A path with a double quote would end them and add arguments,
    /// and no file's name holds a quote, a control character or a second
    /// colon (a stream of a file): each is refused, however it is written.
    #[test]
    fn a_windows_file_link_with_a_quote_a_control_or_a_second_colon_is_refused() {
        for link in [
            // A path with `"`.
            r#"file:///C:/a" "C:/evil.exe"#,
            r#"file:///C:/a".exe"#,
            "file:///C:/a%22%20b.exe",
            "file:///C:/a%22%20%22C:/evil.exe",
            // A control character can only come percent-encoded.
            "file:///C:/a%0Ab",
            "file:///C:/a%00b",
            "file:///C:/a%1Bb",
            "file:///C:/a%7Fb",
            "file:///C:/a%C2%9Bb",
            // `C:\a\b.txt:stream`, and other second colons.
            "file:///C:/a/b.txt:stream",
            r"file:///C:\a\b.txt:stream",
            "file:///C:/a/b.txt::$DATA",
            "file:///C:/a/b.txt%3Astream",
            "file:///C:/D:/x.exe",
            "file:///C:/ms-settings:privacy",
        ] {
            assert_eq!(plan(link, HERE, true), refused(Refusal::NoDrive), "{link}");
        }
    }

    /// A Windows path that starts with two separators and then a host's
    /// name is a network path: such a `file:` link is refused as another
    /// host, however its separators are written.
    #[test]
    fn a_windows_file_link_to_a_network_path_is_refused_naming_its_host() {
        let cases = [
            ("file:////host/share/x.exe", "host"),
            ("file://///host/share/x.exe", "host"),
            ("file://localhost//host/share/x.exe", "host"),
            ("file://mybox//host/share", "host"),
            (r"file:///\\host\share\x.exe", "host"),
            (r"file:\\host\share\x.exe", "host"),
            // `%5C%5Chost%5Cs`, and the forward slashes encoded too.
            ("file:///%5C%5Chost%5Cs", "host"),
            ("file:%5C%5Chost%5Cs", "host"),
            ("file:///%2F%2Fhost/share", "host"),
            ("file:////host@80/dav/x", "host@80"),
            ("file:////host@SSL@443/DavWWWRoot/x", "host@SSL@443"),
            ("file:////wsl$/Ubuntu/home/me", "wsl$"),
            // With the link's own slash before it, a rooted path is a
            // network path to Windows: `\\Device\Mup\h\s`.
            (r"file:///\Device\Mup\h\s", "Device"),
        ];
        for (link, host) in cases {
            assert_eq!(plan(link, HERE, true), refused(Refusal::OtherHost(host.to_string())), "{link}");
        }
    }

    /// Every other way to leave the drives is refused too: a device or
    /// extended path, and a path from the root of the object namespace,
    /// which reach a network path or a device as well as `\\host` does.
    #[test]
    fn a_windows_file_link_to_a_device_or_rooted_path_is_refused() {
        for link in [
            // `\??\UNC\h\s`, written out and percent-encoded.
            r"file:\??\UNC\h\s",
            r"file:///\??\UNC\h\s",
            "file:%5C%3F%3F%5CUNC%5Ch%5Cs",
            "file:///%5C%3F%3F%5CUNC%5Ch%5Cs",
            "file:///%3F%3F/UNC/h/s",
            "file:///%5C%3F%3F%5CC:%5Cx.exe",
            // `\Device\Mup\h\s` and `\GLOBALROOT\…`.
            r"file:\Device\Mup\h\s",
            "file:///Device/Mup/h/s",
            "file:%5CDevice%5CMup%5Ch%5Cs",
            r"file:\GLOBALROOT\Device\Mup\h\s",
            "file:///GLOBALROOT/Device/Mup/h/s",
            // `\\?\…` and `\\.\…`.
            "file:////%3F/UNC/host/share/x",
            "file:////./unc/host/share/x",
            "file:////%3F/C:/Users/me/x.exe",
            "file:////./C:/Users/me/x.exe",
            "file:////./pipe/x",
            "file:////%3F/GLOBALROOT/Device/Mup/host/share",
            // A `?` written as it is starts the query: what is left of the
            // path is two separators and no host.
            "file:////?/UNC/host/share/x",
            r"file:///\\?\C:\Users\me\x.exe",
            "file:////",
        ] {
            assert_eq!(plan(link, HERE, true), refused(Refusal::NoDrive), "{link}");
        }
    }

    /// Explorer is given a path on a drive letter and nothing else: it
    /// reads more than paths (`shell:` folders, namespace objects, links),
    /// and a `file:` link can carry any of them where its path goes.
    #[test]
    fn a_windows_file_link_whose_path_is_not_on_a_drive_is_refused() {
        for link in [
            // A path with no drive, from the root or relative.
            "file:///etc/hosts",
            "file:x.txt",
            "file:///x.txt",
            "file:docs/x.txt",
            r"file:docs\x.txt",
            "file:../x.exe",
            "file:.",
            // A drive with no root after it: `C:x.exe`.
            "file:C:x.exe",
            "file:///C:x.exe",
            "file:C:tool.exe",
            "file:///C:",
            "file:///1:/x",
            "file:ms-settings:privacy",
            "file:///ms-settings:privacy",
            "file:https://example.com/",
            r"file:shell:AppsFolder\Calc!App",
            "file:::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
            "file:///",
            "file://localhost",
            "file://mybox",
            "file:",
        ] {
            assert_eq!(plan(link, HERE, true), refused(Refusal::NoDrive), "{link}");
        }
        // Elsewhere a path needs no drive.
        assert_eq!(plan("file:///etc/hosts", HERE, false), open_plan("file:///etc/hosts"));
    }

    /// A quote or a space in a link would let it end the argument its
    /// handler's command line gives it, so both are percent-encoded.
    #[test]
    fn quotes_and_whitespace_in_a_link_are_percent_encoded() {
        let cases = [
            (r#"https://example.com/" --flag"#, "https://example.com/%22%20--flag"),
            ("https://example.com/a b?c=1&d='e'", "https://example.com/a%20b?c=1&d='e'"),
            // What is encoded already is left alone, and so is the rest.
            ("https://example.com/a%20b%22c", "https://example.com/a%20b%22c"),
            ("https://example.com/naïve/100%", "https://example.com/naïve/100%"),
            // Any whitespace, not only the ASCII space.
            ("https://example.com/a\u{a0}b\u{3000}c", "https://example.com/a%C2%A0b%E3%80%80c"),
            ("mailto:a b@example.com", "mailto:a%20b@example.com"),
        ];
        for windows in [false, true] {
            for (link, opened) in cases {
                assert_eq!(plan(link, HERE, windows), open_plan(opened), "{link}");
            }
        }
        // A `file:` link that opens (elsewhere than Windows) is encoded too.
        assert_eq!(plan("file:///home/me/My Files/a.txt", HERE, false), open_plan("file:///home/me/My%20Files/a.txt"));
    }

    /// What the tab says of each refusal: the link, then why.
    #[test]
    fn each_refusal_names_the_link_and_its_cause() {
        let says = |uri: &str, refusal: Refusal| failure_text(uri, &OpenError::Refused(refusal));
        assert_eq!(says("-rf", Refusal::NoScheme), "couldn't open -rf: it has no URI scheme");
        assert_eq!(
            says("ms-settings:privacy", Refusal::Scheme("ms-settings".into())),
            "couldn't open ms-settings:privacy: its scheme (ms-settings) isn't one Brindle opens"
        );
        assert_eq!(
            says("file://host/share/x.exe", Refusal::OtherHost("host".into())),
            "couldn't open file://host/share/x.exe: it names another host (host)"
        );
        assert_eq!(
            says("file:shell:Downloads", Refusal::NoDrive),
            "couldn't open file:shell:Downloads: its path isn't on a drive letter"
        );
    }

    /// The refusals a click gets, worded as the tab shows them.
    #[test]
    fn a_refused_link_is_answered_at_once_with_its_refusal() {
        let refused = |uri: &str| match open(uri).try_recv() {
            Ok(Some(Err(error))) => failure_text(uri, &error),
            other => panic!("{uri} wasn't refused at once: {other:?}"),
        };
        assert_eq!(refused("example.com/x"), "couldn't open example.com/x: it has no URI scheme");
        assert_eq!(
            refused("ms-settings:privacy"),
            "couldn't open ms-settings:privacy: its scheme (ms-settings) isn't one Brindle opens"
        );
        assert_eq!(
            refused(r"C:\Windows\System32\calc.exe"),
            r"couldn't open C:\Windows\System32\calc.exe: its scheme (C) isn't one Brindle opens"
        );
        assert_eq!(
            refused("file://brindle-test-another-host.invalid/share/x.exe"),
            "couldn't open file://brindle-test-another-host.invalid/share/x.exe: it names another host \
             (brindle-test-another-host.invalid)"
        );
    }

    #[test]
    fn a_missing_program_names_it_and_the_error() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory");
        let text = failure_text("https://example.com", &OpenError::NotStarted { program: "xdg-open".into(), error });
        assert_eq!(text, "couldn't open https://example.com: couldn't run xdg-open: no such file or directory");
    }

    #[test]
    fn an_exit_status_is_named() {
        let failed = |code| OpenError::Failed { program: "xdg-open".into(), code };
        assert_eq!(
            failure_text("https://example.com", &failed(Some(4))),
            "couldn't open https://example.com: xdg-open exited with status 4"
        );
        assert_eq!(
            failure_text("https://example.com", &failed(None)),
            "couldn't open https://example.com: xdg-open was ended by a signal"
        );
        assert_eq!(
            failure_text("https://example.com", &OpenError::Shell { code: 31 }),
            "couldn't open https://example.com: the shell couldn't open it (ShellExecute returned 31)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_opener_answers_with_its_status() {
        let dir = crate::test_support::TempDir::new("opener-fails");
        let script = dir.join("fail");
        crate::test_support::write_executable(&script, "#!/bin/sh\nexit 4\n");
        let error = run_opener(&script.display().to_string(), "https://example.com").unwrap_err();
        assert!(matches!(error, OpenError::Failed { code: Some(4), .. }), "{error:?}");
        let error = run_opener(&dir.join("missing").display().to_string(), "https://example.com").unwrap_err();
        assert!(matches!(error, OpenError::NotStarted { .. }), "{error:?}");
    }

    /// Links take ctrl without alt, whatever shift does: ctrl-alt is a
    /// block selection, and alt alone or no modifier leaves links alone.
    #[test]
    fn links_take_ctrl_without_alt() {
        let held =
            |control: bool, alt: bool, shift: bool| gpui::Modifiers { control, alt, shift, ..Default::default() };
        for shift in [false, true] {
            assert!(!wants_links(&held(false, false, shift)), "none, shift {shift}");
            assert!(wants_links(&held(true, false, shift)), "ctrl, shift {shift}");
            assert!(!wants_links(&held(false, true, shift)), "alt, shift {shift}");
            assert!(!wants_links(&held(true, true, shift)), "ctrl-alt, shift {shift}");
        }
    }

    /// `plan_here` plans for this machine: the platform's rules and its own
    /// name, so a link `plan` treats the same everywhere comes out the same,
    /// and a file link is this platform's kind.
    #[test]
    fn plan_here_plans_for_this_machine() {
        let windows = cfg!(windows);
        for link in ["https://example.com/a", "mailto:dev@example.com", "javascript:void", "file://otherhost/x"] {
            assert_eq!(plan_here(link), plan(link, this_host(), windows), "{link}");
        }
        assert_eq!(plan_here("https://example.com/a"), Plan::Open("https://example.com/a".into()));
        assert_eq!(plan_here("javascript:void"), Plan::Refuse(Refusal::Scheme("javascript".into())));
        let local = plan_here("file:///etc/hosts");
        match windows {
            true => assert_eq!(local, Plan::Refuse(Refusal::NoDrive)),
            false => assert_eq!(local, Plan::Open("file:///etc/hosts".into())),
        }
    }
}
