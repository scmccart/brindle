//! Architecture checks for the rules in `docs/architecture/`: the crate
//! boundaries, the few source rules a text search can hold, and `ports R4`,
//! which keeps production code's platform `cfg`s in their seams (the
//! `platform_cfgs` scanner and the tree checks after it). Part of plain
//! `cargo test`; it reads manifests and sources and runs nothing.
//!
//! Each test names the doc rules it enforces. When one fails, change the
//! code, or change the rule in its doc and here in the same commit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// The workspace's crates by directory, with their package names. A new
/// crate needs an entry here and its rules below.
const CRATES: [(&str, &str); 5] = [
    (".", "brindle"),
    ("remote-agent", "brindle-remote-agent"),
    ("remote-protocol", "brindle-remote-protocol"),
    ("term-core", "brindle-term-core"),
    ("ui", "brindle-ui"),
];

/// Every dependency a crate below the app may name, of any kind (normal,
/// build, dev) and for any target. The app (`brindle`) has no closed list.
const ALLOWED: [(&str, &[&str]); 4] = [
    ("ui", &["gpui"]),
    (
        "term-core",
        &["alacritty_terminal", "base64", "parking_lot"],
    ),
    ("remote-protocol", &["serde", "serde_json", "windows"]),
    (
        "remote-agent",
        &[
            "brindle-remote-protocol",
            "brindle-term-core",
            "embed-resource",
            "libc",
            "windows",
        ],
    ),
];

/// I/O and ambient inputs that pure code must not reach for.
const IO: [&str; 8] = [
    "std::fs",
    "std::net",
    "std::process",
    "std::env",
    "fs::",
    "net::",
    "process::",
    "env::",
];
const CLOCK_AND_THREADS: [&str; 4] = ["Instant::now", "SystemTime::now", "std::thread", "thread::"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest(dir: &str) -> toml::Table {
    let path = root().join(dir).join("Cargo.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The package names in one dependency table (`package = "…"` renames
/// included).
fn names(table: &toml::Table, section: &str) -> Vec<String> {
    let Some(deps) = table.get(section).and_then(|deps| deps.as_table()) else {
        return Vec::new();
    };
    deps.iter()
        .map(|(key, value)| {
            value
                .get("package")
                .and_then(|package| package.as_str())
                .unwrap_or(key)
                .to_string()
        })
        .collect()
}

/// A crate's dependencies of the given kinds, for every target.
fn dependencies(dir: &str, sections: &[&str]) -> BTreeSet<String> {
    let manifest = manifest(dir);
    let mut tables = vec![&manifest];
    if let Some(targets) = manifest.get("target").and_then(|target| target.as_table()) {
        tables.extend(targets.values().filter_map(|target| target.as_table()));
    }
    tables
        .iter()
        .flat_map(|table| sections.iter().flat_map(|section| names(table, section)))
        .collect()
}

const EVERY_KIND: [&str; 3] = ["dependencies", "build-dependencies", "dev-dependencies"];

/// The `.rs` files under a directory, as paths relative to the repository.
fn sources(dir: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&root().join(dir), &mut found);
    found.sort();
    found
        .into_iter()
        .map(|path| {
            path.strip_prefix(root())
                .expect("a path under the repository")
                .to_path_buf()
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Scope {
    /// The whole file, its tests included.
    All,
    /// Up to the file's `#[cfg(test)] mod tests`.
    BeforeTests,
}

/// Whether `line` has `needle` as a path of its own, not as the tail of a
/// longer name (`prefs::` is not `fs::`).
fn names_path(line: &str, needle: &str) -> bool {
    line.match_indices(needle).any(|(at, _)| {
        !line[..at]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_alphanumeric() || before == '_')
    })
}

/// The lines of `files` that name one of `needles`, comments aside, as
/// `path:line: text`.
fn offenders(files: &[PathBuf], scope: Scope, needles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        let path = root().join(file);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut previous = "";
        for (index, line) in text.lines().enumerate() {
            let code = line.trim();
            if matches!(scope, Scope::BeforeTests)
                && previous == "#[cfg(test)]"
                && code.starts_with("mod tests")
            {
                break;
            }
            previous = code;
            if code.starts_with("//") {
                continue;
            }
            if needles.iter().any(|needle| names_path(code, needle)) {
                found.push(format!("{}:{}: {code}", file.display(), index + 1));
            }
        }
    }
    found
}

fn assert_none(found: Vec<String>, rule: &str) {
    assert!(found.is_empty(), "{rule}:\n{}", found.join("\n"));
}

/// ports R1: the rules below cover every crate there is.
#[test]
fn the_workspace_is_the_five_crates_these_rules_cover() {
    let manifest = manifest(".");
    let members: BTreeSet<&str> = manifest["workspace"]["members"]
        .as_array()
        .expect("workspace.members")
        .iter()
        .filter_map(|member| member.as_str())
        .collect();
    let expected: BTreeSet<&str> = CRATES.iter().map(|(dir, _)| *dir).collect();
    assert_eq!(
        members, expected,
        "a new crate needs its place in docs/architecture/README.md and rules in this file"
    );
    for (dir, name) in CRATES {
        assert_eq!(
            self::manifest(dir)["package"]["name"].as_str(),
            Some(name),
            "{dir}"
        );
    }
}

/// ports R1, R2, R3; fcis R4; railway R2 (no `anyhow` below the app).
#[test]
fn crates_below_the_app_name_only_their_allowed_dependencies() {
    for (dir, allowed) in ALLOWED {
        let named = dependencies(dir, &EVERY_KIND);
        assert!(!named.is_empty(), "{dir} names no dependency at all");
        let extra: Vec<&String> = named
            .iter()
            .filter(|name| !allowed.contains(&name.as_str()))
            .collect();
        assert!(
            extra.is_empty(),
            "{dir} may depend only on {allowed:?} (docs/architecture/ports-and-adapters.md R3), \
             but also names {extra:?}"
        );
    }
}

/// ports R1: the positive direction, so the rule can't hold vacuously.
#[test]
fn the_app_and_the_helper_build_on_the_shared_crates() {
    let app = dependencies(".", &["dependencies"]);
    for shared in ["brindle-ui", "brindle-term-core", "brindle-remote-protocol"] {
        assert!(app.contains(shared), "brindle must depend on {shared}");
    }
    let helper = dependencies("remote-agent", &["dependencies"]);
    for shared in ["brindle-term-core", "brindle-remote-protocol"] {
        assert!(
            helper.contains(shared),
            "brindle-remote-agent must depend on {shared}"
        );
    }
}

/// ports R1: the two binaries share crates, never each other. ports R12:
/// Zed's GPL terminal crates are never dependencies.
#[test]
fn no_crate_names_a_forbidden_dependency() {
    let app = dependencies(".", &EVERY_KIND);
    assert!(
        !app.contains("brindle-remote-agent"),
        "brindle must not depend on the helper's crate: shared code goes in \
         brindle-remote-protocol or brindle-term-core"
    );
    for (dir, name) in CRATES {
        let named = dependencies(dir, &EVERY_KIND);
        for gpl in ["terminal", "terminal_view"] {
            assert!(
                !named.contains(gpl),
                "{name} must not depend on Zed's GPL crate {gpl}"
            );
        }
    }
}

/// fcis R4: `brindle-term-core` is pure as a crate.
#[test]
fn term_core_does_no_io_and_reads_no_clock() {
    let files = sources("term-core/src");
    assert!(!files.is_empty());
    let needles = [&IO[..], &CLOCK_AND_THREADS[..]].concat();
    assert_none(
        offenders(&files, Scope::All, &needles),
        "brindle-term-core takes bytes and values and returns values \
         (docs/architecture/functional-core-imperative-shell.md R4)",
    );
}

/// ports R2: components reach the system only through gpui.
#[test]
fn brindle_ui_does_no_io_of_its_own() {
    let files = sources("ui/src");
    assert!(!files.is_empty());
    assert_none(
        offenders(&files, Scope::All, &IO),
        "brindle-ui reaches the system only through gpui \
         (docs/architecture/ports-and-adapters.md R2)",
    );
}

/// process R6: the protocols' message modules are codecs and nothing else.
#[test]
fn the_wire_modules_do_no_io() {
    let files: Vec<PathBuf> = ["client", "holder", "wire", "version", "relay", "fuzz"]
        .iter()
        .map(|module| PathBuf::from(format!("remote-protocol/src/{module}.rs")))
        .collect();
    let needles = [&IO[..], &CLOCK_AND_THREADS[..]].concat();
    assert_none(
        offenders(&files, Scope::All, &needles),
        "the protocol's message modules only encode and decode \
         (docs/architecture/process-architecture.md R6)",
    );
}

/// ports R5, process R8: the server's core and holder reach the system
/// only through `server::platform`.
#[test]
fn the_server_core_and_holder_name_no_platform_type() {
    let mut files = sources("remote-agent/src/server/core");
    assert!(!files.is_empty());
    files.push(PathBuf::from("remote-agent/src/server/core.rs"));
    files.push(PathBuf::from("remote-agent/src/server/holder.rs"));
    assert_none(
        offenders(
            &files,
            Scope::BeforeTests,
            &["libc::", "windows::", "std::os::"],
        ),
        "the core and the holder name no platform type; what they need from the system \
         goes in server/platform/{unix,windows} (docs/architecture/ports-and-adapters.md R5)",
    );
}

/// testing R3: the opt-in suites keep their own runner.
#[test]
fn the_opt_in_suites_are_harness_false_targets() {
    let manifest = manifest(".");
    let tests = manifest["test"].as_array().expect("[[test]] targets");
    for suite in ["e2e", "perf"] {
        let target = tests
            .iter()
            .find(|test| test.get("name").and_then(|name| name.as_str()) == Some(suite))
            .unwrap_or_else(|| panic!("no [[test]] target named {suite}"));
        assert_eq!(
            target.get("harness").and_then(|harness| harness.as_bool()),
            Some(false),
            "{suite} must be a harness = false target"
        );
    }
}

/// The search itself: a path of its own counts, the tail of a longer name
/// doesn't.
#[test]
fn a_needle_matches_only_a_path_of_its_own() {
    assert!(names_path("use std::fs;", "std::fs"));
    assert!(names_path("let text = fs::read(path)?;", "fs::"));
    assert!(names_path("(env::var_os(name))", "env::"));
    assert!(!names_path("prefs::load()", "fs::"));
    assert!(!names_path("self.subprocess::<T>()", "process::"));
}

// ports R4: a platform `cfg` in production code stays in the seam that owns
// its concern, or in a file on the rule's exception.

/// The seams of `docs/architecture/ports-and-adapters.md`'s "How it applies
/// here" table, as files and (ending in `/`) directories. This mirrors that
/// table and changes with it, in the same commit.
const SEAMS: &[&str] = &[
    // the terminal server's OS layer
    "remote-agent/src/server/platform/",
    // PTYs and pseudoconsoles
    "remote-agent/src/pty.rs",
    "remote-agent/src/pty/",
    "remote-agent/src/conpty/",
    // the server directory, endpoint names, access checks, scopes
    "remote-protocol/src/server_dir.rs",
    "remote-protocol/src/endpoint.rs",
    "remote-protocol/src/access.rs",
    "remote-protocol/src/scope.rs",
    "remote-protocol/src/win.rs",
    // the app's connection to a server
    "src/remote/local/unix.rs",
    "src/remote/local/windows.rs",
    "src/remote/local/pipe.rs",
    // a remote endpoint's transport
    "src/remote/client.rs",
    // a local program's name and directory, and its directory reports' path style
    "src/terminal/process.rs",
    // the primary selection
    "src/primary.rs",
    // desktop notifications and window activation
    "src/notify/dbus.rs",
    "src/notify/toast.rs",
    "src/notify/activation.rs",
    // launcher registration
    "src/desktop.rs",
    // opening links
    "src/terminal/links.rs",
    // small Windows helpers
    "src/win.rs",
    // the relay, which runs on Linux only
    "remote-agent/src/server/relay.rs",
    "remote-agent/src/liveness.rs",
    // resources compiled into the Windows executables
    "build.rs",
    "remote-agent/build.rs",
];

/// `ports R4`'s exception: the production files that still have platform
/// `cfg`s outside a seam, listed again in `docs/architecture/README.md`'s
/// `ports R4` row. It only shrinks, and ends with `port-to-macos`.
const R4_EXCEPTION: [&str; 17] = [
    "src/instance.rs",
    "src/main.rs",
    "src/notify/mod.rs",
    "src/remote/local.rs",
    "src/remote/local/relay.rs",
    "src/remote/mod.rs",
    "src/session.rs",
    "src/settings.rs",
    "src/terminal/mod.rs",
    "src/terminal_view.rs",
    "src/workspace.rs",
    "src/zone_view.rs",
    "remote-agent/src/main.rs",
    "remote-agent/src/server/cli.rs",
    "remote-agent/src/server/core.rs",
    "remote-agent/src/server/core/sessions.rs",
    "remote-agent/src/server/daemon.rs",
];

/// The words that make a `cfg` predicate a platform's: `unix` and `windows`
/// as names, `target_os` and `target_family` as keys.
const PLATFORM_WORDS: [&str; 4] = ["unix", "windows", "target_os", "target_family"];

/// What a `cfg` predicate says about the builds it holds in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Predicate {
    /// It holds only in test builds: `test`, or `all(…)` with a test-only
    /// argument.
    TestOnly,
    /// It names a platform (`PLATFORM_WORDS`), and isn't test-only.
    Platform,
    /// Neither, such as `debug_assertions`, `feature = "x"` or `not(test)`.
    Neither,
}

/// A `cfg` predicate's syntax: a name, a `key = "value"` pair (kept by its
/// key), or `all`, `any` or `not` over predicates.
enum Cfg {
    Name(String),
    Pair(String),
    Call(String, Vec<Cfg>),
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A predicate's tokens: words, `"` for a whole string, and punctuation.
fn cfg_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '"' {
            let mut escaped = false;
            for c in chars.by_ref() {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => break,
                    (false, _) => {}
                }
            }
            tokens.push("\"".to_string());
        } else if is_word(c) {
            let mut word = c.to_string();
            while let Some(&next) = chars.peek().filter(|&&next| is_word(next)) {
                word.push(next);
                chars.next();
            }
            tokens.push(word);
        } else {
            tokens.push(c.to_string());
        }
    }
    tokens
}

/// One predicate from the front of `tokens`, with the tokens after it.
fn parse_cfg(tokens: &[String]) -> Option<(Cfg, &[String])> {
    let (word, rest) = tokens.split_first()?;
    if !word.chars().all(is_word) {
        return None;
    }
    match rest.first().map(String::as_str) {
        Some("(") => {
            let mut rest = &rest[1..];
            let mut args = Vec::new();
            loop {
                if rest.first().map(String::as_str) == Some(")") {
                    return Some((Cfg::Call(word.clone(), args), &rest[1..]));
                }
                let (arg, after) = parse_cfg(rest)?;
                args.push(arg);
                rest = match after.first().map(String::as_str) {
                    Some(",") => &after[1..],
                    Some(")") => after,
                    Some(_) | None => return None,
                };
            }
        }
        Some("=") => (rest.get(1).map(String::as_str) == Some("\""))
            .then(|| (Cfg::Pair(word.clone()), &rest[2..])),
        Some(_) | None => Some((Cfg::Name(word.clone()), rest)),
    }
}

fn test_only(cfg: &Cfg) -> bool {
    match cfg {
        Cfg::Name(name) => name == "test",
        Cfg::Pair(_) => false,
        Cfg::Call(name, args) => name == "all" && args.iter().any(test_only),
    }
}

fn names_platform(cfg: &Cfg) -> bool {
    match cfg {
        Cfg::Name(word) | Cfg::Pair(word) => PLATFORM_WORDS.contains(&word.as_str()),
        Cfg::Call(_, args) => args.iter().any(names_platform),
    }
}

/// What the predicate `text` (the inside of `cfg(…)`) says. Text that isn't
/// a predicate (a macro's `$meta`) is a platform's if it has a platform word.
fn predicate(text: &str) -> Predicate {
    let tokens = cfg_tokens(text);
    match parse_cfg(&tokens) {
        Some((cfg, [])) if test_only(&cfg) => Predicate::TestOnly,
        Some((cfg, [])) if names_platform(&cfg) => Predicate::Platform,
        Some((_, [])) => Predicate::Neither,
        Some(_) | None
            if tokens
                .iter()
                .any(|token| PLATFORM_WORDS.contains(&token.as_str())) =>
        {
            Predicate::Platform
        }
        Some(_) | None => Predicate::Neither,
    }
}

/// `text` with its comments and the insides of its string and character
/// literals blanked (newlines kept), so that only code is searched while
/// every offset and line number still matches `text`. Quotes stay, so a
/// `#[path = "…"]` can be read back from `text`.
fn code_of(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut code = bytes.to_vec();
    let blank = |code: &mut [u8], range: Range<usize>| {
        for byte in &mut code[range] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let word_at = |at: Option<usize>| at.is_some_and(|at| is_word(char::from(bytes[at])));
    let mut at = 0;
    while at < bytes.len() {
        let next = bytes.get(at + 1).copied();
        match (bytes[at], next) {
            (b'/', Some(b'/')) => {
                let end = bytes[at..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(bytes.len(), |n| at + n);
                blank(&mut code, at..end);
                at = end;
            }
            (b'/', Some(b'*')) => {
                let (mut depth, mut end) = (0, at);
                while end < bytes.len() {
                    if bytes[end..].starts_with(b"/*") {
                        depth += 1;
                        end += 2;
                    } else if bytes[end..].starts_with(b"*/") {
                        depth -= 1;
                        end += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        end += 1;
                    }
                }
                blank(&mut code, at..end);
                at = end;
            }
            (b'"', _) => {
                let mut end = at + 1;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                let end = end.min(bytes.len());
                blank(&mut code, at + 1..end);
                at = end + 1;
            }
            // A raw string (`r"…"`, `br#"…"#`), not a word ending in `r`.
            (b'r', Some(b'"' | b'#'))
                if match at.checked_sub(1) {
                    Some(before) if bytes[before] == b'b' => !word_at(before.checked_sub(1)),
                    before => !word_at(before),
                } =>
            {
                let hashes = bytes[at + 1..].iter().take_while(|&&b| b == b'#').count();
                let open = at + 1 + hashes;
                if bytes.get(open) != Some(&b'"') {
                    at += 1;
                    continue;
                }
                let close: Vec<u8> = std::iter::once(b'"')
                    .chain(std::iter::repeat_n(b'#', hashes))
                    .collect();
                let end = bytes[open + 1..]
                    .windows(close.len())
                    .position(|window| window == close)
                    .map_or(bytes.len(), |n| open + 1 + n);
                blank(&mut code, open + 1..end);
                at = end + close.len();
            }
            // A character literal (`'{'`, `'\''`), or else a lifetime or label.
            (b'\'', Some(next)) => {
                let close = if next == b'\\' {
                    bytes
                        .get(at + 3..)
                        .and_then(|rest| rest.iter().position(|&b| b == b'\''))
                        .map(|n| at + 3 + n)
                } else {
                    let width = match next {
                        0x00..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    (bytes.get(at + 1 + width) == Some(&b'\'')).then_some(at + 1 + width)
                };
                match close {
                    Some(close) => {
                        blank(&mut code, at + 1..close);
                        at = close + 1;
                    }
                    None => at += 1,
                }
            }
            _ => at += 1,
        }
    }
    String::from_utf8(code).expect("blanking whole characters keeps UTF-8")
}

/// The offset of the first non-space at or after `at`.
fn skip_space(code: &str, at: usize) -> usize {
    code.len() - code[at..].trim_start().len()
}

/// The offset after `word` at `at`, when the code there is that word and
/// not the start of a longer one.
fn keyword(code: &str, at: usize, word: &str) -> Option<usize> {
    let end = at + word.len();
    (code[at..].starts_with(word) && !code[end..].starts_with(is_word)).then_some(end)
}

/// The offset of the bracket that closes the one at `open`.
fn closing(code: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (offset, byte) in code.bytes().enumerate().skip(open) {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// An attribute in the code: `#[…]`, or `#![…]` when `inner`.
struct Attribute {
    start: usize,
    end: usize,
    inner: bool,
    body: Range<usize>,
}

/// The attribute starting at `at`, read until its brackets balance.
fn attribute_at(code: &str, at: usize) -> Option<Attribute> {
    let bytes = code.as_bytes();
    if bytes.get(at) != Some(&b'#') {
        return None;
    }
    let inner = bytes.get(at + 1) == Some(&b'!');
    let open = at + 1 + usize::from(inner);
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let close = closing(code, open)?;
    Some(Attribute {
        start: at,
        end: close + 1,
        inner,
        body: open + 1..close,
    })
}

/// What an attribute is, as far as `ports R4` cares.
enum Meta<'a> {
    /// `cfg(PREDICATE)`.
    Cfg(&'a str),
    /// `cfg_attr(PREDICATE, ATTRIBUTE, …)`.
    CfgAttr(&'a str, Vec<&'a str>),
    /// `path = "FILE"`, with the file read from the text.
    Path(&'a str),
    Other,
}

/// The parts of `text` between commas outside brackets, trimmed, without
/// empty ones.
fn arguments(text: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0, 0);
    for (at, c) in text.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

fn meta<'a>(code: &'a str, text: &'a str, attribute: &Attribute) -> Meta<'a> {
    let body = code[attribute.body.clone()].trim();
    let (word, rest) = body.split_at(body.find(|c: char| !is_word(c)).unwrap_or(body.len()));
    let rest = rest.trim_start();
    let inside = rest
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'));
    match (word, inside) {
        ("cfg", Some(inside)) => Meta::Cfg(inside),
        ("cfg_attr", Some(inside)) => match arguments(inside).split_first() {
            Some((predicate, attributes)) => Meta::CfgAttr(predicate, attributes.to_vec()),
            None => Meta::Other,
        },
        ("path", None) if rest.starts_with('=') => {
            let quotes: Vec<usize> = code[attribute.body.clone()]
                .match_indices('"')
                .map(|(at, _)| attribute.body.start + at)
                .collect();
            match quotes[..] {
                [open, close] => Meta::Path(&text[open + 1..close]),
                _ => Meta::Other,
            }
        }
        _ => Meta::Other,
    }
}

/// Whether a `cfg_attr`'s attributes only set lint levels, which changes no
/// code (`fcis R7`'s ungated helpers use it for `dead_code`).
fn lint_only(attributes: &[&str]) -> bool {
    !attributes.is_empty()
        && attributes.iter().all(|attribute| {
            ["allow", "expect", "warn", "deny"].iter().any(|lint| {
                keyword(attribute, 0, lint)
                    .is_some_and(|end| attribute[end..].trim_start().starts_with('('))
            })
        })
}

/// The module a `mod NAME;` at `at` declares (any visibility), or `None`
/// for any other item, an inline `mod NAME { … }` included.
fn mod_declaration(code: &str, at: usize) -> Option<&str> {
    let mut at = skip_space(code, at);
    if let Some(end) = keyword(code, at, "pub") {
        at = skip_space(code, end);
        if code[at..].starts_with('(') {
            at = skip_space(code, closing(code, at)? + 1);
        }
    }
    let start = skip_space(code, keyword(code, at, "mod")?);
    let end = code[start..]
        .find(|c: char| !is_word(c))
        .map_or(code.len(), |n| start + n);
    (end > start && code[skip_space(code, end)..].starts_with(';')).then(|| &code[start..end])
}

/// The offset after the item starting at `at`: after its `;`, or after the
/// brace that closes its first `{`.
fn item_end(code: &str, at: usize) -> usize {
    let (mut nesting, mut braces) = (0, 0);
    for (offset, byte) in code.bytes().enumerate().skip(at) {
        match byte {
            b'(' | b'[' => nesting += 1,
            b')' | b']' => nesting -= 1,
            b';' if nesting == 0 && braces == 0 => return offset + 1,
            b'{' => braces += 1,
            // The block around the item ends, so the item has.
            b'}' if braces == 0 => return offset,
            b'}' => {
                braces -= 1;
                if braces == 0 {
                    return offset + 1;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// Paths joined with `/`, with `.` and `..` resolved.
fn joined(dir: &str, path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in dir.split('/').chain(path.split(['/', '\\'])) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// The file of `mod name;` declared in the file `declaring`, as rustc finds
/// it: a `#[path]` relative to the declaring file's directory; else
/// `name.rs`, or `name/mod.rs` when `files` has no `name.rs`, beside a
/// `mod.rs`, `lib.rs` or `main.rs`, and under a directory named for any
/// other declaring file.
fn module_file(
    declaring: &str,
    name: &str,
    path: Option<&str>,
    files: &BTreeSet<String>,
) -> String {
    let (dir, file) = declaring.rsplit_once('/').unwrap_or(("", declaring));
    if let Some(path) = path {
        return joined(dir, path);
    }
    let base = match file {
        "mod.rs" | "lib.rs" | "main.rs" => dir.to_string(),
        file => joined(dir, file.trim_end_matches(".rs")),
    };
    let flat = joined(&base, &format!("{name}.rs"));
    if files.contains(&flat) {
        flat
    } else {
        joined(&base, &format!("{name}/mod.rs"))
    }
}

/// Whether `file` is in one of `seams` (a file, or a directory ending in `/`).
fn in_seams(file: &str, seams: &[&str]) -> bool {
    seams.iter().any(|seam| {
        if seam.ends_with('/') {
            file.starts_with(seam)
        } else {
            file == *seam
        }
    })
}

/// What scanning one file for platform `cfg`s found.
#[derive(Debug, Default)]
struct Scan {
    /// The platform `cfg`s outside the allowances, as `path:line: text`.
    sites: Vec<String>,
    /// The files of the modules it declares under a test-only `cfg`.
    test_modules: Vec<String>,
}

/// The platform `cfg`s in `text`, the file at `path` (`ports R4`): every
/// `cfg(…)` and `cfg!(…)` whose predicate names a platform, and every
/// `cfg_attr(…)` whose first argument does, outer or inner, except
/// - test code: an item under a test-only `cfg`, read to its `;` or to the
///   brace that closes its first `{` (a `mod NAME;` there is returned as a
///   test module, whose file is test code too);
/// - a seam's module declaration: a platform `cfg` on `mod NAME;`, among
///   any other attributes, whose file is in `seams`;
/// - a lint-only `cfg_attr`, which sets only `allow`, `expect`, `warn` or
///   `deny`.
///
/// Comments and the insides of literals are skipped. `files` are the files
/// there are, for finding a module's (`/`-separated, as `path` is).
fn platform_cfgs(path: &str, text: &str, seams: &[&str], files: &BTreeSet<String>) -> Scan {
    let code = code_of(text);
    let bytes = code.as_bytes();
    let (mut found, mut test_modules) = (Vec::new(), Vec::new());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(attribute) = attribute_at(&code, at).filter(|attribute| attribute.inner) {
            match meta(&code, text, &attribute) {
                Meta::Cfg(cfg) => match predicate(cfg) {
                    // The rest of the module is test code.
                    Predicate::TestOnly => {
                        at = code.len();
                        continue;
                    }
                    Predicate::Platform => found.push(attribute.start),
                    Predicate::Neither => {}
                },
                Meta::CfgAttr(cfg, set) => match predicate(cfg) {
                    Predicate::Platform if !lint_only(&set) => found.push(attribute.start),
                    Predicate::Platform | Predicate::TestOnly | Predicate::Neither => {}
                },
                Meta::Path(_) | Meta::Other => {}
            }
            at = attribute.end;
        } else if let Some(first) = attribute_at(&code, at) {
            // The outer attributes on one item, then the item.
            let mut item = skip_space(&code, first.end);
            let mut attributes = vec![first];
            while let Some(next) = attribute_at(&code, item).filter(|next| !next.inner) {
                item = skip_space(&code, next.end);
                attributes.push(next);
            }
            let (mut test, mut gates, mut others, mut file) = (false, Vec::new(), Vec::new(), None);
            for attribute in &attributes {
                match meta(&code, text, attribute) {
                    Meta::Cfg(cfg) => match predicate(cfg) {
                        Predicate::TestOnly => test = true,
                        Predicate::Platform => gates.push(attribute.start),
                        Predicate::Neither => {}
                    },
                    Meta::CfgAttr(cfg, set) => match predicate(cfg) {
                        Predicate::Platform if !lint_only(&set) => others.push(attribute.start),
                        Predicate::Platform | Predicate::TestOnly | Predicate::Neither => {}
                    },
                    Meta::Path(path) => file = Some(path),
                    Meta::Other => {}
                }
            }
            let module =
                mod_declaration(&code, item).map(|name| module_file(path, name, file, files));
            if test {
                test_modules.extend(module);
                at = item_end(&code, item);
            } else {
                if !module.is_some_and(|module| in_seams(&module, seams)) {
                    found.extend(gates);
                }
                found.extend(others);
                at = item;
            }
        } else if bytes[at..].starts_with(b"cfg!")
            && !at
                .checked_sub(1)
                .is_some_and(|before| is_word(char::from(bytes[before])))
        {
            let open = skip_space(&code, at + 4);
            match closing(&code, open).filter(|_| bytes.get(open) == Some(&b'(')) {
                Some(close) => {
                    if predicate(&code[open + 1..close]) == Predicate::Platform {
                        found.push(at);
                    }
                    at = close + 1;
                }
                None => at += 4,
            }
        } else {
            at += 1;
        }
    }
    found.sort_unstable();
    let lines: Vec<&str> = text.lines().collect();
    let sites = found
        .into_iter()
        .map(|offset| {
            let line = bytes[..offset].iter().filter(|&&b| b == b'\n').count();
            format!("{path}:{}: {}", line + 1, lines[line].trim())
        })
        .collect();
    Scan {
        sites,
        test_modules,
    }
}

/// `files` as a set, for [`platform_cfgs`] and [`module_file`].
fn known(files: &[&str]) -> BTreeSet<String> {
    files.iter().map(|file| file.to_string()).collect()
}

/// A path relative to the repository, with `/` separators on every platform.
fn slashed(path: &Path) -> String {
    let parts: Vec<_> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}

/// Production code, as `ports R4` defines it: each crate's `src/` tree and
/// the build scripts.
fn production_files() -> Vec<String> {
    let mut files: Vec<String> = CRATES
        .iter()
        .flat_map(|(dir, _)| {
            sources(&if *dir == "." {
                "src".to_string()
            } else {
                format!("{dir}/src")
            })
        })
        .map(|path| slashed(&path))
        .collect();
    files.extend(["build.rs", "remote-agent/build.rs"].map(String::from));
    files
}

/// Whether `file` is a test module's file, or a file under its directory.
fn in_test_module(file: &str, test_modules: &[String]) -> bool {
    test_modules.iter().any(|module| {
        let dir = module
            .strip_suffix("/mod.rs")
            .or_else(|| module.strip_suffix(".rs"))
            .unwrap_or(module);
        file == module || file.starts_with(&format!("{dir}/"))
    })
}

/// Each production file's platform `cfg`s outside the allowances, without
/// the files of test modules.
fn production_platform_cfgs() -> BTreeMap<String, Vec<String>> {
    let files = production_files();
    let known: BTreeSet<String> = files.iter().cloned().collect();
    let (mut sites, mut test_modules) = (BTreeMap::new(), Vec::new());
    for file in &files {
        let path = root().join(file);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let scan = platform_cfgs(file, &text, SEAMS, &known);
        test_modules.extend(scan.test_modules);
        sites.insert(file.clone(), scan.sites);
    }
    sites.retain(|file, _| !in_test_module(file, &test_modules));
    sites
}

/// ports R4: production code has a platform `cfg` only in the seam that
/// owns the concern, or in a file on the rule's exception.
#[test]
fn platform_cfgs_stay_in_their_seams() {
    let sites = production_platform_cfgs();
    for walked in [
        "src/main.rs",
        "remote-agent/src/server/core.rs",
        "term-core/src/lib.rs",
        "build.rs",
    ] {
        assert!(
            sites.contains_key(walked),
            "the walk of production code missed {walked}"
        );
    }
    let outside: Vec<&String> = sites
        .iter()
        .filter(|(file, _)| !in_seams(file, SEAMS) && !R4_EXCEPTION.contains(&file.as_str()))
        .flat_map(|(_, sites)| sites)
        .collect();
    assert!(
        outside.is_empty(),
        "platform cfgs outside a seam (docs/architecture/ports-and-adapters.md R4):\n{}\n\
         Move each difference into the seam that owns its concern (an item there for every platform), \
         or change README's `ports R4` row (docs/architecture/README.md) and R4_EXCEPTION in this file \
         in the same commit. A seam's own `#[cfg(…)] mod NAME;` and a lint-only `cfg_attr` are allowed. \
         The scanner reads text, not Rust: the line it names shows what it took for a cfg.",
        outside
            .iter()
            .map(|site| site.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// ports R4: the exception only shrinks. A file whose last platform `cfg`
/// moved into a seam leaves it.
#[test]
fn every_excepted_file_still_has_a_platform_cfg() {
    let sites = production_platform_cfgs();
    let clean: Vec<&str> = R4_EXCEPTION
        .into_iter()
        .filter(|file| sites.get(*file).is_none_or(Vec::is_empty))
        .collect();
    assert!(
        clean.is_empty(),
        "these files have no platform cfg outside a seam any more: take them off R4_EXCEPTION here \
         and README's `ports R4` row (docs/architecture/README.md) in the same commit: {clean:?}"
    );
}

/// ports R4: the seam table and the exception name paths that exist.
#[test]
fn every_seam_and_excepted_path_exists() {
    let missing: Vec<&str> = SEAMS
        .iter()
        .copied()
        .chain(R4_EXCEPTION)
        .filter(|path| match path.strip_suffix('/') {
            Some(dir) => !root().join(dir).is_dir(),
            None => !root().join(path).is_file(),
        })
        .collect();
    assert!(
        missing.is_empty(),
        "SEAMS and R4_EXCEPTION name paths that don't exist; change them with \
         ports-and-adapters.md's seam table and README's `ports R4` row: {missing:?}"
    );
}

/// ports R4: README's exception row and `R4_EXCEPTION` list the same files.
#[test]
fn the_ports_r4_exception_matches_the_readme() {
    let path = root().join("docs/architecture/README.md");
    let readme = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let row = readme
        .lines()
        .find(|line| line.starts_with("| `ports R4` |"))
        .expect("README's \"Exceptions in force\" has a `ports R4` row");
    let listed: BTreeSet<&str> = row
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|quoted| quoted.ends_with(".rs"))
        .collect();
    assert_eq!(
        listed,
        BTreeSet::from(R4_EXCEPTION),
        "README's `ports R4` row and R4_EXCEPTION must list the same files"
    );
}

/// The platform `cfg` scanner itself: every form it is taught is found,
/// with its line.
#[test]
fn a_platform_cfg_outside_a_seam_is_found() {
    let text = r#"#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#[cfg(windows)] fn f() {}
fn g() {
    if cfg!(target_os = "linux") {}
}
#[cfg(any(target_os = "linux", windows))]
fn h() {}
#[cfg(not(unix))]
fn i() {}
#[cfg(target_family = "unix")]
fn j() {}
#[cfg_attr(
    windows,
    path = "x.rs"
)]
mod k;
#[cfg(windows)] mod helpers;
"#;
    let files = known(&["a/lib.rs", "a/helpers.rs", "a/x.rs", "a/k.rs"]);
    let scan = platform_cfgs("a/lib.rs", text, &["a/win.rs"], &files);
    assert_eq!(
        scan.sites,
        [
            r#"a/lib.rs:1: #![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]"#,
            "a/lib.rs:2: #[cfg(windows)] fn f() {}",
            r#"a/lib.rs:4: if cfg!(target_os = "linux") {}"#,
            r#"a/lib.rs:6: #[cfg(any(target_os = "linux", windows))]"#,
            "a/lib.rs:8: #[cfg(not(unix))]",
            r#"a/lib.rs:10: #[cfg(target_family = "unix")]"#,
            "a/lib.rs:12: #[cfg_attr(",
            "a/lib.rs:17: #[cfg(windows)] mod helpers;",
        ]
    );
    assert!(scan.test_modules.is_empty());
}

/// The scanner's allowances: test code, lint-only `cfg_attr`s, seam module
/// declarations, comments, paths that merely contain a platform's name, and
/// `cfg`s that name no platform.
#[test]
fn what_is_not_a_platform_cfg_is_not_found() {
    let text = r#"#[cfg(test)]
mod tests {
    #[cfg(unix)]
    fn g() {}
}
#[cfg(test)]
fn t() {
    if cfg!(windows) {}
}
#[cfg(all(test, unix))]
mod helpers;
#[cfg_attr(not(windows), allow(dead_code))]
fn a() {}
#[cfg_attr(windows, allow(dead_code, reason = "…"))]
fn b() {}
use windows::Win32;
// #[cfg(windows)]
#[cfg(debug_assertions)]
fn c() {}
#[cfg(feature = "x")]
fn d() {}
#[cfg(windows)]
mod win;
#[cfg(unix)]
#[path = "local/unix.rs"]
mod sys;
"#;
    let files = known(&["a/lib.rs", "a/helpers.rs", "a/win.rs", "a/local/unix.rs"]);
    let scan = platform_cfgs("a/lib.rs", text, &["a/win.rs", "a/local/unix.rs"], &files);
    assert_eq!(scan.sites, Vec::<String>::new());
    assert_eq!(scan.test_modules, ["a/helpers.rs"]);
}

/// Braces in literals and comments don't end a skipped test item early,
/// lifetimes aren't literals, and a `cfg` written in a string isn't one.
#[test]
fn literals_and_comments_are_not_code_to_the_scanner() {
    let text = r##"#[cfg(test)]
mod tests {
    const OPEN: char = '{';
    const CLOSE: char = '}';
    const QUOTE: char = '\'';
    const TEXT: &str = "}} \" }";
    const RAW: &str = r#"}"#;
    /* } /* } */ } */
    // }
    #[cfg(unix)]
    fn g() {}
}
const S: &str = "#[cfg(windows)]";
fn f<'a>(x: &'a str) -> &'a str { x }
#[cfg(windows)] fn h() {}
"##;
    let scan = platform_cfgs("a/lib.rs", text, &[], &known(&["a/lib.rs"]));
    assert_eq!(scan.sites, ["a/lib.rs:15: #[cfg(windows)] fn h() {}"]);
}

/// Only a predicate that needs `test` to hold makes an item test code.
#[test]
fn a_test_only_predicate_needs_test_to_hold() {
    assert_eq!(predicate("test"), Predicate::TestOnly);
    assert_eq!(predicate("all(windows, test)"), Predicate::TestOnly);
    assert_eq!(predicate("not(test)"), Predicate::Neither);
    assert_eq!(predicate("any(test, unix)"), Predicate::Platform);
    assert_eq!(predicate("unix"), Predicate::Platform);
}

/// `mod NAME;` names the file rustc would read.
#[test]
fn a_declared_module_resolves_as_rustc_resolves_it() {
    let files = known(&["a/x.rs", "a/b/x.rs", "a/b/y.rs", "a/m/mod.rs"]);
    assert_eq!(module_file("a/lib.rs", "x", None, &files), "a/x.rs");
    assert_eq!(module_file("a/b.rs", "x", None, &files), "a/b/x.rs");
    assert_eq!(module_file("a/b/mod.rs", "x", None, &files), "a/b/x.rs");
    assert_eq!(
        module_file("a/c.rs", "y", Some("b/y.rs"), &files),
        "a/b/y.rs"
    );
    assert_eq!(module_file("a/lib.rs", "m", None, &files), "a/m/mod.rs");
}
