# Design

## Context

See proposal.md for why. Finding 9 of the 2026-11-16 audit counts 14 `fcis R7` entries and 7 `ports R7` entries. This change takes 11 of the `fcis R7` entries and all 7 `ports R7` entries:

| Entry | Code today | Rule |
| --- | --- | --- |
| b13 #1 | `server/platform/windows/daemon.rs:155` `parse_ready_handle`, `:170` `role_args` (and the Linux twins `unix/daemon.rs:75` `parse_ready_fd`, `:92` `role_args`) | `fcis R7` |
| b13 #6 | `server/platform/windows/dir.rs:38-67` `ensure_with` | `ports R7` |
| b14 #1 | `server/platform/windows/listen.rs:104` `pipe_name`, `:128` `check_path`, `:237` `check` | `fcis R7`, `ports R7` |
| b14 #7 | `server/platform/windows/pipe.rs:529-546` `read_done`, `:594-606` `write_done`, `:384-386`/`:429-435` readiness, `:655-687` `readable`/`writable`/`sent`/`hung_up`/`meets` | `fcis R7`, `R8`, `ports R7` |
| b15 | `server/platform/windows/pty.rs:374` `plain`, `:398` `program_name`, `:412` `environment`, `:429` `wants_inbox`; `:257-317` `Group::host`, `Host::end_its_programs` | `fcis R7`, `ports R7` |
| b09 #3 | `conpty/api.rs:67-109` `packaged`, `:115-133` `shipped_dir` | `ports R7` (and `fcis R9`) |
| b10 #3, #4 | `pty/unix.rs` `default_shell`, `home_dir`, `expand_remote_cwd`, `command_for`'s start directory, the `TERM` default, `parse_stat_field` | `ports R7`, `fcis R7` |
| b20 #3 | `remote-protocol/src/endpoint.rs:100-108`, the `#[cfg(not(windows))]` `Endpoint::shown` | `fcis R7` |
| b21 #1, #2 | `remote-protocol/src/win.rs:322-353` `Ace`, `Dacl`, `allows_only` | `fcis R8`, `fcis R7` |
| b21 #3 | `remote-protocol/src/win.rs:504-584` `ensure_helper_copy`, `place_copy`, `prune_helper_copies` | `ports R7` |
| b01 #7 | `src/desktop.rs:399-406`, `start_menu::resolved` | `fcis R7`, `R8` |
| b02 #4 | `src/notify/dbus.rs:134` `escape_markup` | `fcis R7` |
| b07 #10 | `src/workspace.rs:2258` `cfg!(windows)` in `terminal_automatic_title` | `fcis R7` |
| b23 | `src/instance.rs:124-130` `imp::socket_path`, `:257-264` `imp::pipe_name` | `fcis R7`, `ports R7` |
| b28 #3 | `src/terminal/mod.rs:1214-1219`, the `#[cfg(unix)]` `default_program` | `ports R7` |

Other changes take the remaining three `fcis R7` entries:
- b07 #11, `shell_quote`: change 7 (`keep-pure-modules-pure`);
- b31 #2, term-core's `cfg!(windows)`: change 5 (`enforce-platform-seams`, `PathStyle`);
- b11 #1, the core's `client_reparent` pair: change 5's `ports R4` exception list keeps it until `port-to-macos`.

**What the plan's section 10 gets wrong, and what this change does about it:**
- **`pipe_name` and `check`:** the plan files them under "b19/b21" in `remote-protocol`. They are b14 #1, in the agent's `windows/listen.rs`, and b19 has no finding 9 entry. Their decisions still move to `remote-protocol`, beside the pipe-name builders and `peer_allowed` (Decision 4).
- **The helper-copy decisions:** the plan cites "b14, b21". Only b21 #3 has them.
- **b23 `instance.rs`:** the plan leaves it out, but it is one of finding 9's `fcis R7` entries and one of its `ports R7` entries. It is added.
- **b07 #11:** change 7 owns it, so it is dropped here.
- **Dependencies:** the plan lists change 5 only. This change also depends on change 7, for:
  - its `fcis R5` probe wording;
  - `PURE_MODULES` and `production_lines` in `tests/architecture.rs`;
  - `program_title`.

**Constraints:**
- **Crate boundaries:**
  - A decision both binaries use goes in `brindle-remote-protocol`. The binaries never reference each other (`ports R1`, `R3`).
  - `remote-protocol`'s `win.rs` is `#[cfg(windows)]` as a whole, so what moves out of it needs a neutral sibling.
  - `remote-protocol` returns its own error types or `io::Result`, never `String` (`railway R2`).
- **Change 5's rules:**
  - `ports R4` binds production code only.
  - A lint-only `#[cfg_attr(<platform>, allow(dead_code, …))]` is not a platform `cfg`.
  - Its seam set covers `server/platform/`, `pty.rs`, `pty/`, `conpty/` and `remote-protocol/src/{server_dir,endpoint,access,scope,win}.rs`.
  - The `cfg` sites in its 17 listed files stay where they are, until `port-to-macos`.
- **Change 7's `fcis R5` probe:** a read-only `Fn(&Path) -> bool` that answers one question about a path the function itself derived. A callback that reaches the world otherwise stays forbidden.
- **The model:**
  - `remote-agent/src/conpty/cmdline.rs` is declared `pub mod cmdline;` with only `#[cfg_attr(not(windows), allow(dead_code, reason = …))]`.
  - It splits Windows paths on `/` and `\` itself (`program_stem`).
  - It is tested on Linux.
- **Both crates' gates:** `remote-agent`'s `server` and `pty` modules are gated `cfg(any(target_os = "linux", windows))`. Those are Brindle's two platforms, so a module under them still builds and is tested everywhere CI runs.

## Goals / Non-Goals

**Goals:**
- Every decision in the table is a pure function or a GPUI-free model in ungated code, with unit tests in its own file (`fcis R7`, `R8`) that run in Linux `cargo test` and Windows `cargo test`.
- The seam code left behind only gathers, translates handles and calls (`ports R7`). It reads the same things it reads today, in the same order, wherever an extra read could block or cost.
- A check keeps the pure modules and the named neutral items free of platform `cfg`.

**Non-Goals:**
- **Behaviour:** no change to it, including log lines, error texts and `io::ErrorKind`s, which stay byte for byte.
- **Change 5's `ports R4` exception list:** moving `cfg` sites out of the 17 listed files is `port-to-macos`'s job.
- **Failure outcomes:** `prune_helper_copies`, `packaged` and `kill_remaining` still swallow what they swallow today. Change 12 gives them outcomes.
- **Types and decisions not in finding 9:**
  - b13 #2's `millis` and b14 #6's `Input` flags go to change 13;
  - b02 #2/#3 (`fcis R1` entries) go to change 13;
  - the relay's login shell (b15 `fcis R1`) goes to change 13;
  - `api(inbox: bool)` keeps its `bool`.
- **The Unix directory check** `platform/unix/dir.rs::ensure_for` (b12, a smell, not an entry).
- **arg0:** the login shell's `-<name>` (`pty/unix.rs:46-53`) stays in the Linux seam. It reads a Linux path with Linux's own `Path` (Decision 3).

## Decisions

### 1. Layers and crates

| Part | Layer | Crate |
| --- | --- | --- |
| `access::{Ace, Dacl, allows_only, SYSTEM_SID, admit_client}` | pure, whole module (`access.rs` already is one) | `brindle-remote-protocol` |
| `hresult::win32_code` | pure, new whole module | `brindle-remote-protocol` |
| `server_dir::{check_pipe_name, BadPipeName, holder_socket_stem, copy_temp_name, is_current, after_lost_rename, LostRename, pruned_builds, EntryKind, after_image_removal, ImageRemoval, PruneDir}` | pure items in an ungated seam module | `brindle-remote-protocol` |
| `win::{ensure_helper_copy, place_copy, prune_helper_copies, os}`, `impl Dacl { read, of_handle, of_path }` | seam: reads, renames, removes, then calls the above | `brindle-remote-protocol` |
| `server/platform/ready_arg.rs` | pure, new whole module | `brindle-remote-agent` |
| `server/platform/ensure_dir.rs` | pure, new whole module | `brindle-remote-agent` |
| `server/platform/pipe_state.rs` | GPUI-free model without handles, new whole module | `brindle-remote-agent` |
| `conpty/cmdline.rs` (`program`, `environment`) | pure, existing whole module | `brindle-remote-agent` |
| `conpty/choice.rs`, `conpty/hosts.rs` | pure, new whole modules | `brindle-remote-agent` |
| `pty/program.rs`, `pty/stat.rs` | pure, new whole modules | `brindle-remote-agent` |
| `platform/{unix,windows}/{daemon,dir,listen,pipe,pty,event_loop}.rs`, `conpty/api.rs`, `pty/unix.rs` | seams: gather, translate handles, call | `brindle-remote-agent` |
| `desktop::without_verbatim_prefix`, `notify::escape_markup`, `workspace::program_title`, `cmdline::unix_default_program`, `instance::{socket_path_in, pipe_name_for}` | pure items | `brindle` |
| `desktop::start_menu::resolved`, `terminal::default_program` (unix), `instance::imp::{socket_path, pipe_name}`, `notify::dbus` | seams: gather and call | `brindle` |

No decision moves into a view, a thread or a seam. No crate reference changes: the agent already depends on `remote-protocol`, and nothing in the app uses the new protocol items. `hresult.rs` is in `remote-protocol` because `win.rs` (`os`) and the agent (`event_loop::win32`, `pty::plain`) both convert `HRESULT`s, and the agent can see `remote-protocol` but not the other way round.

### 2. How a neutral module is declared

Each new module copies `conpty::cmdline`'s form:
- an ungated `mod` (or `pub mod`) declaration, carrying at most `#[cfg_attr(not(windows), allow(dead_code, reason = "only the Windows … calls it"))]`, or `not(target_os = "linux")` for the Linux-only callers;
- no `cfg` in its production code;
- `#[cfg(test)] mod tests` in the same file.

Where the module sits:
- **Beside the seam that calls it, in the same directory.** `server/platform/` gains `ready_arg.rs`, `ensure_dir.rs` and `pipe_state.rs`. `conpty/` gains `choice.rs` and `hosts.rs`. `pty/` gains `program.rs` and `stat.rs`.
- **Named for the concern, never `util` or `helpers` (`ports R13`).**
- **The seam re-exports what its callers name today:**
  - `platform::daemon::role_args` stays a platform item, and is one line;
  - `platform::pty::program_name` stays the item `holder.rs:762,787` calls;
  - `win::{Dacl, SYSTEM_SID}` stay reachable through `pub use crate::access::{…}`.

  So `core.rs`, `holder.rs` and Brindle's client don't change.

**Alternative rejected:** a separate `platform/neutral/` directory. That puts each decision away from the handles it serves, and splits one concern across two directories.

### 3. Another platform's paths and names are text

Neutral code never parses another platform's path with `Path`, because `file_name`, `is_absolute`, `components` and prefixes all differ per platform (`fcis R7`):
- **Paths it reads** (a Windows pipe name, a verbatim prefix, a console title) arrive as `&str` or `OsStr` and are handled as text.
- **Paths it builds** come only from joining relative components onto a base it was given, with `Path::join`, as `server_dir::helper_copy` does already.
- **Tests** compare paths built the same way on both sides (`PathBuf::from(base).join("brindle")…`). Such a path comes out equal on Linux and Windows, whatever separator `join` puts in.

The login shell's `-<name>` (`Path::file_name` on a Linux program path) stays in `pty/unix.rs`: it is Linux's own path, read by Linux's `Path`. Moving it would need `OsStr::from_encoded_bytes_unchecked` to keep non-UTF-8 names. The `TERM` default and every other `pty/unix.rs` decision move.

### 4. Values in, probes for derived paths, laziness kept in the shell

Every moved function takes the values the seam gathered and returns a value or a closed enum. Three take a probe as change 7's `fcis R5` defines it:
- `pty::program::passwd_shell(shell, exists)`: whether the passwd entry's shell exists;
- `pty::program::start_dir(wanted, home, is_dir)`: whether a candidate directory exists;
- `conpty::choice::shipped(exe_dir, extract_dir, holds_embedded)`: whether a directory holds the embedded ConPTY files, a read-only comparison of bytes.

Two of today's lookups are lazy for a reason, and stay lazy. The pure part is split in two steps, so the shell asks for the costly value only when the first step answers `None`, as change 7 does for titles (its Decision 3):
- **The passwd entry:** `getpwuid_r` can go through NSS to LDAP and block the holder's one thread (`process R8`). `shell_from_env(request, own) -> Option<OsString>` and `home_from_env(request, own) -> Option<PathBuf>` come first. Only when they give nothing does the seam read the passwd entry and call `passwd_shell` or use its home.
- **The process's start time in `end_its_programs`:** it is computed for each candidate today only when the candidate is on the host. The seam now reads it for every candidate it has opened. That is one `GetProcessTimes` on a handle already open, during a hang-up only, so it stays eager rather than adding a second step.

**Alternative rejected:** passing `passwd_entry` as an `FnOnce` lookup. It reads the process's user record, which change 7's probe definition excludes.

### 5. The sites

**5.1 `server/platform/ready_arg.rs` (b13 #1 and the Linux twin).**
- **Types:**
  - `pub enum ReadyArg { Fd, Handle }`, with `flag()` (`--ready-fd`, `--ready-handle`) and `placeholder()` (`{ready_fd}`, `{ready_handle}`);
  - `pub fn role_args(arg: ReadyArg, role: &str) -> Vec<String>`;
  - `pub fn parse_fd(role, args) -> Result<Option<i32>, String>`: `>= 0`, the Unix rule;
  - `pub fn parse_handle(role, args) -> Result<Option<usize>, String>`: `!= 0`, the Windows rule.

  Both parsers share a private `split(arg, role, args) -> Result<Option<&str>, String>` for the `[]` and `[flag, value]` shapes and the usage text. The two parsers keep their own value text: "a file descriptor number" and "a handle number".
- **Errors:** the agent's `String`s, unchanged, since these messages go straight to the CLI's stderr.
- **What stays in the seams:**
  - `Ready::from_fd`/`from_handle`;
  - `READY_FD`;
  - the trampoline's `inherited_ready`, which asks Windows whether the handle is a pipe it holds, and only takes `ReadyArg::Handle.flag()`.
- **Brindle's own literals** (`src/remote/local/unix.rs:54`, `windows.rs:76`) are the client's, not decisions, and stay.

**5.2 `server/platform/ensure_dir.rs` (b13 #6).**
- **Types:**
  - `pub enum Found { Dir, NotDir, Missing, Unreadable }`: what `symlink_metadata` said; a link is `NotDir`;
  - `pub fn first(found, protect: Protect) -> First`, where `Protect { Existing, AsFound }` and `First { Use, Protect, Create, Refuse, FailRead }`;
  - `pub fn after_create(found) -> AfterCreate`, where `AfterCreate { Use, Refuse, FailCreate }`.

  Two enums, so that a second look can't ask to create again or to protect.
- **What stays in the seam:**
  - the `io::Error` kept beside `Unreadable` for the message;
  - `create`, `win::protect_dir` and the message texts;
  - `ensure`'s `default_dir() == Some(dir)` comparison, which picks `Protect`.

**5.3 `server/platform/pipe_state.rs` (b14 #7).** The overlapped pipe's bookkeeping, without handles, `Op`s or buffers.
- **Win32 codes:** plain `u32` constants:
  - `NO_ERROR` 0, `WRITE_FAULT` 29, `HANDLE_EOF` 38, `BROKEN_PIPE` 109, `MORE_DATA` 234, `OPERATION_ABORTED` 995.

  A `#[cfg(windows)]` test in `pipe.rs` asserts that they equal the `windows` crate's constants.
- **Results:**
  - `read_ended(bytes, error) -> ReadEnd { Bytes(usize), End, Cancelled, Failed(u32) }`: today's match, where a read of nothing is the end;
  - `write_ended(bytes, error) -> WriteEnd { Took(usize), Failed(u32) }`: a write that took nothing is `Failed(WRITE_FAULT)`.
- **State:**
  - **`ReadState { start, end, eof, error }`:**
    - `ready()`;
    - `apply(ReadEnd, Handle)`, where `enum Handle { Open, Closed }` and a closed reader is at its end whatever the read said.
  - **`WriteState { sending, sent, queued, error, shut }`:**
    - `unsent()` and `ready()` (under `WRITE_MARK`, or failed);
    - `fail(code)` and `apply(WriteEnd)`;
    - `rotate(Handle) -> Rotation { Send, Idle, Close }`: the swap, dropping a queue's capacity over `KEEP`, and closing a shut writer that sent everything.
  - **`Sides { reader: Option<&ReadState>, writer: Option<&WriteState> }`:** `readable`, `writable`, `sent`, `hung_up` and `meets(Need)`.
  - **`Need`** moves here. `From<Interest> for Need` stays in `pipe.rs`.
- **What stays in the seam:**
  - handles and `Op`;
  - the buffers' memory and the unsafe slices over `sending`;
  - issuing, `close_writer`, `settle`, `waits` and `due`;
  - `State`'s methods, each one line over `Sides`.

  `WRITE_MARK` and `KEEP` move here, and `pipe.rs` re-exports `WRITE_MARK` for `event_loop`.

**5.4 `remote-protocol`: the pipe name and the client check (b14 #1).**
- **The name:**
  - `server_dir::check_pipe_name(name: &str) -> Result<(), BadPipeName>`, where `enum BadPipeName { NoPrefix, BadName, TooLong }` and `Display` gives today's three problem texts;
  - the length is counted with `encode_utf16`, and `PIPE_PREFIX` and `MAX_NAME` (256) move with it;
  - a test checks that `core_pipe`, `holder_pipe` and `pty_pipe` with the longest build id all pass.
- **The listener keeps:**
  - `path.to_str()` (a name that isn't UTF-8 is `NoPrefix`, as today);
  - `encode_wide`;
  - wrapping as `InvalidInput` with `"{path} is not a pipe name: {problem}"`.
- **The check:**
  - `access::admit_client(own: &Principal, client: io::Result<Principal>) -> io::Result<()>` is today's `check`: `PermissionDenied` with the same two texts, through `peer_allowed(.., Side::Server)`;
  - the listener calls it, and `the_client_must_be_the_same_user_at_no_lower_integrity` moves to `access.rs`.

**5.5 `remote-protocol`: DACLs, holder files, helper copies, HRESULTs (b20 #3, b21, b13 #2's `win32`, b15's `plain`).**
- **DACLs:**
  - `Ace`, `Dacl`, `Dacl::allows_only` and `SYSTEM_SID` move to `access.rs`;
  - `win.rs` keeps `impl Dacl { read, of_handle, of_path }`, an inherent impl in another module of the same crate, and re-exports the moved names;
  - by-value tests cover b21 #1's five refusals.
- **Holder sockets:**
  - `server_dir::holder_socket_stem(name) -> Option<&str>` returns the stem of `holder-….sock`, or `None`;
  - the Unix `Endpoint::shown` builds its paths from it, and the `cfg` pair in `endpoint.rs` still picks the parser, a seam's choice in a seam file;
  - its tests sit beside `parse_holder_lock`'s. It splits a file name, not a wire frame, so it is not a fuzz target (`process R6`). Change 3's fuzzing of `parse_holder_lock` is unaffected.
- **Helper copies:**
  - `copy_temp_name(pid, n) -> String` is today's `brindle-remote.exe.<pid>-<n>.tmp`;
  - `is_current(copy_build: Option<&str>, build) -> bool`;
  - `after_lost_rename(copy_build, build, copy_exists) -> LostRename { UseWinner, Replace, Fail }`;
  - `pruned_builds(entries: Vec<(OsString, EntryKind)>, keep: &str) -> Vec<OsString>`, where `EntryKind { Dir, Other }`, which skips `keep` and non-directories;
  - `after_image_removal(ImageRemoval) -> PruneDir`, where `ImageRemoval { Removed, Missing, Failed }` and `PruneDir { Remove, Keep }`, so a running image keeps its directory.
- **What stays in `win.rs`:**
  - `MoveFileExW`, `fs::copy`, `build_of` and the reads;
  - `remove_file`, `remove_dir_all` and `read_dir`;
  - the `NEXT` counter.

  It reads `copy.exists()` before calling `after_lost_rename`, on the path whose rename just failed. The callers (`core.rs:185-201` `prune_copies`, `src/remote/local/windows.rs:62-72`) don't change.
- **HRESULTs:**
  - `hresult::win32_code(hresult: u32) -> Option<u32>` is `Some(code & 0xffff)` for facility 7 (`0x8007xxxx`), else `None`;
  - each caller keeps its own fallback:
    - `win::os` falls back to `io::Error::other`;
    - `event_loop::win32` returns the whole code;
    - `pty::plain` returns the error as it was.

**5.6 `conpty` (b09 #3, b15).**
- **`cmdline::program(command: Option<&str>, env: &BTreeMap<String, String>) -> &str`:** the command, else a non-empty `COMSPEC` of any case, else `cmd.exe`. `pty::program_name` calls it.
- **`cmdline::environment(given, exact, own: Vec<(String, String)>) -> Vec<(String, String)>`:**
  - when `exact`, `given`;
  - otherwise `own` without the names `given` sets, whatever their case, then `given`.

  The seam passes its UTF-8 `vars_os`. Change 4 moves `environment_block` out of this file, which is a different function.
- **`choice::wants_inbox(env) -> bool`, with `INBOX`.**
- **`choice::extract_dir(local_app_data: Option<&OsStr>, version) -> Option<PathBuf>`:** an empty variable is unset.
- **`choice::shipped(exe_dir, extract_dir, holds_embedded) -> Shipped`,** where `enum Shipped { Beside(PathBuf), Extracted(PathBuf), ToExtract(PathBuf), Nowhere }`.
- **What `api.rs` keeps:**
  - the `OnceLock`s;
  - the log lines;
  - `create_dir_all`, `write_replacing` and `load`;
  - `same` (the probe it passes).

**5.7 `conpty/hosts.rs` (b15, which processes a hang-up ends).**
- **Types:**
  - `pub struct Listed { pid, parent }`, mapped from `PROCESSENTRY32W`;
  - `pub struct HostSeen { pid, started }`.
- **Functions:**
  - `is_session_host(exited, host_started: Option<u64>, root_started: Option<u64>) -> bool`: not exited, both times known, and the host started first, which rules out a reused pid;
  - `is_ours(snapshot: &[Listed], host: u32, own: u32) -> bool`;
  - `candidates(snapshot, host, own) -> Vec<u32>`: everything but this process and the host;
  - `ends(on: Option<u32>, started: Option<u64>, host: &HostSeen) -> bool`: on the host, and started no earlier than it.
- **What the seam keeps:**
  - `OpenProcess`, `console_host`, `GetProcessTimes`, `WaitForSingleObject` and `TerminateProcess`;
  - asking and ending through one handle.

**5.8 `pty/program.rs` and `pty/stat.rs` (b10 #3, #4).**
- **`program.rs`:**
  - `shell_from_env(request: Option<&str>, own: Option<&OsStr>) -> Option<OsString>`;
  - `passwd_shell(shell: Option<OsString>, exists) -> OsString`, falling back to `/bin/sh`;
  - `home_from_env(request, own) -> Option<PathBuf>`;
  - `expand_home(cwd: &str, home: Option<&Path>) -> PathBuf`, for `~` and `~/…`;
  - `start_dir(wanted, home, is_dir) -> Option<PathBuf>`;
  - `default_term(exact: bool, env) -> Option<&'static str>`.
- **`stat.rs`:** `STAT_PGRP`, `STAT_TPGID` and `parse_stat_field(stat, field) -> Option<i32>`, with tests for an empty line, a missing `)`, a short line, and a command holding `) (`.
- **What `pty/unix.rs` keeps:**
  - `passwd_entry`, `Command`, arg0 and `/proc` reads;
  - `home_dir(env)`, which `platform/unix/dir.rs` also calls. It becomes `home_from_env(…).or_else(passwd home)`.
- **The home directory in `command_for`:** the seam reads it once, only when the request names a directory. That adds a passwd read only when no `HOME` is set anywhere and the directory exists, a case Brindle's `SPAWN` never makes, since it carries `HOME` (Risks).
- **Declarations:** `pty.rs` declares `mod program;` and `mod stat;` ungated, with `cfg_attr(windows, allow(dead_code, …))`, beside today's `#[cfg(target_os = "linux")] mod unix;`.

**5.9 The app.**
- **`desktop::without_verbatim_prefix(path: &str) -> Option<String>`:** `\\?\UNC\x` gives `\\x`, `\\?\C:\x` gives `C:\x`, and anything else `None`. It sits at the module level of `desktop.rs` with `cfg_attr(not(windows), allow(dead_code))`, and `start_menu::resolved` calls it.
- **`escape_markup`:** moves from `notify/dbus.rs` to `notify/mod.rs` beside `toast_xml`, with `cfg_attr(not(target_os = "linux"), allow(dead_code))` and its test. `dbus.rs` calls `super::escape_markup`.
- **`workspace::program_title`:** it takes `style: PathStyle` and drops a console title only for `PathStyle::Windows`. A console's default title is a Windows program path, which is why the path style decides. Callers pass `process::LOCAL_PATH_STYLE`, which is what `cfg!(windows)` said:
  - `zone_view.rs:491` and `:613`;
  - `workspace.rs:118`.

  Whether a remote tab on a Windows Brindle should use its host's style is a behaviour question (Open Questions).
- **`cmdline::unix_default_program(shell: Option<String>, args) -> (String, Vec<String>)`:** `$SHELL` as given, an empty one included as today, else `/bin/sh`. The `#[cfg(unix)]` `default_program` passes `env::var("SHELL").ok()`.
- **`instance.rs`:**
  - `socket_path_in(runtime_dir: Option<OsString>, cache_dir: Option<PathBuf>, temp_dir: PathBuf, state_dir) -> PathBuf`, where an empty runtime directory is unset;
  - `pipe_name_for(user: Option<String>, state_dir) -> String`, where each character that isn't an ASCII letter or digit becomes `_`;
  - both are ungated, with `cfg_attr(not(unix)/not(windows), allow(dead_code))`. `imp::socket_path` and `imp::pipe_name` gather and call.

### 6. The check: neutral code is never gated

`tests/architecture.rs` gains `the_neutral_code_is_never_gated`, built on change 5's platform-`cfg` matcher and change 7's `production_lines`. It checks two kinds of code:
- **Whole modules:** change 7's `PURE_MODULES` (every whole pure module, the new ones included), plus `NEUTRAL_MODULES`, for ungated modules that aren't whole pure (`remote-protocol/src/server_dir.rs`, which reads the working directory). For each:
  - **its production lines** have no platform `cfg`, except a lint-only `cfg_attr`;
  - **its own `mod` declaration** carries none;
  - **each ancestor's declaration** carries none, or exactly `cfg(any(target_os = "linux", windows))`, which builds on both of Brindle's platforms.

  Every whole pure module meets this today, which a grep of the current list shows. So the check adds no work for change 7's modules.
- **Items:** `NEUTRAL_ITEMS: &[(&str, &str)]`, the file and function of each item that sits in a seam or shell file:
  - `desktop.rs` `without_verbatim_prefix`;
  - `notify/mod.rs` `escape_markup`;
  - `instance.rs` `socket_path_in` and `pipe_name_for`;
  - `workspace.rs` `program_title`.

  Each item's `fn` line starts at column 0, so it isn't inside an inline module. The attributes directly above it carry no platform `cfg`.

Each list has a test that every entry exists. Planted-text tests show the check in both directions:
- **`a_gated_neutral_module_is_found`:**
  - a production `#[cfg(windows)] fn`;
  - a `#[cfg(unix)] mod x;` declaration;
  - an ancestor gated `cfg(windows)`.
- **`a_gated_neutral_item_is_found`:**
  - an item inside `#[cfg(windows)] mod imp {`;
  - one under `#[cfg(unix)]`.
- **`a_lint_only_cfg_attr_and_test_code_do_not_count`.**

The new whole modules join `fcis`'s "Whole pure modules" table, which change 7 keeps in sync with `PURE_MODULES`. `NEUTRAL_MODULES` and `NEUTRAL_ITEMS` are named in `fcis`'s Enforcement section. They are short, and their own existence test holds them.

**Alternatives rejected:**
- **A separate "neutral" table with its own sync test.** A pure module has no reason to carry a platform `cfg`, so reusing `PURE_MODULES` keeps one list.
- **Checking `cfg` sites by item name across whole files.** A text scanner can't follow an item into its module's other items. That is change 7's reasoning.

### 7. Docs

- **`functional-core-imperative-shell.md`:**
  - the new whole modules join the "Whole pure modules" table, in a row "beside a platform seam";
  - R7's paragraph "The other platform's data" names `pipe_state` and `server_dir::check_pipe_name` beside `program_stem`;
  - Enforcement says the new check holds R7 for the whole modules and the named items, and review holds the rest.
- **`ports-and-adapters.md`:** R7's examples add `access::admit_client`, `ready_arg::role_args` and `pty::program`.
- **`.claude/rules/`:**
  - `remote-agent.md`'s `ports R7` line points at the new modules, and "Enforced by" names the check;
  - `remote-protocol.md`'s `fcis R7` line names `access.rs`, `hresult.rs` and `server_dir.rs`.

### 8. Who adapts to whom

The plan's order is 3, 4, 5, 7, then 10, with 11, 12 and 13 after.

| Sibling | Overlap | Rule |
| --- | --- | --- |
| 5 `enforce-platform-seams` | the scanner, the seam set, `PathStyle`, `LOCAL_PATH_STYLE` | 10 needs them; 10 lands after 5 |
| 7 `keep-pure-modules-pure` | `PURE_MODULES`, `production_lines`, the `fcis` table split, `program_title`, `shell_quote`, `windows_default_program`'s probe, `server_dir::choose`, `instance::launch_token` | 10 lands after 7. If 10 lands first, it adds `style` to `terminal_automatic_title` itself, and 7 keeps the argument when it splits `program_title` (7's own risk note) |
| 4 `isolate-test-runs` | `environment_block` leaves `conpty/cmdline.rs` | independent functions; whoever lands second rebases |
| 3 (fuzzing) | `parse_holder_lock` in `fuzz.rs` | unaffected; `holder_socket_stem` isn't a decoder |
| 8 | `notify/mod.rs` | `escape_markup` is one moved function; rebase |
| 11 `deterministic-tests` | sleeps and bounds in the same Windows test modules (`pipe.rs`, `pty.rs`, `listen.rs`, `daemon.rs`) | 11 edits tests that stay in the seams; tests that move here move as they are |
| 12 `surface-swallowed-failures` | outcomes for `prune_helper_copies`, `packaged`, `kill_remaining` | 12 builds on the pure decisions here, adding outcomes to the seams |
| 13 `extract-decisions-into-models` | b13 #2 `millis`, b14 #6 `Input` flags, types R1 in `pipe_state` | 13 may turn `ReadState`'s `eof` and `error` into one enum |
| `port-to-macos` | macOS shell, home, socket directory | its choices become arguments to `unix_default_program`, `pty::program`, `socket_path_in`; no edit to it here |
| `add-pocket-client` | `access.rs` works on values | kept: everything moved in is values |

## Risks / Trade-offs

- **[A moved decision behaves differently]** → Each function's tests are written first, against today's code, from the cases its seam handles. Windows CI runs the seams' existing Windows tests unchanged. Those tests are the end-to-end proof: `ready_handle_arguments`, `paths_are_pipe_names`, `the_program_is_the_command_or_the_command_interpreter`, `the_sessions_environment_picks_the_host`, the copy-race and prune tests, the pipe flood tests and the `kill`/hang-up tests. `windows-e2e` runs the holder, ConPTY and titles end to end.
- **[Extra reads]** → None where a read can block or cost:
  - the passwd entry stays lazy (Decision 4);
  - `GetProcessTimes` per opened candidate runs only during a hang-up;
  - `copy.exists()` is read only after a failed rename;
  - the home directory is read once per spawn with a directory, and only touches passwd when no `HOME` is set anywhere.
- **[Byte-level error texts drift]** → `BadPipeName`'s `Display` and `admit_client`'s messages are pinned by tests that assert the full text. `ensure_dir`'s and `ready_arg`'s texts stay in place.
- **[Win32 constants written as numbers]** → A `#[cfg(windows)]` test compares each with the `windows` crate's constant.
- **[First run on the other platform]** → The new modules' tests run on Linux in CI for the first time, and the Linux ones on Windows. Following `testing R17`, the change isn't reported as verified on Windows until the `windows` job passes.
- **[A shared name hides a platform difference]** → `ReadyArg` keeps both rules, `>= 0` and `!= 0`, and each parser's tests assert the other's flag is refused.
- **[`fcis` table churn with change 7]** → This change adds rows only, after 7's split.

## Migration Plan

- **Order:** each site lands as one commit:
  1. the pure function and its tests;
  2. the seam calls it, and the code it replaces goes;
  3. that site's existing Windows tests still pass.

  Order of the sites: `remote-protocol` first, then the agent, then the app, then the check and docs together.
- **Rollback:** reverting one site's commit restores it. Nothing persisted, wire-visible or user-visible changes.
- **Verification:**
  - Linux: `cargo test --workspace --locked` and `cargo build --release --locked`;
  - Windows CI: `cargo test`, then `windows-e2e`;
  - the new architecture check;
  - `cargo build --locked --workspace` on both platforms with no warnings, which catches a missing `allow(dead_code)`.

## Open Questions

- **Console titles on remote tabs:** should a remote tab on a Windows Brindle use its host's path style? Then a Linux program's title that looks like a Windows program path would be kept. Today it is dropped, so changing it is a behaviour change, for its own change.
- **`win::os`, `event_loop::win32` and `pty::plain`:** they now share `win32_code` but keep three fallbacks. Whether they should be one function is a question about their callers' `io::Error`s, and change 12's outcome types may settle it.
