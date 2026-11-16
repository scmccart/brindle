//! Architecture checks for the rules in `docs/architecture/`: the crate
//! boundaries, and the few source rules a text search can hold. Part of
//! plain `cargo test`; it reads manifests and sources and runs nothing.
//!
//! Each test names the doc rules it enforces. When one fails, change the
//! code, or change the rule in its doc and here in the same commit.

use std::collections::BTreeSet;
use std::fs;
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
