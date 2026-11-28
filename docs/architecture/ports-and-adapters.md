# Ports and adapters

Brindle uses the "values in, values out" form of ports and adapters: boundaries exist only where the code meets something it doesn't own (an operating system, a UI toolkit, another machine), and logic is never put behind one. There are two kinds of boundary here. The **crate boundaries** keep the app, the helper and the four shared crates pointing one way. The **seams** are the modules that own a platform concern, one per concern, `cfg`-selected, with the same item names on every platform. Code review enforces the **Rules** section; reviewers cite rules by number (`ports R4`).

## Why

Brindle runs on Linux and Windows, with macOS proposed (`port-to-macos`), and its helper also runs on remote Linux hosts with none of the app's dependencies. Without the crate rule, the compiler would let a component read `crate::settings`, or the helper pull in a GPU toolkit. Without the seam rule, each platform difference becomes a `cfg` at its call site, and a third platform means touching every one of them. Both failures are cheap to prevent and expensive to undo.

## The idea

- References point one way: the two binaries depend on the shared crates, never on each other, and the shared crates never on each other.
- A platform concern has exactly one home. Callers use its items and contain no `cfg` for it.
- Seams are thin. What to run, which environment, how to quote: those are pure helpers the seam calls (`fcis R1`).

```rust
// remote-agent/src/server/platform/mod.rs: both module sets export the same items.
#[cfg(unix)]    mod unix;    #[cfg(unix)]    pub use unix::*;
#[cfg(windows)] mod windows; #[cfg(windows)] pub use windows::*;

// The server core and the holder are platform-free:
use crate::server::platform::{Loop, Source, Listener, Pty};
```

## Idioms

**A per-platform module set** for a whole subsystem: `server/platform/{unix,windows}` each provide `event_loop` (`Loop`, `Source`, `Waker`), `conn` (`Stream`), `listen` (`Listener`, `connect`), `lock`, `daemon` (`ready_from_args`, `start_role`, `ReadyPipe`), `pty` (`Pty`, `Program`, `Group`), `dir` and `sys`. A missing item on one platform is a compile error on that platform's CI job.

**A single module** for a narrow concern: `src/terminal/process.rs` (a local program's name and directory), `src/primary.rs` (the primary selection), `src/desktop.rs` (launcher registration), `src/notify/{dbus,toast}.rs` behind `Notifier`.

**The app maps; the component crate doesn't know.** `Theme::ui_tokens` turns a terminal theme into `brindle-ui` tokens. Config names such as `picker_up` stand for `brindle_ui` actions in `actions.rs`. `brindle_ui::default_bindings()` returns keys and the app binds them.

**A stand-in at the seam instead of a mock.** `BRINDLE_NOTIFY_LOG` writes notifications to a file in place of D-Bus; `BRINDLE_LINK_OPENER` replaces `xdg-open`; `BRINDLE_SERVER_DIR` and `BRINDLE_NO_SCOPE` give tests a private server. In unit tests, a constructor argument is better still: the instance test's stand-in servers take the scope opt-out as an `Endpoint` flag, not from the environment.

## How it applies here

| Concern | Its one home |
| --- | --- |
| the terminal server's OS layer (loop, pipes and sockets, locks, detached start, PTYs, and pipe sessions: `piped`'s `Piped`, `PipeCommand` and `spawn_piped`, re-exported at `platform::`'s top, with `agent_directory` for an agent session's directory; the pure `platform/piped.rs` beside them decides the Windows flags, handle list and job, the launch, the batch-file hint, the hang-up steps and that directory) | `remote-agent/src/server/platform/` (`mod.rs` and its `unix` and `windows` sets) |
| PTYs and pseudoconsoles | `remote-agent/src/pty.rs`, `remote-agent/src/pty/`, `remote-agent/src/conpty/` |
| the server directory, endpoint names, access checks, scopes | `remote-protocol/src/{server_dir,endpoint,access,scope,win}.rs` |
| the app's connection to a server | `src/remote/local/{unix,windows,pipe}.rs` |
| a remote endpoint's transport (OpenSSH, Docker and Podman exec) | `src/remote/client.rs` |
| a local program's name and directory, the path style its directory reports use (`LOCAL_PATH_STYLE`) and whether its directory is read from the system or taken from those reports (`LOCAL_DIRECTORY`) | `src/terminal/process.rs` |
| the primary selection | `src/primary.rs` |
| showing the pointer a frame sets at once, not at the mouse's next move | `src/cursor.rs` |
| desktop notifications and window activation | `src/notify/{dbus,toast,activation}.rs` |
| launcher registration | `src/desktop.rs` |
| opening links | `src/terminal/links.rs` |
| small Windows helpers | `src/win.rs` |
| the relay, which runs on Linux only (declared under `target_os = "linux"`; every other platform refuses `relay`) | `remote-agent/src/server/relay.rs`, `remote-agent/src/liveness.rs` |
| resources compiled into the Windows executables (icon, version, manifest; `embed-resource` exists only on a Windows build host; the helper's version resource goes into `brindle-remote` only, not the test binary `brindle-fake-acp-agent`) | `build.rs`, `remote-agent/build.rs` |
| the UI toolkit | `gpui`, used by `brindle` and `brindle-ui` only |
| the emulator | `alacritty_terminal`, wrapped by `brindle-term-core` |

`tests/architecture.rs`'s `SEAMS` lists the same paths, and changes with this table in the same commit.

The composition root is `src/main.rs`: it takes the instance lock and handles `--install-desktop` before GPUI starts, then sets the GPUI globals (`Settings`, `Notifier`, `Hosts`, `Detached`) before the window opens.

## Rules

- **R1.** Crate references MUST follow the dependency rule in [README.md](README.md#dependency-rule): `brindle` and `brindle-remote-agent` depend on the shared crates, never on each other; `brindle-ui`, `brindle-term-core`, `brindle-remote-protocol` and `brindle-transcript` never depend on each other or on a binary's crate. A new crate gets its place in that rule and in `tests/architecture.rs` in the same change.
- **R2.** `brindle-ui` MUST depend on `gpui` alone and do no IO of its own. It knows nothing of terminals, config, themes' sources or the `ACTIONS` table, and it binds no keys itself. Mapping the app's concepts onto it is the app's job.
- **R3.** Each crate below the app MUST name only the dependencies on its closed list in `tests/architecture.rs` (`ui`: `gpui`, `log`; `term-core`: `alacritty_terminal`, `base64`, `parking_lot`; `remote-protocol`: `serde`, `serde_json`, `windows`; `transcript`: `serde`, `serde_json`; `remote-agent`: the three shared crates it builds on (`brindle-remote-protocol`, `brindle-term-core`, `brindle-transcript`), `agent-client-protocol-schema` (named only in `src/acp/`), `similar` (named only in `src/acp/diff.rs`, with no deadline or timeout), `libc`, `windows`, `embed-resource`). In the app, `brindle-transcript` is named only in `src/agent/transcript.rs`, which re-exports what the rest of the app uses. Adding one is a deliberate change to that list and this rule, and still obeys `CLAUDE.md`'s `Cargo.lock` rule. An entry inherited with `workspace = true` counts as the crate the root's `[workspace.dependencies]` names. A `[patch]` or `[replace]` of any crate is a change to this rule and to the test, as adding a dependency is.
- **R4.** A platform concern MUST have one seam, and production code outside it MUST NOT carry a platform `cfg` for that concern. Put a new platform difference in the narrowest module that owns the concern, or add an item to both sets of a per-platform module set. Never start a second, parallel seam for a concern that has one.
  - **Production code** is each workspace crate's `src/` tree, plus `build.rs` and `remote-agent/build.rs`. Test code is outside this rule and may pair platforms (`testing`'s "Platform pairs"): `#[cfg(test)]` items and the files of modules declared under one, `tests/**`, `remote-agent/tests/**` and the other crates' `tests/`, and examples (`ui/examples/`).
  - **A platform `cfg`** is a `cfg(…)` or `cfg_attr(…)` attribute, outer or inner, or a `cfg!(…)`, whose predicate names `unix`, `windows`, `target_os` or `target_family`. For `cfg_attr`, only its first argument is the predicate. A test-only predicate (`test`, or `all(…)` with a test-only argument; not `not(test)` or `any(test, …)`) makes its item test code.
  - **Not a platform difference:** a seam's module declaration, which is a platform `cfg` on `mod NAME;` (among any other attributes, `#[path]` included) whose file is a seam in the table above, as in `#[cfg(unix)] mod unix;`; and a lint-only `cfg_attr`, every attribute of which is `allow`, `expect`, `warn` or `deny`, such as `allow(dead_code)` on a pure helper that only one platform's seam calls (`fcis R7`).
  - Exception: the production files on [README.md](README.md#exceptions-in-force)'s `ports R4` row, and no others. `tests/architecture.rs` holds the same list. It only shrinks, and ends with the `port-to-macos` change.
- **R5.** A per-platform module set MUST export the same item names on every platform, and the code above it MUST name no platform type. The server core (`server/core.rs`, `server/core/`) and the holder (`server/holder.rs`) name no `libc::`, `windows::` or `std::os::` item outside their tests.
- **R6.** A seam is for platform, never for logic. Don't add a trait, a module or an indirection around a decision to make it mockable: a decision is a pure function, called and tested directly (`fcis R1`). A protocol version an outside program chooses (an ACP agent's) is a seam of the same kind as a platform (R14).
- **R7.** Seam code MUST be thin: it translates handles, bytes and calls. A decision it needs (what to run, the environment, the quoting, a name) comes from a pure helper that is tested on every platform (`cmdline::launch_line`, `windows_default_program`, `merge_environment`, `server_dir::choose`, `access::admit_client`, `ready_arg::role_args`, `pty::program`).
- **R8.** A seam's public items SHOULD take and return Brindle's own types or `std` types. Platform types (`HANDLE`, `RawFd`, `zbus` and `windows` crate types) stay inside the seam module.
- **R9.** A seam MUST return an expected platform failure as a value in the caller's terms (`railway R12`): a squatted pipe name is `AddrInUse`, a peer with another identity is a `Refusal`.
- **R10.** Tests SHOULD substitute at a seam with a stand-in that already exists, and add one in the seam's style when none does: an environment variable for the shipped binary, a constructor argument for unit tests. A unit test MUST NOT set a process-wide environment variable to steer code under test; other tests in the binary run in parallel.
- **R11.** GPUI globals and other process-wide state SHOULD be created in `main.rs` before the window opens. No dependency-injection container and no registry looked up by name.
- **R12.** Zed's `terminal` and `terminal_view` crates are GPL-3.0 and MUST NOT become dependencies, and no code is copied from them. They may be read for the shape of GPUI's API only.
- **R13.** Inside a crate, code SHOULD be organised by concern, one module per feature with its tests. No `utils`, `helpers`, `services` or `managers` modules.
- **R14.** Each ACP protocol version Brindle speaks MUST have one module under `remote-agent/src/acp/` (`acp/v1/`, later `acp/v2/`), and only that module names its `agent_client_protocol_schema` version module, its method names and its field names. The version a session speaks is a closed `Dialect`, chosen once from `initialize`'s answer and kept in the checkpoint; its variants are matched only in `acp/dialect.rs`, one exhaustive `match` per method, each calling the same-named item of its version's module (as R5's module sets do). Everything else in the crate reads `acp/forms.rs` and the JSON-RPC envelope (`acp/envelope.rs`), never a version: no `if v1`, no version check at a use (OpenSpec change `agents-tool-content`, decision 2).

## Anti-patterns

- **`#[cfg(windows)]` in a view or in `workspace.rs` to do something platform-specific in place.** Instead: an item in the seam that owns the concern, with an implementation for each platform (R4).
- **A second module that also knows how to find a process's directory.** Instead: extend `terminal/process.rs` (R4).
- **A `trait Clock` or `trait SessionStore` added so a test can swap it.** Instead: pass the value in, or test the pure function (R6).
- **A seam function that decides which shell to start.** Instead: `windows_default_program` decides; the seam starts what it is given (R7).
- **A component in `brindle-ui` that takes a `Theme` or reads `Settings`.** Instead: tokens in, and the app maps (R2).
- **`brindle` depending on `brindle-remote-agent` to reuse a type.** Instead: move the type to `brindle-remote-protocol`, `brindle-term-core` or `brindle-transcript` (R1).
- **`std::env::set_var` in a unit test.** Instead: a constructor argument (R10).

## Enforcement

`tests/architecture.rs`, run by plain `cargo test`:

- R1: `the_workspace_is_the_six_crates_these_rules_cover`, `the_app_and_the_helper_build_on_the_shared_crates`, `no_crate_names_a_forbidden_dependency`.
- R2 and R3: `crates_below_the_app_name_only_their_allowed_dependencies` (which follows `workspace = true` entries to the root's `[workspace.dependencies]` and their renames) and `no_dependency_is_patched_or_replaced` (no `[patch.*]` or `[replace]` entry in any of the six manifests), with the manifest reading tested by `a_workspace_dependency_counts_by_its_package` and `a_patch_or_replacement_is_found`; `brindle_ui_does_no_io_of_its_own`, whose search follows `use` items and full paths; and `only_the_acp_modules_name_the_schema_crate` (no source outside `remote-agent/src/acp/` names `agent_client_protocol_schema`, and no manifest but the helper's lists it); and `only_the_agent_transcript_module_names_the_transcript_crate` (no file in `src/` but `src/agent/transcript.rs` names `brindle_transcript`); and `only_the_markdown_module_names_the_parser` (no file in `src/` or `tests/` but `src/agent/markdown.rs` names `pulldown_cmark`); and `only_the_diff_module_names_similar` (no source but `remote-agent/src/acp/diff.rs` names `similar`, which sets no deadline or timeout).
- R4: `platform_cfgs_stay_in_their_seams` (every platform `cfg` in production code is in a seam or an excepted file), `every_excepted_file_still_has_a_platform_cfg` (the exception only shrinks), `every_seam_and_excepted_path_exists` and `the_ports_r4_exception_matches_the_readme`. The scanner is a text reader with tests of its own: `a_platform_cfg_outside_a_seam_is_found`, `what_is_not_a_platform_cfg_is_not_found`, `literals_and_comments_are_not_code_to_the_scanner`, `a_test_only_predicate_needs_test_to_hold` and `a_declared_module_resolves_as_rustc_resolves_it`. A `cfg` a macro produces is beyond it, and stays a review matter.
- R5: `the_server_core_and_holder_name_no_platform_type`. CI compiles both platforms, which holds the "same item names" half.
- R12: `no_crate_names_a_forbidden_dependency` (the dependency half; copying is a review matter).
- R14: `acp_versions_stay_in_their_modules` (`agent_client_protocol_schema::v1` only under `acp/v1/`, and `::v2` only under `acp/v2/`; `Dialect::V` only in `acp/dialect.rs`; no `v1::` or `v2::` in `acp/reduce/`, `acp/rpc.rs` or `acp/checkpoint.rs`). Method and field names outside a version's module are review's.

- R10: `tests_set_no_process_environment` (no test sets or removes a process-wide environment variable: `tests/**`, `remote-agent/tests/**` and the test code of every crate's `src/`; production code is review's).

R6–R9, R11 and R13 rely on review.

## PR checklist

- [ ] No new crate reference points the wrong way; the architecture test still passes unchanged, or its lists changed on purpose with this doc.
- [ ] No `cfg` for a platform concern outside that concern's seam.
- [ ] A new platform item exists in every platform's module set, under the same name.
- [ ] No trait or module was added around logic for mockability.
- [ ] Seam code decides nothing; its decisions are pure helpers with tests.
- [ ] No platform type leaves a seam; expected failures come back as values.
- [ ] Nothing in `brindle-ui` knows about terminals, config or `ACTIONS`.

## References

- The architecture plugin's `ports-and-adapters` skill (the default, "values in, values out" variant).
- `docs/code-map.md`: "Platform layer", "`brindle-ui`: Boundary"; `CLAUDE.md`: "Windows" (the `cfg` seams).
- `openspec/changes/archive/` for `add-brindle-ui` (why the UI crate was split out) and `terminal-server-windows` (the platform layer); `openspec/changes/port-to-macos`.
