//! Architecture checks for the rules in `docs/architecture/`: the crate
//! boundaries (closed dependency lists, no patches), the few source rules a
//! text search can hold (its needles follow `use` items), `fcis R2` and
//! `R3` for the whole pure modules, the opt-in suites' runners, `ports R4`,
//! which keeps production code's platform `cfg`s in their seams (the
//! `platform_cfgs` scanner and the tree checks after it), and `fcis R7`'s
//! `cfg` half, which keeps neutral code ungated even there. Part of plain
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
const CRATES: [(&str, &str); 6] = [
    (".", "brindle"),
    ("remote-agent", "brindle-remote-agent"),
    ("remote-protocol", "brindle-remote-protocol"),
    ("term-core", "brindle-term-core"),
    ("transcript", "brindle-transcript"),
    ("ui", "brindle-ui"),
];

/// Every dependency a crate below the app may name, of any kind (normal,
/// build, dev) and for any target. The app (`brindle`) has no closed list.
const ALLOWED: [(&str, &[&str]); 5] = [
    ("ui", &["gpui", "log"]),
    (
        "term-core",
        &["alacritty_terminal", "base64", "parking_lot"],
    ),
    ("remote-protocol", &["serde", "serde_json", "windows"]),
    ("transcript", &["serde", "serde_json"]),
    (
        "remote-agent",
        &[
            "agent-client-protocol-schema",
            "brindle-remote-protocol",
            "brindle-term-core",
            "brindle-transcript",
            "embed-resource",
            "libc",
            "similar",
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

/// The root manifest's `[workspace.dependencies]`, which a member's
/// `{ workspace = true }` entries name.
fn workspace_dependencies(root: &toml::Table) -> Option<&toml::Table> {
    root.get("workspace")?.get("dependencies")?.as_table()
}

/// The package name of the dependency entry `value` under `key`: its
/// `package = "…"` rename, else the key.
fn package(key: &str, value: &toml::Value) -> String {
    value
        .get("package")
        .and_then(|package| package.as_str())
        .unwrap_or(key)
        .to_string()
}

/// The package names in one dependency table (`package = "…"` renames
/// included). A `{ workspace = true }` entry is read from `workspace`, the
/// root's `[workspace.dependencies]`, so a rename written there counts.
fn names(table: &toml::Table, section: &str, workspace: Option<&toml::Table>) -> Vec<String> {
    let Some(deps) = table.get(section).and_then(|deps| deps.as_table()) else {
        return Vec::new();
    };
    deps.iter()
        .map(|(key, value)| {
            let inherited =
                value.get("workspace").and_then(|inherit| inherit.as_bool()) == Some(true);
            let entry = workspace
                .filter(|_| inherited)
                .and_then(|workspace| workspace.get(key))
                .unwrap_or(value);
            package(key, entry)
        })
        .collect()
}

/// A manifest's dependencies of the given kinds, for every target, with
/// `workspace` the root's `[workspace.dependencies]`.
fn dependency_names(
    manifest: &toml::Table,
    workspace: Option<&toml::Table>,
    sections: &[&str],
) -> BTreeSet<String> {
    let mut tables = vec![manifest];
    if let Some(targets) = manifest.get("target").and_then(|target| target.as_table()) {
        tables.extend(targets.values().filter_map(|target| target.as_table()));
    }
    tables
        .iter()
        .flat_map(|table| {
            sections
                .iter()
                .flat_map(|section| names(table, section, workspace))
        })
        .collect()
}

/// A crate's dependencies of the given kinds, for every target.
fn dependencies(dir: &str, sections: &[&str]) -> BTreeSet<String> {
    let root = manifest(".");
    dependency_names(&manifest(dir), workspace_dependencies(&root), sections)
}

/// Every crate a manifest patches or replaces, by package name: the entries
/// of each `[patch.<source>]` table (`package = "…"` followed) and the
/// `name:version` (or `name@version`) keys of `[replace]`.
fn patched(manifest: &toml::Table) -> Vec<String> {
    let table = |name: &str| manifest.get(name).and_then(|table| table.as_table());
    let patches = table("patch")
        .into_iter()
        .flat_map(|sources| sources.values())
        .filter_map(|source| source.as_table())
        .flat_map(|source| source.iter().map(|(key, value)| package(key, value)));
    let replaced = table("replace")
        .into_iter()
        .flat_map(|replace| replace.keys())
        .map(|spec| spec.split([':', '@']).next().unwrap_or(spec).to_string());
    patches.chain(replaced).collect()
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

/// Whether `text` has `needle` as a path of its own: not the tail of a
/// longer name (`prefs::` is not `fs::`), nor, when the needle ends in a
/// name, its head (`std::environment` is not `std::env`).
fn names_path(text: &str, needle: &str) -> bool {
    let open_ended = !needle.ends_with(is_word);
    text.match_indices(needle).any(|(at, _)| {
        !text[..at].ends_with(is_word)
            && (open_ended || !text[at + needle.len()..].starts_with(is_word))
    })
}

/// One name a `use` item brings into scope: `local` as the code writes it
/// (`*` for a glob) and the path it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Import {
    local: String,
    path: String,
}

/// The names the use tree `tree` imports below `prefix`: a path, `path as
/// name`, `self`, a glob, or a `{…}` group of these, nested to any depth.
fn use_tree(prefix: &str, tree: &str, imports: &mut Vec<Import>) {
    let below = |path: &str| match (prefix, path) {
        (prefix, "") => prefix.to_string(),
        ("", path) => path.to_string(),
        (prefix, path) => format!("{prefix}::{path}"),
    };
    let tree = tree.trim();
    if let Some(open) = tree.find('{')
        && let Some(inside) = tree[open + 1..].strip_suffix('}')
    {
        let head: String = tree[..open].split_whitespace().collect();
        let base = below(head.trim_start_matches("::").trim_end_matches("::"));
        for part in arguments(inside) {
            use_tree(&base, part, imports);
        }
        return;
    }
    let words: Vec<&str> = tree.split_whitespace().collect();
    let (path, alias) = match words.as_slice() {
        [path @ .., "as", alias] => (path.concat(), Some(*alias)),
        path => (path.concat(), None),
    };
    let path = path.trim_start_matches("::");
    let path = match path.strip_suffix("self") {
        Some(head) if head.is_empty() || head.ends_with("::") => head.trim_end_matches("::"),
        Some(_) | None => path,
    };
    let full = below(path);
    if full.is_empty() {
        return;
    }
    let (local, path) = match full.strip_suffix('*') {
        Some(glob) => ("*".to_string(), glob.trim_end_matches("::").to_string()),
        None => {
            let last = full.rsplit("::").next().unwrap_or(&full);
            (alias.unwrap_or(last).to_string(), full.clone())
        }
    };
    imports.push(Import { local, path });
}

/// The `use` items in `code` (code only, as [`code_of`] gives it), each as
/// the offset of its `use` and the names it imports. A `use` item runs to
/// its `;`, across lines; `use<…>` (precise capturing) isn't one.
fn use_items(code: &str) -> Vec<(usize, Vec<Import>)> {
    let mut items = Vec::new();
    let mut from = 0;
    while let Some(found) = code[from..].find("use") {
        let at = from + found;
        from = at + "use".len();
        let Some(after) = keyword(code, at, "use") else {
            continue;
        };
        if code[..at].ends_with(|c: char| is_word(c) || c == '#')
            || code[skip_space(code, after)..].starts_with('<')
        {
            continue;
        }
        let Some(end) = code[after..].find(';').map(|n| after + n) else {
            break;
        };
        let mut imports = Vec::new();
        use_tree("", &code[after..end], &mut imports);
        items.push((at, imports));
        from = end + 1;
    }
    items
}

/// Every name the `use` items in `text` import, in order.
fn use_paths(text: &str) -> Vec<Import> {
    use_items(&code_of(text))
        .into_iter()
        .flat_map(|(_, imports)| imports)
        .collect()
}

/// The paths written on `line` (two segments or more, or rooted at `::`),
/// each with whether it was rooted, segments joined by `::`.
fn paths_in(line: &str) -> Vec<(bool, String)> {
    let bytes = line.as_bytes();
    // Names are ASCII here, so every offset taken is a character boundary.
    let ascii_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let word = |at: usize| bytes.get(at).copied().is_some_and(ascii_word);
    let starts_name = |at: usize| {
        bytes
            .get(at)
            .is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_')
    };
    let mut paths = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let before = at.checked_sub(1).map(|before| bytes[before]);
        if before.is_some_and(|b| ascii_word(b) || b == b':') {
            at += 1;
            continue;
        }
        let rooted = bytes[at..].starts_with(b"::") && before != Some(b'>');
        let start = if rooted { at + 2 } else { at };
        if !starts_name(start) {
            at += 1;
            continue;
        }
        let (mut segments, mut end) = (Vec::new(), start);
        loop {
            let segment = end;
            while word(end) {
                end += 1;
            }
            segments.push(&line[segment..end]);
            if bytes[end..].starts_with(b"::") && starts_name(end + 2) {
                end += 2;
            } else {
                break;
            }
        }
        if rooted || segments.len() > 1 {
            paths.push((rooted, segments.join("::")));
        }
        at = end;
    }
    paths
}

/// The paths on `line` as `imports` resolve them: a path whose first
/// segment an import names starts with that import's path instead
/// (`Clock::now` after `use std::time::Instant as Clock` is
/// `std::time::Instant::now`); one that no import names stays as written,
/// and is also taken under each glob import's path. A rooted path, or one
/// from `std`, `core`, `crate`, `self` or `super`, is taken as written.
fn resolved_paths(line: &str, imports: &[Import]) -> Vec<String> {
    let mut resolved = Vec::new();
    for (rooted, path) in paths_in(line) {
        let (first, rest) = path.split_once("::").unwrap_or((&path, ""));
        if rooted || ["std", "core", "crate", "self", "super"].contains(&first) {
            resolved.push(path);
            continue;
        }
        let named: Vec<String> = imports
            .iter()
            .filter(|import| import.local == first)
            .map(|import| format!("{}::{rest}", import.path))
            .collect();
        if named.is_empty() {
            resolved.extend(
                imports
                    .iter()
                    .filter(|import| import.local == "*")
                    .map(|glob| format!("{}::{path}", glob.path)),
            );
            resolved.push(path);
        } else {
            resolved.extend(named);
        }
    }
    resolved
}

/// The numbers of the lines among `lines` (numbered code, as [`code_lines`]
/// and [`production_lines`] give it) that name one of `needles`
/// ([`names_path`]): written on the line, through a path on it resolved by
/// the imports among `lines` ([`resolved_paths`]), or as an import of a
/// `use` item that starts on it. Today's short needles (`env::`) still match
/// the line as written, so the imports only add sites.
fn needle_lines(lines: &[(usize, String)], needles: &[&str]) -> Vec<usize> {
    let code = lines
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let names = |path: &str| needles.iter().any(|needle| names_path(path, needle));
    let items = use_items(&code);
    let imports: Vec<Import> = items
        .iter()
        .flat_map(|(_, imports)| imports.iter().cloned())
        .collect();
    let importing: BTreeSet<usize> = items
        .iter()
        .filter(|(_, imports)| imports.iter().any(|import| names(&import.path)))
        .map(|(at, _)| code[..*at].matches('\n').count())
        .collect();
    lines
        .iter()
        .enumerate()
        .filter(|(index, (_, line))| {
            importing.contains(index)
                || names(line)
                || resolved_paths(line, &imports)
                    .iter()
                    .any(|path| names(path))
        })
        .map(|(_, (number, _))| *number)
        .collect()
}

/// The lines of `code` that have code, numbered from 1 and trimmed.
fn numbered(code: &str) -> Vec<(usize, String)> {
    code.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.is_empty()).then(|| (index + 1, line.to_string()))
        })
        .collect()
}

/// The code lines of `text` ([`numbered`] over [`code_of`]), up to its
/// `#[cfg(test)] mod tests` for `Scope::BeforeTests`.
fn code_lines(text: &str, scope: Scope) -> Vec<(usize, String)> {
    let lines = numbered(&code_of(text));
    match scope {
        Scope::All => lines,
        Scope::BeforeTests => {
            let tests = lines
                .windows(2)
                .position(|pair| pair[0].1 == "#[cfg(test)]" && pair[1].1.starts_with("mod tests"));
            match tests {
                Some(attribute) => lines[..attribute].to_vec(),
                None => lines,
            }
        }
    }
}

/// The lines of `files` that name one of `needles` ([`needle_lines`]), as
/// `path:line: text`.
fn offenders(files: &[PathBuf], scope: Scope, needles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        let path = root().join(file);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let original: Vec<&str> = text.lines().collect();
        for number in needle_lines(&code_lines(&text, scope), needles) {
            found.push(format!(
                "{}:{number}: {}",
                file.display(),
                original[number - 1].trim()
            ));
        }
    }
    found
}

fn assert_none(found: Vec<String>, rule: &str) {
    assert!(found.is_empty(), "{rule}:\n{}", found.join("\n"));
}

/// ports R1: the rules below cover every crate there is.
#[test]
fn the_workspace_is_the_six_crates_these_rules_cover() {
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

/// ports R1, R2, R3; fcis R4. `{ workspace = true }` entries count by the
/// package the root's `[workspace.dependencies]` names. `railway R2`'s
/// dependency half (no `anyhow` below the app) holds only through these
/// closed lists, because `anyhow` is on none of them; its other half, that
/// each crate returns its own error types, is review's.
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

/// ports R3: no manifest patches or replaces a crate. A patch swaps the
/// code behind a name that the closed lists vouch for.
#[test]
fn no_dependency_is_patched_or_replaced() {
    let found: Vec<String> = CRATES
        .iter()
        .flat_map(|(dir, _)| {
            patched(&manifest(dir))
                .into_iter()
                .map(move |name| format!("{dir}/Cargo.toml: {name}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "a [patch] or [replace] of any crate is a change to docs/architecture/ports-and-adapters.md R3 \
         and to this test, as adding a dependency is: {found:?}"
    );
}

/// The manifest reading: a `{ workspace = true }` entry counts as the crate
/// the root's `[workspace.dependencies]` names, its rename included.
#[test]
fn a_workspace_dependency_counts_by_its_package() {
    let root: toml::Table = r#"
[workspace]
members = ["ui"]

[workspace.dependencies]
term = { package = "alacritty_terminal", version = "0.26" }
serde = "1"
"#
    .parse()
    .expect("a manifest");
    let member: toml::Table = r#"
[dependencies]
term = { workspace = true }
serde = { workspace = true, features = ["derive"] }
json = { package = "serde_json", version = "1" }

[target.'cfg(windows)'.build-dependencies]
resources = { workspace = true }
"#
    .parse()
    .expect("a manifest");
    assert_eq!(
        dependency_names(&member, workspace_dependencies(&root), &EVERY_KIND),
        BTreeSet::from(
            ["alacritty_terminal", "resources", "serde", "serde_json"].map(String::from)
        )
    );
}

/// The manifest reading: every `[patch.*]` entry, by its package, and every
/// `[replace]` entry, by its name.
#[test]
fn a_patch_or_replacement_is_found() {
    let manifest: toml::Table = r#"
[dependencies]
gpui = "0.2.2"

[patch.crates-io]
libc = { git = "https://example.invalid/libc" }
attrs = { path = "vendor/xattr", package = "xattr" }

[patch."https://github.com/zed-industries/zed"]
gpui = { path = "../gpui" }

[replace]
"num-bigint-dig:0.8.4" = { path = "vendor/num-bigint-dig" }
"#
    .parse()
    .expect("a manifest");
    let found: BTreeSet<String> = patched(&manifest).into_iter().collect();
    assert_eq!(
        found,
        BTreeSet::from(["gpui", "libc", "num-bigint-dig", "xattr"].map(String::from))
    );
    let plain: toml::Table = "[dependencies]\nlibc = \"0.2\"\n"
        .parse()
        .expect("a manifest");
    assert_eq!(patched(&plain), Vec::<String>::new());
}

/// ports R1: the positive direction, so the rule can't hold vacuously.
#[test]
fn the_app_and_the_helper_build_on_the_shared_crates() {
    let app = dependencies(".", &["dependencies"]);
    for shared in [
        "brindle-ui",
        "brindle-term-core",
        "brindle-remote-protocol",
        "brindle-transcript",
    ] {
        assert!(app.contains(shared), "brindle must depend on {shared}");
    }
    let helper = dependencies("remote-agent", &["dependencies"]);
    for shared in ["brindle-term-core", "brindle-remote-protocol", "brindle-transcript"] {
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
         brindle-remote-protocol, brindle-term-core or brindle-transcript"
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

/// process R3: a panic is confined to the session it came from (Brindle's
/// guarded sink calls, the core's `Screen::guard`), which needs unwinding,
/// so no profile in the root manifest, a package override included, sets
/// `panic = "abort"`.
#[test]
fn no_profile_aborts_on_panic() {
    fn aborting(path: String, table: &toml::Table, found: &mut Vec<String>) {
        for (key, value) in table {
            let path = format!("{path}.{key}");
            match value {
                toml::Value::Table(inner) => aborting(path, inner, found),
                toml::Value::String(setting) if key == "panic" && setting == "abort" => {
                    found.push(format!("{path} = \"abort\""))
                }
                _ => {}
            }
        }
    }
    let manifest = manifest(".");
    let mut found = Vec::new();
    if let Some(profiles) = manifest.get("profile").and_then(toml::Value::as_table) {
        aborting("profile".into(), profiles, &mut found);
    }
    assert_none(
        found,
        "process R3: no profile may set panic = \"abort\"; the panic guards rely on unwinding",
    );
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

/// fcis R4: `brindle-transcript` is pure as a crate.
#[test]
fn transcript_does_no_io_and_reads_no_clock() {
    let files = sources("transcript/src");
    assert!(!files.is_empty());
    let needles = [&IO[..], &CLOCK_AND_THREADS[..]].concat();
    assert_none(
        offenders(&files, Scope::All, &needles),
        "brindle-transcript takes values and returns values \
         (docs/architecture/functional-core-imperative-shell.md R4)",
    );
}

/// The ACP schema crate's home: no source outside the helper's pure
/// `src/acp/` modules names it, so the core's shell reaches ACP only
/// through `acp::reduce`, and no manifest but the helper's lists it, so it
/// never enters Brindle's build graph (ports R1, R3).
#[test]
fn only_the_acp_modules_name_the_schema_crate() {
    let acp = Path::new("remote-agent/src/acp");
    let mut files = vec![PathBuf::from("build.rs")];
    files.extend(sources("src"));
    files.extend(sources("tests"));
    for (dir, _) in CRATES.iter().filter(|(dir, _)| *dir != ".") {
        files.extend(sources(dir));
    }
    files.retain(|file| !file.starts_with(acp));
    assert!(files.iter().any(|file| file.starts_with("remote-agent/src")));
    assert_none(
        offenders(&files, Scope::All, &["agent_client_protocol_schema"]),
        "only remote-agent/src/acp/ decodes ACP (docs/architecture/ports-and-adapters.md R3)",
    );
    let listing: Vec<&str> = CRATES
        .iter()
        .filter(|(dir, _)| *dir != "remote-agent")
        .filter(|(dir, _)| dependencies(dir, &EVERY_KIND).contains("agent-client-protocol-schema"))
        .map(|(_, name)| *name)
        .collect();
    assert!(
        listing.is_empty(),
        "only brindle-remote-agent may depend on agent-client-protocol-schema \
         (docs/architecture/ports-and-adapters.md R3), but {listing:?} do"
    );
}

/// One module per ACP version (ports R14): each version's schema module is
/// named only under its own `acp/vN/`, `Dialect`'s variants are matched only
/// in `acp/dialect.rs`, and the reducer, the request bookkeeping and the
/// checkpoint name no version module.
#[test]
fn acp_versions_stay_in_their_modules() {
    let acp = Path::new("remote-agent/src/acp");
    let mut files = sources("remote-agent/src");
    files.extend(sources("remote-agent/tests"));
    assert!(files.iter().any(|file| file.starts_with(acp.join("v1"))));
    for version in ["v1", "v2"] {
        let home = acp.join(version);
        let outside: Vec<PathBuf> = files
            .iter()
            .filter(|file| !file.starts_with(&home))
            .cloned()
            .collect();
        assert_none(
            offenders(
                &outside,
                Scope::All,
                &[&format!("agent_client_protocol_schema::{version}")],
            ),
            &format!(
                "only remote-agent/src/acp/{version}/ names ACP {version}'s schema module \
                 (docs/architecture/ports-and-adapters.md R14)"
            ),
        );
    }
    let dialect = acp.join("dialect.rs");
    assert!(files.contains(&dialect), "{} is missing", dialect.display());
    let others: Vec<PathBuf> = files.iter().filter(|file| **file != dialect).cloned().collect();
    assert_none(
        offenders(&others, Scope::All, &["Dialect::V"]),
        "Dialect's variants are matched only in remote-agent/src/acp/dialect.rs; \
         call its methods (docs/architecture/ports-and-adapters.md R14)",
    );
    let versionless: Vec<PathBuf> = ["reduce.rs", "rpc.rs", "checkpoint.rs"]
        .iter()
        .map(|file| acp.join(file))
        .collect();
    assert_none(
        offenders(&versionless, Scope::All, &["v1::", "v2::"]),
        "the reducer, rpc and the checkpoint read acp/forms.rs, never a version's \
         module (docs/architecture/ports-and-adapters.md R14)",
    );
}

/// The transcript crate's one door in the app: no file in `src/` but
/// `src/agent/transcript.rs` names `brindle_transcript`, so the app reaches
/// the transcript's types through `crate::agent::transcript` (ports R3).
#[test]
fn only_the_agent_transcript_module_names_the_transcript_crate() {
    let door = Path::new("src/agent/transcript.rs");
    let mut files = sources("src");
    assert!(files.iter().any(|file| file == door), "{} is missing", door.display());
    files.retain(|file| file != door);
    assert_none(
        offenders(&files, Scope::All, &["brindle_transcript"]),
        "the app names brindle_transcript only in src/agent/transcript.rs; use \
         crate::agent::transcript (docs/architecture/ports-and-adapters.md R3)",
    );
}

/// The Markdown parser's one door: no source in `src/` or `tests/` but
/// `src/agent/markdown.rs` names `pulldown_cmark`, so the rest of the app
/// sees only `brindle-ui`'s rich-text data and a parser swap touches one
/// file (ports R3, fcis's "Whole pure modules").
#[test]
fn only_the_markdown_module_names_the_parser() {
    let door = Path::new("src/agent/markdown.rs");
    let mut files = sources("src");
    assert!(files.iter().any(|file| file == door), "{} is missing", door.display());
    files.extend(sources("tests"));
    files.retain(|file| file != door);
    assert_none(
        offenders(&files, Scope::All, &["pulldown_cmark"]),
        "the app names pulldown_cmark only in src/agent/markdown.rs; render Markdown through \
         crate::agent::markdown (docs/architecture/functional-core-imperative-shell.md)",
    );
}

/// The line diff's one door in the helper (ports R3): no source but
/// `remote-agent/src/acp/diff.rs` names `similar`, and the diff itself sets
/// no deadline or timeout, so the same texts always give the same hunks
/// (`agents-tool-content` decision 4).
#[test]
fn only_the_diff_module_names_similar() {
    let door = Path::new("remote-agent/src/acp/diff.rs");
    let mut files = sources("remote-agent/src");
    assert!(
        files.iter().any(|file| file == door),
        "{} is missing",
        door.display()
    );
    files.extend(sources("remote-agent/tests"));
    files.extend(sources("src"));
    files.retain(|file| file != door);
    assert_none(
        offenders(&files, Scope::All, &["similar"]),
        "only remote-agent/src/acp/diff.rs names similar \
         (docs/architecture/ports-and-adapters.md R3)",
    );
    assert_none(
        offenders(&[door.to_path_buf()], Scope::All, &["deadline", "timeout"]),
        "acp/diff.rs diffs with no deadline or timeout, so its hunks are deterministic \
         (agents-tool-content decision 4)",
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

/// The opt-in suites (`testing R3`), by `[[test]]` target name.
const OPT_IN_SUITES: [&str; 2] = ["e2e", "perf"];

/// The root manifest's `[[test]]` target named `suite`.
fn test_target<'a>(manifest: &'a toml::Table, suite: &str) -> &'a toml::Value {
    manifest["test"]
        .as_array()
        .expect("[[test]] targets")
        .iter()
        .find(|test| test.get("name").and_then(|name| name.as_str()) == Some(suite))
        .unwrap_or_else(|| panic!("no [[test]] target named {suite}"))
}

/// testing R3: the opt-in suites keep their own runner.
#[test]
fn the_opt_in_suites_are_harness_false_targets() {
    let manifest = manifest(".");
    for suite in OPT_IN_SUITES {
        assert_eq!(
            test_target(&manifest, suite)
                .get("harness")
                .and_then(|harness| harness.as_bool()),
            Some(false),
            "{suite} must be a harness = false target"
        );
    }
}

/// Whether `text` has `literal` as a string literal in code, not in a
/// comment.
fn literal_in_code(text: &str, literal: &str) -> bool {
    let code = code_of(text);
    text.match_indices(literal)
        .any(|(at, _)| code.as_bytes()[at] == b'"')
}

/// The shared parser of the runners' arguments (`tests/common/mod.rs`): the
/// one place that reads `--shard`.
const SHARED_PARSER: &str = "Selection::parse";

/// What `testing R3` asks of an opt-in suite's runner that its source
/// `text` doesn't name in code (not in a comment): the variable that
/// enables it (`BRINDLE_<SUITE>`, a string literal), `--list`,
/// `--self-test` and `--shard` (string literals too, the last also through
/// [`SHARED_PARSER`], which holds the literal for both runners).
fn runner_gaps(suite: &str, text: &str) -> Vec<&'static str> {
    let variable = format!("\"BRINDLE_{}\"", suite.to_uppercase());
    let shared = names_path(&code_of(text), SHARED_PARSER);
    [
        (variable.as_str(), "its variable, BRINDLE_<SUITE>", false),
        ("\"--list\"", "--list", false),
        ("\"--self-test\"", "--self-test", false),
        ("\"--shard\"", "--shard", shared),
    ]
    .into_iter()
    .filter(|(literal, _, instead)| !instead && !literal_in_code(text, literal))
    .map(|(_, gap, _)| gap)
    .collect()
}

/// testing R3: each opt-in suite's runner has its enabling variable,
/// `--list`, `--shard` and a `--self-test` (which holds the harness's own
/// pure helpers, `fcis R8` binding production code only).
#[test]
fn each_opt_in_runner_has_its_variable_list_and_self_test() {
    let manifest = manifest(".");
    for suite in OPT_IN_SUITES {
        let path = test_target(&manifest, suite)
            .get("path")
            .and_then(|path| path.as_str())
            .unwrap_or_else(|| panic!("the {suite} target names its runner's path"));
        let file = root().join(path);
        let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let gaps = runner_gaps(suite, &text);
        assert!(
            gaps.is_empty(),
            "{path}, the {suite} runner, lacks {gaps:?} \
             (docs/architecture/testing-architecture.md R3)"
        );
    }
    let common = root().join("tests/common/mod.rs");
    let text = fs::read_to_string(&common).unwrap_or_else(|e| panic!("{}: {e}", common.display()));
    assert!(
        literal_in_code(&text, "\"--shard\""),
        "tests/common/mod.rs, the runners' shared parser, no longer reads --shard \
         (docs/architecture/testing-architecture.md R3)"
    );
}

/// The runner check: a runner missing a feature is found, and one written
/// only in a comment doesn't count.
#[test]
fn a_runner_without_a_self_test_is_found() {
    let runner = r#"//! BRINDLE_SOAK=1 cargo test --test soak -- --self-test --shard 1/2
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // "--self-test" comes later.
    if std::env::var("BRINDLE_SOAK").as_deref() != Ok("1") {
        return;
    }
    if args.iter().any(|a| a == "--list") {
        list();
    }
    // Selection::parse(&args) comes later too.
}
"#;
    assert_eq!(runner_gaps("soak", runner), ["--self-test", "--shard"]);
    assert_eq!(
        runner_gaps("other", runner),
        ["its variable, BRINDLE_<SUITE>", "--self-test", "--shard"]
    );
}

/// A runner that parses `--shard` itself, or through the shared
/// `Selection::parse`, has it; one that only mentions it in a comment or a
/// usage line doesn't.
#[test]
fn a_runner_without_shard_is_found() {
    let complete = |shard: &str| {
        format!(
            r#"//! --shard K/N
fn main() {{
    let args: Vec<String> = std::env::args().skip(1).collect();
    eprintln!("usage: [--shard K/N]");
    if std::env::var("BRINDLE_SOAK").as_deref() != Ok("1") {{
        return;
    }}
    if args.iter().any(|a| a == "--self-test" || a == "--list") {{
        self_test();
    }}
    {shard}
}}
"#
        )
    };
    assert_eq!(runner_gaps("soak", &complete("")), ["--shard"]);
    assert!(runner_gaps("soak", &complete("let s = Selection::parse(&args);")).is_empty());
    assert!(runner_gaps("soak", &complete("let on = args.iter().any(|a| a == \"--shard\");")).is_empty());
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

/// The needles follow `use` items: grouped, nested over several lines, and
/// renamed. An import of a needle is itself a site.
#[test]
fn a_needle_follows_grouped_nested_and_renamed_imports() {
    let text = "use std::{fs, env};
use std::{
    time::{Instant, SystemTime},
    env as vars,
};
use std::time::Instant as Clock;
pub(crate) use std::process::*;
fn f() {
    let now = Clock::now();
    let id = id();
}
";
    let imports = use_paths(text);
    let import = |local: &str, path: &str| Import {
        local: local.to_string(),
        path: path.to_string(),
    };
    assert_eq!(
        imports,
        [
            import("fs", "std::fs"),
            import("env", "std::env"),
            import("Instant", "std::time::Instant"),
            import("SystemTime", "std::time::SystemTime"),
            import("vars", "std::env"),
            import("Clock", "std::time::Instant"),
            import("*", "std::process"),
        ]
    );
    assert_eq!(
        resolved_paths("let now = Clock::now();", &imports),
        ["std::time::Instant::now"]
    );
    assert_eq!(
        resolved_paths("vars::var(name)", &imports),
        ["std::env::var"]
    );
    let lines = numbered(&code_of(text));
    assert_eq!(needle_lines(&lines, &["std::env"]), [1, 2]);
    assert_eq!(needle_lines(&lines, &["std::fs"]), [1]);
    assert_eq!(needle_lines(&lines, &["std::time::Instant::now"]), [9]);
    assert_eq!(needle_lines(&lines, &["Instant::now"]), [9]);
    assert_eq!(needle_lines(&lines, &["std::process"]), [7]);
}

/// A path written in full is a site without any import, rooted or not.
#[test]
fn a_needle_matches_a_path_written_in_full() {
    let text = "fn f() {
    let started = std::time::SystemTime::now();
    let home = ::std::env::var(\"HOME\");
}
";
    assert_eq!(
        resolved_paths("let home = ::std::env::var(\"HOME\");", &[]),
        ["std::env::var"]
    );
    let lines = numbered(&code_of(text));
    assert_eq!(needle_lines(&lines, &["std::time::SystemTime::now"]), [2]);
    assert_eq!(needle_lines(&lines, &["SystemTime::now"]), [2]);
    assert_eq!(needle_lines(&lines, &["std::env"]), [3]);
}

/// A needle is a whole path segment: not the head of a longer name, not
/// the tail of one, and not in a comment.
#[test]
fn a_needle_does_not_match_a_longer_name() {
    let text = "use std::environment;
use crate::terminal::process;
fn f() {
    prefs::load();
    process::name();
    // use std::env;
    /* use std::fs; */
    let path = \"std::fs\";
}
";
    let lines = numbered(&code_of(text));
    assert_eq!(needle_lines(&lines, &["std::env"]), Vec::<usize>::new());
    assert_eq!(
        needle_lines(&lines, &["fs::", "std::fs"]),
        Vec::<usize>::new()
    );
    assert_eq!(needle_lines(&lines, &["std::process"]), Vec::<usize>::new());
    assert!(!names_path("std::environment", "std::env"));
    assert!(!names_path("Instant::now_or_never()", "Instant::now"));
}

// fcis R2, R3: the whole pure modules read nothing ambient.

/// The whole pure modules: the files of `fcis`'s "Whole pure modules"
/// table (`docs/architecture/functional-core-imperative-shell.md`), whose
/// term-core row `term_core_does_no_io_and_reads_no_clock` covers. The pure
/// items in shell modules are review's. This mirrors that table and changes
/// with it, in the same commit. `the_neutral_code_is_never_gated` holds the
/// same files to `fcis R7`.
const PURE_MODULES: [&str; 65] = [
    "term-core/src/progress.rs",
    "term-core/src/context.rs",
    "term-core/src/pointer.rs",
    "term-core/src/kitty.rs",
    "src/notify/kitty.rs",
    "src/terminal/keys.rs",
    "src/terminal/mouse.rs",
    "src/terminal/pointer.rs",
    "src/terminal/cmdline.rs",
    "src/terminal/dir_check.rs",
    "src/terminal/feedback.rs",
    "src/notify/policy.rs",
    "src/agent/transcript.rs",
    "src/agent/spawn.rs",
    "src/agent/close.rs",
    "src/remote/local/endpoint_env.rs",
    "src/agent/login.rs",
    "src/agent/shared.rs",
    "src/agent/markdown.rs",
    "src/agent/model.rs",
    "src/layout.rs",
    "src/reattach.rs",
    "src/workspace/tabs.rs",
    "src/workspace/launch.rs",
    "src/workspace/overlays.rs",
    "src/workspace/previews.rs",
    "src/workspace/notices.rs",
    "src/workspace/mark.rs",
    "src/picker/sessions.rs",
    "src/settings_dialog/themes.rs",
    "ui/src/filter.rs",
    "ui/src/primitives/placement.rs",
    "remote-protocol/src/client.rs",
    "remote-protocol/src/holder.rs",
    "remote-protocol/src/wire.rs",
    "remote-protocol/src/version.rs",
    "remote-protocol/src/relay.rs",
    "remote-protocol/src/upgrade.rs",
    "remote-protocol/src/access.rs",
    "remote-protocol/src/hresult.rs",
    "remote-agent/src/conpty/cmdline.rs",
    "remote-agent/src/server/platform/ready_arg.rs",
    "remote-agent/src/server/platform/ensure_dir.rs",
    "remote-agent/src/server/platform/pipe_state.rs",
    "remote-agent/src/server/platform/wait.rs",
    "remote-agent/src/server/platform/piped.rs",
    "remote-agent/src/conpty/choice.rs",
    "remote-agent/src/conpty/hosts.rs",
    "remote-agent/src/pty/program.rs",
    "remote-agent/src/pty/stat.rs",
    "remote-agent/src/server/core/policy.rs",
    "remote-agent/src/server/holder/policy.rs",
    "remote-agent/src/server/login_env.rs",
    "remote-agent/src/acp/envelope.rs",
    "remote-agent/src/acp/forms.rs",
    "remote-agent/src/acp/dialect.rs",
    "remote-agent/src/acp/v1/decode.rs",
    "remote-agent/src/acp/v1/encode.rs",
    "remote-agent/src/acp/diff.rs",
    "remote-agent/src/acp/plain.rs",
    "remote-agent/src/acp/lines.rs",
    "remote-agent/src/acp/rpc.rs",
    "remote-agent/src/acp/reduce.rs",
    "remote-agent/src/acp/checkpoint.rs",
    "remote-agent/src/fake_acp/script.rs",
];

/// The type names that make a `static` mutable state (a name containing
/// one counts: `RefCell`, `AtomicU64`).
const MUTABLE_STATICS: [&str; 6] = ["OnceLock", "LazyLock", "Mutex", "RwLock", "Cell", "Atomic"];

/// File-system calls a pure module never makes: a probe is passed in
/// (`fcis R5`), never called as a method.
const FS_METHODS: [&str; 9] = [
    ".is_file()",
    ".is_dir()",
    ".exists()",
    ".try_exists(",
    ".metadata()",
    ".symlink_metadata()",
    ".canonicalize()",
    ".read_dir()",
    ".read_link()",
];

/// The numbers of the lines among `lines` (numbered code) that declare
/// mutable state: a `static mut`, a `static` whose type names one of
/// `MUTABLE_STATICS`, or `thread_local!`. A `'static` lifetime isn't one.
fn mutable_static_lines(lines: &[(usize, String)]) -> Vec<usize> {
    let code = lines
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = BTreeSet::new();
    for (at, _) in code.match_indices("static") {
        let Some(after) = keyword(&code, at, "static") else {
            continue;
        };
        if code[..at].ends_with(|c: char| is_word(c) || c == '\'') {
            continue;
        }
        let rest = &code[after..];
        let declaration = &rest[..rest.find(['=', ';']).unwrap_or(rest.len())];
        let mutable = keyword(declaration, skip_space(declaration, 0), "mut").is_some()
            || declaration
                .split_once(':')
                .is_some_and(|(_, kind)| MUTABLE_STATICS.iter().any(|name| kind.contains(name)));
        if mutable {
            found.insert(code[..at].matches('\n').count());
        }
    }
    for (at, _) in code.match_indices("thread_local!") {
        if !code[..at].ends_with(is_word) {
            found.insert(code[..at].matches('\n').count());
        }
    }
    found.into_iter().map(|index| lines[index].0).collect()
}

/// The numbers of the lines of `text`'s production code ([`production_lines`])
/// that read something ambient (`fcis R2`, `R3`): the IO and clock needles,
/// imports followed ([`needle_lines`]), a mutable static, or a file-system
/// method (`FS_METHODS`).
fn ambient_reads(text: &str) -> Vec<usize> {
    let lines = production_lines(text);
    let needles = [&IO[..], &CLOCK_AND_THREADS[..]].concat();
    let mut found: BTreeSet<usize> = needle_lines(&lines, &needles).into_iter().collect();
    found.extend(mutable_static_lines(&lines));
    found.extend(
        lines
            .iter()
            .filter(|(_, line)| FS_METHODS.iter().any(|method| line.contains(method)))
            .map(|(number, _)| *number),
    );
    found.into_iter().collect()
}

/// fcis R2, R3: a whole pure module's production code reads no clock,
/// environment, file, process, thread, network or mutable static. Its
/// tests may (`fcis R8` asks only that they need none).
#[test]
fn the_pure_modules_read_nothing_ambient() {
    let mut found = Vec::new();
    for file in PURE_MODULES {
        let path = root().join(file);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let original: Vec<&str> = text.lines().collect();
        found.extend(
            ambient_reads(&text)
                .into_iter()
                .map(|number| format!("{file}:{number}: {}", original[number - 1].trim())),
        );
    }
    assert_none(
        found,
        "a whole pure module takes time, the environment and facts about the system as \
         arguments (docs/architecture/functional-core-imperative-shell.md R2, R3, R5): \
         read them in the shell and pass them in, or pass a probe",
    );
}

/// fcis: every whole pure module is a file there is.
#[test]
fn every_pure_module_exists() {
    let missing: Vec<&str> = PURE_MODULES
        .into_iter()
        .filter(|file| !root().join(file).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "PURE_MODULES names files that don't exist; change it with fcis's \
         \"Whole pure modules\" table: {missing:?}"
    );
}

/// `path` with each `{a,b}` group expanded, in order.
fn expand_braces(path: &str) -> Vec<String> {
    let Some((head, rest)) = path.split_once('{') else {
        return vec![path.to_string()];
    };
    let Some((group, tail)) = rest.split_once('}') else {
        return vec![path.to_string()];
    };
    group
        .split(',')
        .flat_map(|choice| expand_braces(&format!("{head}{}{tail}", choice.trim())))
        .collect()
}

/// The files of `fcis`'s "Whole pure modules" table: the backticked `.rs`
/// paths in its first column, `{a,b}` expanded. The term-core row names a
/// crate, not a file, so it gives none.
fn fcis_whole_modules(doc: &str) -> BTreeSet<String> {
    doc.lines()
        .skip_while(|line| !line.starts_with("### Whole pure modules"))
        .skip(1)
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .skip(2)
        .filter_map(|row| row.split('|').nth(1))
        .flat_map(|cell| {
            cell.split('`')
                .skip(1)
                .step_by(2)
                .flat_map(expand_braces)
                .collect::<Vec<_>>()
        })
        .filter(|path| path.ends_with(".rs"))
        .collect()
}

/// fcis: `PURE_MODULES` and the doc's table list the same files.
#[test]
fn the_pure_module_list_matches_the_fcis_table() {
    let path = root().join("docs/architecture/functional-core-imperative-shell.md");
    let doc = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        fcis_whole_modules(&doc),
        PURE_MODULES.iter().map(|file| file.to_string()).collect(),
        "fcis's \"Whole pure modules\" table and PURE_MODULES must list the same files"
    );
}

/// The pure-module check finds each kind of ambient read in production
/// code, and nothing in test code.
#[test]
fn a_pure_module_reading_the_world_is_found() {
    let text = "use std::{env, fs};
use std::time::Instant as Clock;
static ID: OnceLock<String> = OnceLock::new();
fn candidate(path: &Path) -> bool {
    path.is_file()
}
fn started() -> SystemTime {
    std::time::SystemTime::now()
}
fn elapsed(since: Instant) -> Duration {
    Clock::now() - since
}
static NAMES: &[&'static str] = &[\"static mut\"];
fn probe(is_file: impl Fn(&Path) -> bool, path: &Path) -> bool {
    is_file(path)
}
#[cfg(test)]
mod tests {
    use std::time::Instant;
    static LOCK: Mutex<()> = Mutex::new(());
    #[test]
    fn t() {
        let _ = Instant::now();
    }
}
";
    assert_eq!(ambient_reads(text), [1, 3, 5, 8, 11]);
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
    // showing a frame's pointer at once
    "src/cursor.rs",
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
const R4_EXCEPTION: [&str; 15] = [
    "src/instance.rs",
    "src/main.rs",
    "src/notify/mod.rs",
    "src/remote/local.rs",
    "src/remote/mod.rs",
    "src/session.rs",
    "src/settings.rs",
    "src/terminal/mod.rs",
    "src/terminal_view.rs",
    "src/workspace.rs",
    "src/zone_view.rs",
    "remote-agent/src/main.rs",
    "remote-agent/src/server/cli.rs",
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

/// `code[range]` as spaces, newlines kept.
fn blank(code: &mut [u8], range: Range<usize>) {
    for byte in &mut code[range] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// `text` with its comments and the insides of its string and character
/// literals blanked (newlines kept), so that only code is searched while
/// every offset and line number still matches `text`. Quotes stay, so a
/// `#[path = "…"]` can be read back from `text`.
fn code_of(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut code = bytes.to_vec();
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

/// The parts of `text` between commas outside brackets (a `cfg_attr`'s
/// arguments, a `use` group's trees), trimmed, without empty ones.
fn arguments(text: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0, 0);
    for (at, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
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

/// The outer attributes starting at `at` (on one item), and the offset of
/// that item.
fn outer_attributes(code: &str, first: Attribute) -> (Vec<Attribute>, usize) {
    let mut item = skip_space(code, first.end);
    let mut attributes = vec![first];
    while let Some(next) = attribute_at(code, item).filter(|next| !next.inner) {
        item = skip_space(code, next.end);
        attributes.push(next);
    }
    (attributes, item)
}

/// A file's production code, as `ports R4` and `fcis R8` read it.
struct Production {
    /// The file's code ([`code_of`]) with its test code blanked too, so that
    /// offsets and line numbers still match the text.
    code: String,
    /// The `mod NAME;` declarations in its test code, each with its
    /// `#[path]`: their files are test code too.
    test_modules: Vec<(String, Option<String>)>,
}

/// `text`'s production code. Test code is an item under a test-only `cfg`
/// (`#[cfg(test)]`, `#[cfg(all(test, …))]`), read to its `;` or to the
/// brace that closes its first `{`, and everything after an inner
/// `#![cfg(test)]`.
fn production(text: &str) -> Production {
    let mut code = code_of(text);
    let (mut tests, mut test_modules) = (Vec::new(), Vec::new());
    let mut at = 0;
    while at < code.len() {
        if let Some(attribute) = attribute_at(&code, at).filter(|attribute| attribute.inner) {
            match meta(&code, text, &attribute) {
                Meta::Cfg(cfg) if predicate(cfg) == Predicate::TestOnly => {
                    // The rest of the module is test code.
                    tests.push(attribute.start..code.len());
                    break;
                }
                Meta::Cfg(_) | Meta::CfgAttr(..) | Meta::Path(_) | Meta::Other => {}
            }
            at = attribute.end;
        } else if let Some(first) = attribute_at(&code, at) {
            let start = first.start;
            let (attributes, item) = outer_attributes(&code, first);
            let (mut test, mut file) = (false, None);
            for attribute in &attributes {
                match meta(&code, text, attribute) {
                    Meta::Cfg(cfg) => test |= predicate(cfg) == Predicate::TestOnly,
                    Meta::Path(path) => file = Some(path.to_string()),
                    Meta::CfgAttr(..) | Meta::Other => {}
                }
            }
            if test {
                if let Some(name) = mod_declaration(&code, item) {
                    test_modules.push((name.to_string(), file));
                }
                at = item_end(&code, item);
                tests.push(start..at);
            } else {
                at = item;
            }
        } else {
            at += 1;
        }
    }
    let mut bytes = std::mem::take(&mut code).into_bytes();
    for range in tests {
        blank(&mut bytes, range);
    }
    Production {
        code: String::from_utf8(bytes).expect("blanking whole items keeps UTF-8"),
        test_modules,
    }
}

/// The production code lines of `text` ([`production`]), numbered from 1
/// and trimmed, without the empty ones.
fn production_lines(text: &str) -> Vec<(usize, String)> {
    numbered(&production(text).code)
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
/// - test code, as [`production`] finds it (a `mod NAME;` there is returned
///   as a test module, whose file is test code too);
/// - a seam's module declaration: a platform `cfg` on `mod NAME;`, among
///   any other attributes, whose file is in `seams`;
/// - a lint-only `cfg_attr`, which sets only `allow`, `expect`, `warn` or
///   `deny`.
///
/// Comments and the insides of literals are skipped. `files` are the files
/// there are, for finding a module's (`/`-separated, as `path` is).
fn platform_cfgs(path: &str, text: &str, seams: &[&str], files: &BTreeSet<String>) -> Scan {
    let Production { code, test_modules } = production(text);
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if let Some(attribute) = attribute_at(&code, at).filter(|attribute| attribute.inner) {
            match meta(&code, text, &attribute) {
                Meta::Cfg(cfg) => match predicate(cfg) {
                    Predicate::Platform => found.push(attribute.start),
                    // Test code is blanked already.
                    Predicate::TestOnly | Predicate::Neither => {}
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
            let (attributes, item) = outer_attributes(&code, first);
            let (mut gates, mut others, mut file) = (Vec::new(), Vec::new(), None);
            for attribute in &attributes {
                match meta(&code, text, attribute) {
                    Meta::Cfg(cfg) => match predicate(cfg) {
                        Predicate::Platform => gates.push(attribute.start),
                        Predicate::TestOnly | Predicate::Neither => {}
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
            if !module.is_some_and(|module| in_seams(&module, seams)) {
                found.extend(gates);
            }
            found.extend(others);
            at = item;
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
        test_modules: test_modules
            .iter()
            .map(|(name, file)| module_file(path, name, file.as_deref(), files))
            .collect(),
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

/// Each production file's platform `cfg`s outside the allowances, and the
/// files of the modules they declare under a test-only `cfg`.
fn scan_production() -> (BTreeMap<String, Vec<String>>, Vec<String>) {
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
    (sites, test_modules)
}

/// Each production file's platform `cfg`s outside the allowances, without
/// the files of test modules.
fn production_platform_cfgs() -> BTreeMap<String, Vec<String>> {
    let (mut sites, test_modules) = scan_production();
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
        "remote-agent/src/server/core/sessions.rs",
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

// fcis R7: neutral code, which the code of every platform calls, is never
// platform-gated, so it builds and is tested on every platform. It may sit
// in a seam (ports R4 allows a seam its `cfg`s), but no `cfg` reaches it.

/// Modules of no platform that aren't whole pure modules (they read the
/// world), so aren't in `PURE_MODULES`. Like the whole pure modules, they
/// are never platform-gated.
const NEUTRAL_MODULES: [&str; 1] = ["remote-protocol/src/server_dir.rs"];

/// Pure items in seam and shell files that one platform's code calls, as
/// (file, `fn` name), from `fcis`'s "Pure items in shell modules" table:
/// each is a `fn` at the top of its file with no platform `cfg` on it.
const NEUTRAL_ITEMS: [(&str, &str); 7] = [
    ("src/desktop.rs", "without_verbatim_prefix"),
    ("src/notify/mod.rs", "escape_markup"),
    ("src/notify/toast.rs", "xml"),
    ("src/notify/toast.rs", "action_key"),
    ("src/instance.rs", "socket_path_in"),
    ("src/instance.rs", "pipe_name_for"),
    ("src/workspace.rs", "program_title"),
];

/// The one gate a neutral module's ancestor may carry, as written without
/// whitespace: Brindle's two platforms together, on both of which a module
/// under it builds and is tested.
const BOTH_PLATFORMS: &str = r#"cfg(any(target_os="linux",windows))"#;

/// Whether `byte` can be part of a word (non-ASCII bytes counted in, so a
/// word never starts inside a character).
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii()
}

/// The outer attributes on each item in `code`, by the item's offset.
fn attributes_by_item(code: &str) -> BTreeMap<usize, Vec<Attribute>> {
    let mut found = BTreeMap::new();
    let mut at = 0;
    while at < code.len() {
        match attribute_at(code, at) {
            Some(attribute) if attribute.inner => at = attribute.end,
            Some(first) => {
                let (attributes, item) = outer_attributes(code, first);
                found.insert(item, attributes);
                at = item;
            }
            None => at += 1,
        }
    }
    found
}

/// The name of the `fn` item starting at `at` (after any visibility and
/// `const`, `async`, `unsafe` or `extern "ABI"`), or `None` for any other
/// item.
fn fn_item(code: &str, at: usize) -> Option<&str> {
    let mut at = skip_space(code, at);
    if let Some(end) = keyword(code, at, "pub") {
        at = skip_space(code, end);
        if code[at..].starts_with('(') {
            at = skip_space(code, closing(code, at)? + 1);
        }
    }
    while let Some(end) = ["const", "async", "unsafe", "extern"]
        .into_iter()
        .find_map(|word| keyword(code, at, word))
    {
        at = skip_space(code, end);
        if code[at..].starts_with('"') {
            at = skip_space(code, at + 1 + code[at + 1..].find('"')? + 1);
        }
    }
    let start = skip_space(code, keyword(code, at, "fn")?);
    let end = code[start..]
        .find(|c: char| !is_word(c))
        .map_or(code.len(), |n| start + n);
    (end > start).then(|| &code[start..end])
}

/// Each item of `code` that `item` recognises at a word's start, with its
/// offset and the name `item` gives it. The words up to that name belong
/// to the same item, so they aren't tried again.
fn items_named<'a>(
    code: &'a str,
    item: impl Fn(&'a str, usize) -> Option<&'a str>,
) -> Vec<(usize, &'a str)> {
    let bytes = code.as_bytes();
    let (mut found, mut next) = (Vec::new(), 0);
    for at in 0..bytes.len() {
        let starts_word = (bytes[at].is_ascii_alphabetic() || bytes[at] == b'_')
            && !at
                .checked_sub(1)
                .is_some_and(|before| is_word_byte(bytes[before]));
        if at < next || !starts_word {
            continue;
        }
        if let Some(name) = item(code, at) {
            found.push((at, name));
            next = name.as_ptr() as usize - code.as_ptr() as usize + name.len();
        }
    }
    found
}

/// `path:line: text` for `offset` in `text`, the form each check here
/// reports a site in.
fn site(path: &str, text: &str, offset: usize) -> String {
    let line = text[..offset].matches('\n').count();
    let shown = text.lines().nth(line).unwrap_or_default().trim();
    format!("{path}:{}: {shown}", line + 1)
}

/// Whether `offset` starts a line of `code`: an item there is at the top of
/// its file, not inside an inline module.
fn at_column_zero(code: &str, offset: usize) -> bool {
    offset == 0 || code.as_bytes()[offset - 1] == b'\n'
}

/// Whether `attribute` gates what it is on by platform: a `cfg` that names
/// one, or a `cfg_attr` that names one and does more than set lint levels.
fn gates(code: &str, text: &str, attribute: &Attribute) -> bool {
    match meta(code, text, attribute) {
        Meta::Cfg(cfg) => predicate(cfg) == Predicate::Platform,
        Meta::CfgAttr(cfg, set) => predicate(cfg) == Predicate::Platform && !lint_only(&set),
        Meta::Path(_) | Meta::Other => false,
    }
}

/// `attribute` as written in `text`, literals included, without whitespace.
fn written(text: &str, attribute: &Attribute) -> String {
    text[attribute.body.clone()]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// A `mod NAME;` declaration, as the never-gated check reads it.
struct Declaration {
    /// The file it is in.
    declaring: String,
    /// The platform gates among its attributes ([`gates`]), each as its
    /// site and as written ([`written`]).
    gates: Vec<(String, String)>,
    /// Its site when it isn't at the top of its file: inside an inline
    /// module, whose gates and path this check doesn't follow.
    nested: Option<String>,
}

/// Every `mod NAME;` in the production code of `texts` (each file's text by
/// its path), by the file it declares ([`module_file`]).
fn module_declarations(texts: &BTreeMap<String, String>) -> BTreeMap<String, Vec<Declaration>> {
    let files: BTreeSet<String> = texts.keys().cloned().collect();
    let mut found: BTreeMap<String, Vec<Declaration>> = BTreeMap::new();
    for (path, text) in texts {
        let code = production(text).code;
        let attributes = attributes_by_item(&code);
        for (at, name) in items_named(&code, mod_declaration) {
            let on_item = attributes.get(&at).map_or(&[][..], Vec::as_slice);
            let start = on_item.first().map_or(at, |attribute| attribute.start);
            let file = on_item
                .iter()
                .find_map(|attribute| match meta(&code, text, attribute) {
                    Meta::Path(file) => Some(file),
                    Meta::Cfg(_) | Meta::CfgAttr(..) | Meta::Other => None,
                });
            found
                .entry(module_file(path, name, file, &files))
                .or_default()
                .push(Declaration {
                    declaring: path.clone(),
                    gates: on_item
                        .iter()
                        .filter(|attribute| gates(&code, text, attribute))
                        .map(|attribute| {
                            (site(path, text, attribute.start), written(text, attribute))
                        })
                        .collect(),
                    nested: (!at_column_zero(&code, start)).then(|| site(path, text, at)),
                });
        }
    }
    found
}

/// Whether `file` is a crate's root, which nothing declares.
fn is_crate_root(file: &str) -> bool {
    matches!(file.rsplit('/').next(), Some("lib.rs" | "main.rs"))
}

/// Where the neutral `modules` are platform-gated (`fcis R7`), as
/// `path:line: text`, sorted, given every production file's text by its
/// path: a platform `cfg` in a module's production code (a lint-only
/// `cfg_attr` aside), any platform gate on its own `mod` declaration, and
/// one on an ancestor's other than [`BOTH_PLATFORMS`]. A module that no
/// declaration at the top of a file declares is reported too, since the
/// check can't follow it.
fn gated_neutral_modules(modules: &[&str], texts: &BTreeMap<String, String>) -> Vec<String> {
    let files: BTreeSet<String> = texts.keys().cloned().collect();
    let declarations = module_declarations(texts);
    let mut found = BTreeSet::new();
    for module in modules {
        match texts.get(*module) {
            Some(text) => found.extend(platform_cfgs(module, text, &[], &files).sites),
            None => {
                found.insert(format!("{module}: not a production file"));
            }
        }
        // Its own declaration first, then each ancestor's.
        let (mut pending, mut seen) = (vec![(module.to_string(), true)], BTreeSet::new());
        while let Some((file, own)) = pending.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            let Some(declared) = declarations.get(&file) else {
                if !is_crate_root(&file) {
                    found.insert(format!(
                        "{file}: no `mod` declaration at the top of its parent declares it"
                    ));
                }
                continue;
            };
            for declaration in declared {
                found.extend(declaration.nested.clone());
                found.extend(
                    declaration
                        .gates
                        .iter()
                        .filter(|(_, written)| own || written != BOTH_PLATFORMS)
                        .map(|(site, _)| site.clone()),
                );
                pending.push((declaration.declaring.clone(), false));
            }
        }
    }
    found.into_iter().collect()
}

/// Where the neutral item `fn name` in `text` (the file at `path`) is
/// gated, as `path:line: text`: each production `fn` of that name that
/// isn't at the top of the file (it is inside an inline module), and each
/// platform gate ([`gates`]) on it.
fn gated_neutral_item(path: &str, text: &str, name: &str) -> Vec<String> {
    let code = production(text).code;
    let attributes = attributes_by_item(&code);
    let mut found = Vec::new();
    for (at, _) in items_named(&code, fn_item)
        .into_iter()
        .filter(|(_, found)| *found == name)
    {
        let on_item = attributes.get(&at).map_or(&[][..], Vec::as_slice);
        let start = on_item.first().map_or(at, |attribute| attribute.start);
        if !at_column_zero(&code, start) {
            found.push(site(path, text, at));
        }
        found.extend(
            on_item
                .iter()
                .filter(|attribute| gates(&code, text, attribute))
                .map(|attribute| site(path, text, attribute.start)),
        );
    }
    found
}

/// Every production file's text, by its path ([`production_files`]).
fn production_texts() -> BTreeMap<String, String> {
    production_files()
        .into_iter()
        .map(|file| {
            let path = root().join(&file);
            let text =
                fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (file, text)
        })
        .collect()
}

/// fcis R7: the whole pure modules, `NEUTRAL_MODULES` and `NEUTRAL_ITEMS`
/// are never platform-gated, so they build and are tested on every
/// platform, whichever platform's code calls them.
#[test]
fn the_neutral_code_is_never_gated() {
    let texts = production_texts();
    let modules: Vec<&str> = PURE_MODULES.into_iter().chain(NEUTRAL_MODULES).collect();
    let mut found = gated_neutral_modules(&modules, &texts);
    for (file, name) in NEUTRAL_ITEMS {
        let text = texts
            .get(file)
            .unwrap_or_else(|| panic!("{file} is not a production file"));
        found.extend(gated_neutral_item(file, text, name));
    }
    assert_none(
        found,
        "neutral code builds and is tested on every platform, so no platform `cfg` reaches it \
         (docs/architecture/functional-core-imperative-shell.md R7): none in a whole pure module's or \
         NEUTRAL_MODULES' production code (a lint-only `cfg_attr` aside), none on its `mod` declaration, \
         and none on an ancestor's but `cfg(any(target_os = \"linux\", windows))`; each NEUTRAL_ITEMS `fn` \
         is at the top of its file with none on it. Keep the platform part in the seam that calls it",
    );
}

/// fcis R7: `NEUTRAL_MODULES` names files there are, and `NEUTRAL_ITEMS`
/// functions in their files' production code.
#[test]
fn every_neutral_module_and_item_exists() {
    let mut missing: Vec<String> = NEUTRAL_MODULES
        .into_iter()
        .filter(|file| !root().join(file).is_file())
        .map(String::from)
        .collect();
    missing.extend(
        NEUTRAL_ITEMS
            .into_iter()
            .filter(|(file, name)| {
                fs::read_to_string(root().join(file)).map_or(true, |text| {
                    let code = production(&text).code;
                    !items_named(&code, fn_item)
                        .iter()
                        .any(|(_, found)| found == name)
                })
            })
            .map(|(file, name)| format!("{file}: fn {name}")),
    );
    assert!(
        missing.is_empty(),
        "NEUTRAL_MODULES and NEUTRAL_ITEMS name modules or functions that don't exist; change them \
         with fcis's tables (docs/architecture/functional-core-imperative-shell.md): {missing:?}"
    );
}

/// `texts` as a map of owned paths and texts, for [`gated_neutral_modules`].
fn texts(files: &[(&str, &str)]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect()
}

/// The never-gated check finds a platform `cfg` in a neutral module's
/// production code, on its own declaration, and on an ancestor's, where
/// only Brindle's two platforms together are allowed.
#[test]
fn a_gated_neutral_module_is_found() {
    let texts = texts(&[
        (
            "a/lib.rs",
            "#[cfg(windows)]\nmod gated;\n#[cfg(any(target_os = \"linux\", windows))]\nmod both;\n",
        ),
        ("a/gated.rs", "pub mod inner;\n"),
        ("a/gated/inner.rs", "pub fn f() {}\n"),
        ("a/both.rs", "#[cfg(unix)]\nmod x;\npub mod y;\n"),
        ("a/both/x.rs", "pub fn f() {}\n"),
        ("a/both/y.rs", "#[cfg(windows)]\nfn f() {}\npub fn g() {}\n"),
        ("a/stray.rs", "pub fn f() {}\n"),
    ]);
    assert_eq!(
        gated_neutral_modules(&["a/both/y.rs"], &texts),
        ["a/both/y.rs:1: #[cfg(windows)]"]
    );
    assert_eq!(
        gated_neutral_modules(&["a/both/x.rs"], &texts),
        ["a/both.rs:1: #[cfg(unix)]"]
    );
    assert_eq!(
        gated_neutral_modules(&["a/gated/inner.rs"], &texts),
        ["a/lib.rs:1: #[cfg(windows)]"]
    );
    // Both platforms together are an ancestor's allowance, not the module's;
    // and a module it declares under a platform `cfg` is in its code.
    assert_eq!(
        gated_neutral_modules(&["a/both.rs"], &texts),
        [
            "a/both.rs:1: #[cfg(unix)]",
            r#"a/lib.rs:3: #[cfg(any(target_os = "linux", windows))]"#
        ]
    );
    assert_eq!(
        gated_neutral_modules(&["a/stray.rs"], &texts),
        ["a/stray.rs: no `mod` declaration at the top of its parent declares it"]
    );
}

/// The never-gated check finds a neutral item inside an inline module (its
/// `fn` isn't at column 0) and one under a platform `cfg`.
#[test]
fn a_gated_neutral_item_is_found() {
    let text = "#[cfg(windows)]
mod imp {
    pub fn inside() {}
}
#[cfg(unix)]
pub(crate) fn under() {}
pub fn plain() {}
";
    assert_eq!(
        gated_neutral_item("a/lib.rs", text, "inside"),
        ["a/lib.rs:3: pub fn inside() {}"]
    );
    assert_eq!(
        gated_neutral_item("a/lib.rs", text, "under"),
        ["a/lib.rs:5: #[cfg(unix)]"]
    );
    assert_eq!(
        gated_neutral_item("a/lib.rs", text, "plain"),
        Vec::<String>::new()
    );
}

/// A lint-only `cfg_attr`, on a declaration, an item or in a module's code,
/// and platform `cfg`s in test code don't count as gates.
#[test]
fn a_lint_only_cfg_attr_and_test_code_do_not_count() {
    let module = "#[cfg_attr(windows, allow(dead_code))]
pub fn f() {}
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = \"only Windows calls it\")
)]
pub(crate) fn g() {}
#[cfg(all(test, windows))]
fn helper() {}
#[cfg(test)]
mod tests {
    #[cfg(windows)]
    fn g() {}
    #[test]
    fn t() {
        if cfg!(unix) {}
    }
}
";
    let texts = texts(&[
        (
            "a/lib.rs",
            "#[cfg_attr(not(windows), allow(dead_code))]\nmod m;\n#[cfg(all(test, unix))]\nmod helpers;\n",
        ),
        ("a/m.rs", module),
    ]);
    assert_eq!(
        gated_neutral_modules(&["a/m.rs"], &texts),
        Vec::<String>::new()
    );
    assert_eq!(
        gated_neutral_item("a/m.rs", module, "g"),
        Vec::<String>::new()
    );
}

/// The lints a crate root must switch on, as `(root file, lints)`: the
/// wildcard lint on every crate, and the panicking-call lints on the three
/// crates that decode what comes from outside the process.
const ROOT_LINTS: [(&str, &[&str]); 6] = [
    ("src/main.rs", &["clippy::wildcard_enum_match_arm"]),
    ("remote-agent/src/main.rs", &["clippy::wildcard_enum_match_arm"]),
    (
        "remote-protocol/src/lib.rs",
        &["clippy::wildcard_enum_match_arm", "clippy::unwrap_used", "clippy::expect_used"],
    ),
    (
        "term-core/src/lib.rs",
        &["clippy::wildcard_enum_match_arm", "clippy::unwrap_used", "clippy::expect_used"],
    ),
    (
        "transcript/src/lib.rs",
        &["clippy::wildcard_enum_match_arm", "clippy::unwrap_used", "clippy::expect_used"],
    ),
    ("ui/src/lib.rs",&["clippy::wildcard_enum_match_arm"]),
];

/// The lints of `wanted` that `root_text` doesn't switch on in a
/// `#![cfg_attr(not(test), warn(…))]` line. A text search: the lint is what
/// finds a violation, this only notices that a root stopped asking.
fn lints_missing_from(root_text: &str, wanted: &[&str]) -> Vec<String> {
    let attributes: Vec<&str> = root_text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("#![cfg_attr(not(test), warn("))
        .collect();
    wanted
        .iter()
        .filter(|lint| !attributes.iter().any(|line| line.contains(*lint)))
        .map(|lint| lint.to_string())
        .collect()
}

/// railway R3, R5, R9, types R2: clippy holds them in non-test code, so a
/// root that loses its attribute stops being checked without a failure.
#[test]
fn the_crate_roots_enable_the_restriction_lints() {
    assert_eq!(ROOT_LINTS.len(), CRATES.len(), "every crate's root is listed");
    let mut found = Vec::new();
    for (file, wanted) in ROOT_LINTS {
        let path = root().join(file);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for lint in lints_missing_from(&text, wanted) {
            found.push(format!("{file}: no `#![cfg_attr(not(test), warn({lint}))]`"));
        }
    }
    assert_none(
        found,
        "the crate roots switch on the lints that hold railway R3, R5 and R9 and types R2 \
         (docs/architecture/railway-oriented-programming.md, Enforcement)",
    );
}

/// The search itself: a root with the attribute passes, one without it is
/// reported.
#[test]
fn a_root_without_the_lint_is_reported() {
    let lint = ["clippy::wildcard_enum_match_arm"];
    let with = "//! docs\n#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]\nmod a;\n";
    let without = "//! docs\n#![warn(clippy::wildcard_enum_match_arm)]\nmod a;\n";
    let also = "#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]\n";
    assert!(lints_missing_from(with, &lint).is_empty());
    assert_eq!(lints_missing_from(without, &lint), ["clippy::wildcard_enum_match_arm"]);
    assert_eq!(lints_missing_from("mod a;\n", &lint), ["clippy::wildcard_enum_match_arm"]);
    assert!(lints_missing_from(also, &["clippy::unwrap_used", "clippy::expect_used"]).is_empty());
    assert_eq!(lints_missing_from(also, &lint), ["clippy::wildcard_enum_match_arm"]);
}

// testing R7: tests wait on conditions, not on the clock.

/// The test trees whose waits go through a helper (`testing R7`): the real
/// process tests, with `eventually`, `until`, `Quiet`, `throttle` and
/// `retry_busy_exec`, and the e2e cases, with `Scale`.
const WAIT_TREES: [&str; 2] = ["remote-agent/tests", "tests/e2e/cases"];

/// The files of [`WAIT_TREES`] that may sleep: the wait helper itself,
/// which scales every pause by `BRINDLE_TEST_SLOW`.
const SLEEPERS: [&str; 1] = ["remote-agent/tests/support/wait.rs"];

/// What a sleep in a test resolves to, however it is imported or renamed.
const SLEEP: &str = "std::thread::sleep";

/// The numbers of the lines of `text` that sleep.
fn sleep_lines(text: &str) -> Vec<usize> {
    needle_lines(&code_lines(text, Scope::All), &[SLEEP])
}

/// testing R7: no test in a wait tree sleeps, except in the wait helper.
#[test]
fn test_waits_go_through_the_wait_helper() {
    let files: Vec<PathBuf> = WAIT_TREES
        .into_iter()
        .flat_map(sources)
        .filter(|file| !SLEEPERS.contains(&slashed(file).as_str()))
        .collect();
    assert_none(
        offenders(&files, Scope::All, &[SLEEP]),
        "a test waits for a condition, within a limit that BRINDLE_TEST_SLOW (e2e: BRINDLE_E2E_SLOW) \
         scales, and never sleeps (docs/architecture/testing-architecture.md R7): use \
         `eventually`, `until`, `Quiet`, `throttle` or `retry_busy_exec` from \
         remote-agent/tests/support/wait.rs, or e2e's `Scale` (tests/e2e/wait.rs)",
    );
}

/// testing R7: the allowed sleepers are files there are.
#[test]
fn every_allowed_sleeper_exists() {
    let missing: Vec<&str> = SLEEPERS
        .into_iter()
        .filter(|file| !root().join(file).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "SLEEPERS names files that don't exist: {missing:?}"
    );
}

/// The sleep search: direct, imported, renamed and full-path sleeps are
/// found, a comment or a string isn't.
#[test]
fn a_sleep_in_a_test_is_found() {
    let text = r#"use std::thread;
use std::thread::sleep as nap;
use std::time::Duration;
fn direct() {
    thread::sleep(Duration::from_millis(1));
}
fn renamed() {
    nap(Duration::from_millis(1));
}
fn full() {
    std::thread::sleep(Duration::from_millis(1));
}
fn comment() {
    // thread::sleep(Duration::from_millis(1));
    let _ = "std::thread::sleep";
}
"#;
    // A renamed or bare `sleep` is found at its import, which is a site.
    assert_eq!(sleep_lines(text), [2, 5, 11]);
    assert_eq!(sleep_lines("use std::thread::sleep;\nfn f() { sleep(d); }\n"), [1]);
    assert!(sleep_lines("use std::thread;\nfn f() { thread::spawn(|| ()); }\n").is_empty());
}

// ports R10, testing R7: tests set no process environment.

/// What changes the process environment, however imported or renamed.
const ENV_WRITES: [&str; 2] = ["std::env::set_var", "std::env::remove_var"];

/// The numbers of the lines of `text` that change the environment and are
/// test code: every line when `whole_file` (a file of tests), else the lines
/// [`production`] blanked.
fn test_env_writes(text: &str, whole_file: bool) -> Vec<usize> {
    let hits = needle_lines(&code_lines(text, Scope::All), &ENV_WRITES);
    if whole_file {
        return hits;
    }
    let production: BTreeSet<usize> = production_lines(text)
        .into_iter()
        .map(|(number, _)| number)
        .collect();
    hits.into_iter()
        .filter(|number| !production.contains(number))
        .collect()
}

/// ports R10: no test sets or removes a process-wide variable.
#[test]
fn tests_set_no_process_environment() {
    let (_, test_modules) = scan_production();
    let mut files: Vec<(String, bool)> = ["tests", "remote-agent/tests", "remote-protocol/tests"]
        .into_iter()
        .flat_map(sources)
        .map(|path| (slashed(&path), true))
        .collect();
    files.extend(
        production_files()
            .into_iter()
            .map(|file| (file.clone(), in_test_module(&file, &test_modules))),
    );
    let mut found = Vec::new();
    for (file, whole) in files {
        let path = root().join(&file);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let original: Vec<&str> = text.lines().collect();
        for number in test_env_writes(&text, whole) {
            found.push(format!("{file}:{number}: {}", original[number - 1].trim()));
        }
    }
    assert_none(
        found,
        "a test changes no process-wide environment variable, since tests run in parallel \
         (docs/architecture/ports-and-adapters.md R10, testing-architecture.md R7): pass the \
         value as an argument, or set it on the `Command` of the process under test",
    );
}

/// The environment search: each form is found in test code, and production
/// code's, a comment's and a string's isn't.
#[test]
fn a_set_var_in_a_test_is_found() {
    let text = r#"use std::env;
pub fn parse() {
    unsafe { std::env::set_var("A", "1") };
}
#[cfg(test)]
mod tests {
    use std::env;
    use std::env::remove_var as unset;
    #[test]
    fn t() {
        unsafe { std::env::set_var("A", "1") };
        unsafe { env::set_var("A", "1") };
        unsafe { unset("A") };
        // std::env::set_var("A", "1");
        let _ = "std::env::remove_var";
    }
}
"#;
    // A renamed import is found at the import, which is a site.
    assert_eq!(test_env_writes(text, false), [8, 11, 12]);
    assert_eq!(test_env_writes("fn f() { std::env::set_var(\"A\", \"1\"); }\n", true), [1]);
    assert_eq!(test_env_writes("fn f() { std::env::set_var(\"A\", \"1\"); }\n", false), Vec::<usize>::new());
    assert_eq!(test_env_writes("fn f() { std::env::var(\"A\"); }\n", true), Vec::<usize>::new());
}
