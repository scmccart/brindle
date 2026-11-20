# Functional core, imperative shell

In Brindle every decision is a pure function or a GPUI-free model, and the code around it (a GPUI entity, a connection thread, an event-loop handler, `main.rs`) only gathers inputs, calls it and applies the result. The split is by module and type inside `brindle` and `brindle-remote-agent`, and by crate for `brindle-term-core`, which is pure as a whole. Code review enforces the **Rules** section; reviewers cite rules by number (`fcis R3`).

## Why

Brindle's shell is expensive to test: a view needs a window, a server role needs a process, and the suite that drives them takes minutes and opens windows. Every decision left inside a view or a thread can only be checked there. Decisions kept pure are checked in milliseconds by `cargo test` on Linux and Windows alike, including the ones about the other platform's data. The server core and the app also have to agree exactly on parsing, marks and status, or a replay drifts; sharing one pure crate is what keeps them identical.

## The idea

- A decision takes values and returns values. Time, environment variables and anything else ambient are arguments.
- A view holds a model and dispatches on what it returns. A thread or loop reads, calls a pure function, and writes.
- The boundary is plain data, never a GPUI handle, a socket or a callback into the world.

```rust
// src/terminal/pace.rs: the pacing decision is pure, and `now` is a parameter.
pub fn step(last_update: Option<Instant>, quick: u32, now: Instant) -> (u32, Option<Duration>) { … }

// The event task around it only reads the clock and waits:
let (quick, wait) = pace::step(self.last_update, self.quick, Instant::now());
if let Some(wait) = wait { timer(wait).await; }
```

## Idioms

**Ambient inputs as arguments.** The caller reads the environment; the helper decides.

```rust
// src/terminal/cmdline.rs
pub fn windows_editor_command(visual: Option<&str>, editor: Option<&str>, path: &str) -> (String, Vec<String>)
// src/reattach.rs: seconds are passed in, never read
pub fn age(started: u64, now: u64) -> Option<String>
// src/notify/policy.rs
impl RateLimiter { pub fn allow(&mut self, tab: u64, source: Source, now: Instant) -> bool }
```

**A probe for a path the function derived.** When the decision is "the first candidate that exists", the function builds the candidates and asks a read-only probe about each (R5); the shell passes the real check, and tests pass a closure over a set.

```rust
// remote-protocol/src/scope.rs: whether the user manager's socket exists, whether a PATH entry's systemd-run is executable
pub fn decide(no_scope: Option<&OsStr>, runtime_dir: Option<&OsStr>, path: Option<&OsStr>,
              exists: impl Fn(&Path) -> bool, executable: impl Fn(&Path) -> bool) -> Outcome
// src/terminal/cmdline.rs: pwsh, then powershell, in each PATH entry; the shell passes `|path| path.is_file()`
pub fn windows_default_program(path_dirs: impl IntoIterator<Item = PathBuf>, comspec: Option<&str>,
                               is_file: impl Fn(&Path) -> bool) -> (String, Vec<String>)
```

**A model under a view.** The model owns the state and every rule about it; the view owns focus, entities and painting.

```rust
// src/picker/sessions.rs: pure. `answer` applies one endpoint's listing; `view` builds the rows and the row to keep.
pub struct SessionsModel { … }
// The workspace holds it in PaletteKind::Sessions { model } and only passes answers in and rows out.
```

`FontsState` (settings dialog), `EditState` (`brindle-ui`'s text input) and `layout::group` follow the same shape.

**Owned mutation.** A model's methods take `&mut self` and change only the model. That is this repo's form of a state transition: ownership makes the change visible at the call site, and nothing else is touched.

**The other platform's data.** A helper that reads Windows paths or command lines is plain string code, so it compiles and is tested on Linux: `conpty::cmdline::program_stem` splits on `/` and `\` itself, because `Path` splits only on `/` there, and `server_dir::check_pipe_name` reads a pipe name as text. Win32 codes and states are plain values too: `pipe_state` keeps a Windows pipe's read and write states with the error codes written as numbers, and a Windows-only test compares each with the `windows` crate's constant.

## How it applies here

### Whole pure modules (checked by `tests/architecture.rs`)

Every line of production code in these is pure. `the_pure_modules_read_nothing_ambient` holds each file to R2 and R3, and its list is this table's.

| Pure code | What it decides |
| --- | --- |
| `brindle-term-core` (whole crate) | parsing, OSC 133 marks, OSC 7501 status records, replay rendering, the emulator's state |
| `src/terminal/{keys,mouse}.rs` | xterm key and mouse encodings |
| `src/terminal/cmdline.rs` | the Windows default program, the Unix and Windows editor commands, the merged environment, the Unix default program |
| `src/terminal/dir_check.rs` | whether the directory a local program last reported exists, from checks the terminal runs off the UI thread: which directory to check next, one at a time, and which answer still counts |
| `src/notify/policy.rs` | whether to notify, the notification's text, rate limits |
| `src/layout.rs` | the layout a tab is detached with (`record`), and which tabs and splits a launch rebuilds from them (`group`) |
| `src/reattach.rs` | which profile a session takes, which sessions are swept into tabs, which endpoints stay connected |
| `src/workspace/{tabs,launch,overlays,previews}.rs` | which tab is active once one is added, removed, switched to or moved, and where a new one goes; the directory a new tab starts in; what asking for an overlay does while another shows; the theme and profile a tab shows while the settings dialog previews them |
| `src/picker/sessions.rs` | the sessions palette's state and rows: what each endpoint listed, the sessions killed from it and the failed kills, the highlighted row, whether every endpoint has answered |
| `src/settings_dialog/themes.rs` | the settings dialog's Theme section (`ThemesState`): the theme list and its selection by name, the theme editor and its fields (`FieldId`), what deleting a theme refuses, and the `ConfigEdit` each operation writes |
| `ui/src/filter.rs`, `ui/src/primitives/placement.rs` | filtering, where an overlay goes |
| `remote-protocol/src/{client,holder,wire,version,relay,upgrade}.rs` | encoding, decoding, version negotiation, whether a build is newer |
| `remote-protocol/src/{access,hresult}.rs` | who may use a pipe, and an `HRESULT`'s Win32 code |
| `remote-agent/src/conpty/cmdline.rs` | launch-line quoting, the program and environment a session starts with |
| `remote-agent/src/server/{core,holder}/policy.rs` | the server core's `Lifecycle` (when it accepts clients, the reconnect grace, when it exits), what a `HELLO` gets, the queue marks, where spawns go and which holders drain, what a client's leaving, an exit and the grace period's end do to a session, which attaches are refused, which sessions a returning owner gets back, which foreground programs are looked at; the holder's spawn refusals, when an exited session is drained, when it starts a core |
| `remote-agent/src/server/platform/{ready_arg,ensure_dir,pipe_state,wait}.rs`, `remote-agent/src/conpty/{choice,hosts}.rs`, `remote-agent/src/pty/{program,stat}.rs` | beside a platform seam: a role's ready argument, whether the server directory is used, made or refused, a pipe's read and write states, a Win32 wait's timeout in milliseconds, which ConPTY is loaded, which processes a console's hang-up ends, a Linux session's shell, home, start directory and `TERM`, and `/proc` stat fields |

### Pure items in shell modules (held by review)

These modules mix pure items with shell code, which a text search can't tell apart, so review holds the items to R2, R3 and R5.

| Pure code | What it decides |
| --- | --- |
| `src/terminal/pace.rs` (`step`) | when the event task waits |
| `src/session.rs` (`merge_windows`, `drop_lost_zones`, `record_zones`, `resolve_tab`) | which windows, tabs and zones a launch restores, and the zones a tab is saved with |
| `src/settings.rs` (`Settings::new`, `Settings::zoomed`) | the fonts and sizes a config and a preview resolve to, and the zoom a key leaves |
| `src/settings_dialog.rs` (`Model`, `focus_target`), `src/settings_dialog/profiles.rs` (`ProfilesState`, `ProfileEditor`), `src/settings_dialog/fonts.rs` (`FontsState`), `src/settings_dialog/clipboard.rs` (`ClipboardState`) | overlay state and rows, which buttons are enabled, where focus goes, and the `ConfigEdit` each operation writes |
| `ui/src/text_input.rs` (`EditState`) | editing |
| `src/workspace.rs` (`program_title`) | whether an OSC title is the program's own, by the path style of the programs' paths |
| `src/desktop.rs` (`without_verbatim_prefix`) | a Windows path without its verbatim `\\?\` prefix |
| `src/notify/mod.rs` (`escape_markup`) | a notification's text as D-Bus markup |
| `src/instance.rs` (`socket_path_in`, `pipe_name_for`) | where the instance listens: its socket's path and its pipe's name |

The shell is the GPUI entities (`Workspace`, `ZoneTreeView`, `TerminalView`, `Palette`, the settings dialog), `TerminalElement`'s paint, the connection's reader and writer threads, the holder's and the server core's loop handlers, and `main.rs`.

## Rules

- **R1.** A decision MUST be a pure function or a method of a GPUI-free model: what to show, send, open, keep or drop; an ordering, a grouping or a policy. The entity, thread or handler around it gathers inputs, calls it and applies the result. A decision inside the shell can only be tested with a window or a server.
- **R2.** Pure code MUST take time as an argument (`now: Instant`, seconds as `u64`) and MUST NOT call `Instant::now()` or `SystemTime::now()`. Random values (tokens, session keys) are made in the shell and passed in.
- **R3.** Pure code MUST NOT read the environment, files, `/proc`, sockets, GPUI globals or mutable statics. The caller reads them and passes values.
- **R4.** `brindle-term-core` MUST stay pure as a crate: no filesystem, network, process, environment, thread or clock call, and no dependency beyond `alacritty_terminal`, `base64` and `parking_lot`. It is what makes the server core's emulator and the app's agree.
- **R5.** Pure functions and models MUST take and return plain values: no `Entity`, `Window`, `App` or `Context`, no `Task`, no socket or handle, no callback that reaches the world. A view holds its model; the model never holds the view. A probe is allowed: a read-only `Fn(&Path) -> bool` (or a check of the same kind) that answers one question about a path the function itself derived, such as whether a candidate is a file. The shell passes the real check; tests pass a closure over plain values. A callback that acts, waits, or reads an entity, a window or a process is not a probe.
- **R6.** Parsing, marks, status and replay logic MUST live in `brindle-term-core` and be used by both the app and the server core. Never fork that logic into `src/` or `remote-agent/`: the two would drift, and replays would differ from what the client drew.
- **R7.** A pure helper MUST compile and be tested on every platform, whichever platform's data it handles. Don't `cfg`-gate logic; `cfg` belongs to the seam that calls it (`ports R4`). Don't use `Path` on another platform's paths.
- **R8.** Every pure function and model MUST ship with unit tests in its own file's `#[cfg(test)] mod tests` that need no display, server, clock or sleep (`testing R1`). R8 binds production code (as `ports R4` defines it: each crate's `src/` and build scripts). Test code is out of its scope: the e2e and perf harnesses' own pure helpers are held by the harness's `--self-test` (`testing R3`).
- **R9.** A function that decides *and* acts SHOULD be split: return what to do as data (rows and the index to keep, the tabs to open, the frame to send), and let the caller do it. Reach for a callback parameter only when the output must interleave with a stream in order, or for a probe (R5).
- **R10.** A new conditional in a view, a thread or a loop handler SHOULD be questioned in review. If it chooses between outcomes from state, it moves into the model or a pure function, and the shell dispatches on a richer return value.

## Anti-patterns

- **A view method that compares fields to decide what a palette lists or which tab opens.** Instead: the model returns the rows or the choice (R1).
- **`Instant::now()` inside a policy or a limiter.** Instead: `now` is a parameter (R2).
- **`std::env::var` inside a helper that picks a program or a path.** Instead: the caller passes the values (R3), as `windows_editor_command` takes `visual` and `editor`.
- **A callback that reads an entity, a window or a process inside pure code.** It is not a probe. Instead: the shell gathers the value (R5).
- **A copy of an OSC or mark rule in the app "because the server doesn't need it".** Instead: one implementation in `brindle-term-core` (R6).
- **A helper behind `#[cfg(windows)]` that only manipulates strings.** Instead: ungated and tested everywhere (R7).
- **Checking a rule through the e2e suite because the rule lives in a view.** Instead: move the rule into a model and unit-test it; keep one e2e case for the wiring.

## Enforcement

- `tests/architecture.rs` checks R4: `term_core_does_no_io_and_reads_no_clock` (source search) and `crates_below_the_app_name_only_their_allowed_dependencies` (the closed dependency list). `the_wire_modules_do_no_io` holds the same for the protocol's message modules.
- `the_pure_modules_read_nothing_ambient` checks R2 and R3 for the whole pure modules (the first table in "How it applies here"). Their production code calls no clock, names nothing of `std::{fs, net, process, env, thread}`, declares no mutable static (`OnceLock`, `LazyLock`, `Mutex`, `RwLock`, `Cell`, `Atomic…`, `static mut`, `thread_local!`) and calls no file-system method (`.is_file()`, `.exists()`, `.metadata()` and the like), so a probe is passed in. The search follows `use` items (grouped, nested, renamed, globs) and full paths, as every source search in that file does, and skips test code. `the_pure_module_list_matches_the_fcis_table` keeps its list equal to the table, `every_pure_module_exists` keeps each file real, and `a_pure_module_reading_the_world_is_found` plants each kind of read and a test-only `Instant::now()` that must not count.
- `platform_cfgs_stay_in_their_seams` (`ports R4`) holds R7's `cfg` half in production code: outside the seams and `ports R4`'s excepted files nothing is platform-gated, and `brindle-term-core` has no platform `cfg` at all.
- `the_neutral_code_is_never_gated` holds R7's `cfg` half inside the seams too, for the code every platform calls, wherever it sits:
  - **Modules:** the whole pure modules (`PURE_MODULES`, the first table) and `NEUTRAL_MODULES` (`remote-protocol/src/server_dir.rs`, which reads the working directory, so it isn't a whole pure module). A lint-only `cfg_attr` aside, their production code and their own `mod` declaration have no platform `cfg`, and each ancestor's declaration has none or exactly `cfg(any(target_os = "linux", windows))`, Brindle's two platforms.
  - **Items:** `NEUTRAL_ITEMS`, the second table's items in files that have platform `cfg`s, a seam or a file on `ports R4`'s exception (`desktop::without_verbatim_prefix`, `notify::escape_markup`, `instance::{socket_path_in, pipe_name_for}`, `workspace::program_title`). Each `fn` is at column 0, so not inside an inline module, with no platform `cfg` among its attributes.
  - `every_neutral_module_and_item_exists` keeps both lists real, and `a_gated_neutral_module_is_found`, `a_gated_neutral_item_is_found` and `a_lint_only_cfg_attr_and_test_code_do_not_count` plant each case.
- Testing on every platform, keeping `Path` off another platform's paths, and keeping the other pure items in shell modules ungated are review matters.
- R1 and R5–R10 otherwise rely on review, as do R2 and R3 for the pure items in shell modules (the second table). Only the reviewer stops an item row, such as `pace::step` or `session::resolve_tab`, from calling `Instant::now()`; the check stops a whole module.

## PR checklist

- [ ] New decisions are pure functions or model methods; shell changes only gather, call, apply and dispatch.
- [ ] Pure code takes time and environment values as arguments and reads nothing ambient.
- [ ] No GPUI handle, socket or world-reaching callback crosses into pure code.
- [ ] Nothing was added to `brindle-term-core` that does IO, reads a clock or starts a thread.
- [ ] Parsing, marks, status and replay changes are in `brindle-term-core`, used by both sides.
- [ ] Pure helpers are ungated and have unit tests that run on every platform.

## References

- The architecture plugin's `functional-core-imperative-shell` skill.
- `CLAUDE.md`, "Architecture": `terminal/`, `brindle-term-core`, `picker.rs`, `settings.rs`.
- `openspec/changes/archive/` for `add-brindle-ui` and `terminal-server`, which set the crate split.
