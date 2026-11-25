# Design

## Context

See proposal.md for why. Vocabulary: "ACP agent", "agent session" and "agent zone" are the new thing; "the helper" and "the terminal server" are `brindle-remote`; "the core" is the `serve` role, never pure code.

This change builds on two changes, both implemented and archived first, and assumes each is as its design says:

- **`agents-sessions`** ("Boundary with `agents-profiles`"): client protocol 4 (`SPAWN_AGENT`, `AGENT_PROMPT`, `AGENT_UNQUEUE`, `AGENT_RESEND`, `AGENT_CANCEL`, `AGENT_PERMISSION`, `TRANSCRIPT` with events as JSON objects, attach and replay through `ATTACH`/`REPLAY_*`, `OK`/`ERROR` keyed by `req`, its refusal texts); the sixth crate `brindle-transcript` (directory `transcript/`, `serde` and `serde_json` only, no feature), exporting `brindle_transcript::{Event, Entry, EntryId, TurnState, StopReason, AgentState, Slot, HoldReason, PermissionOption, PermissionOutcome, ToolKind, ToolStatus, NoticeLevel, Activity, Transcript, FoldError, DecodeError}`, `Event::from_value`, `Transcript::{apply, replay, activity, slot}`); `SessionInfo.agent { activity: client::AgentActivity, name }`; layouts, the resolved directory and a 0×0 size for agent sessions in the listing; the `brindle-fake-acp-agent` test binary; Brindle's offer pinned at 1–3 (`BRINDLE_PROTOCOLS` in `src/remote/local.rs`) with a logged-and-dropped `ToClient::Transcript` arm.
- **`agents-launch-commands`**: `brindle terminal [PROFILE] [-d DIR] [-e …]`, the parsed `Asked { Window, Terminal(TerminalLaunch), Attach(u32) }`, `TabKind { Terminal }`, `ProfileKind { Terminal }` with `Profile::kind()`, `workspace::profile_for(config, kind, name) -> Result<Chosen, KindMismatch>`, the instance request's `kind` field decoded by `Request::asked()` into `Forward`, the reserved word `agent`, and the help text with room for an `AGENT:` section. Its design says how a later change adds `brindle agent` (decisions 2, 4, 5 and 6 there); decision 3 below follows it.

Facts from the code that shape this design:

- **Zones.** `src/zone_view.rs`'s `ZoneContent` has one variant, `Terminal(Entity<TerminalView>)`, and `as_terminal_view()` is infallible. All fifteen of its calls are inside `zone_view.rs`; the rest of the app reaches terminals through `ZoneTreeView::{active_view, active_terminal, terminals, zone_working_directory, zone_records, zone_server_session, status_summary, …}`, which `workspace.rs` calls in about sixteen places (the dump, `--send`, `--find`, the pointer steps, `--wait-for`, the command palette's facts, restore notes, the active directory, the title tick's detection roots, quit and layouts).
- **Profiles.** `src/config.rs`'s `Profile` is a flat struct with `#[serde(default, deny_unknown_fields)]`; `remote` is `Option<RemoteTarget>`, which is itself tagged by `type` inside its inline table. `command`, `args`, `cwd` and `env` already describe an ACP agent. `Profile::description()` feeds the picker and the profile menu (`workspace::profile_rows`).
- **Terminals.** `Terminal` wraps a `Backend::Server { session: remote::local::Session, closing: Closing, host, laid_out }` (Linux and Windows) and emits `TerminalEvent::{TitleChanged, Bell, Notify, Notify99, Exited, Status(Option<Notice>)}`, which `ZoneTreeView` forwards to `Workspace::on_attention`. A session's output reaches the term on the connection's reader thread through its `SessionSink`; events reach GPUI through `EventGate` and are paced (`terminal/pace.rs`).
- **The client.** `remote::local::Session` runs `Start::Spawn` or `Start::Attach` on a thread of its own, keeps a `Phase` (`Starting`, `Left`, `Running`, `Gone`) and an input budget. `SessionSink` (`output`, `note`, `replay`, `exited`, `lost`, `failed`, `foreground`, `replay_size`, `broken`) is what the reader calls, each call under `Connection::deliver`'s `catch_unwind`.
- **Closing.** Nothing in Brindle asks before closing today: `close_zone_by_id`, `remove_tab` and the tab menu's Close act at once. The settings dialog's theme list has the only confirm step (`Mode::ConfirmDelete`), inside its own dialog. `brindle-ui`'s `Dialog` is a modal overlay with title, description, footer, dismiss on escape and backdrop, and focus return (`ui-components`' "Dialogs").
- **Settings dialog.** `settings_dialog/profiles.rs` reads each `[[profiles]]` entry with `try_into::<Profile>()` into `ProfileEntry { profile: Result<Profile, String> }`; `config_edit::write_profile` sets or removes only the keys it knows on an existing table, `move_profile` swaps whole tables and `delete_profile` removes one.
- **`brindle-ui`.** `text_input.rs` is single-line: `EditState::new`, `set_text`, `replace` all pass through `single_line`, and `TextInputElement` shapes one `ShapedLine`. There is no multi-line input, scroll area, collapsible or alert. GPUI 0.2.2's `list` element with `ListAlignment::Bottom` lays out only the items in view (`ListMeasuringBehavior::Visible`) and keeps `logical_scroll_top = None` while scrolled to the end, which is exactly "stick to the bottom".
- **Notifications.** `brindle_term_core::notice::Source` is `Bell`, `Osc9`, `Osc777`, `Osc99`, `Osc7501`; `[notifications]` has one switch per source.
- **Platform gates.** `remote::local` is declared under `cfg(any(target_os = "linux", windows))` in `src/remote/mod.rs`, which is on `ports R4`'s exception row; a new file that names `remote::local` would need a gate of its own.

## Goals / Non-Goals

**Goals:**
- An agent profile opens an agent zone that behaves like a terminal zone wherever a zone's lifetime is concerned: split, quit, crash, restore, reattach, the sessions palette, status dots and notifications; closing differs only in asking while the agent is mid-turn.
- `brindle agent` joins the subcommand form exactly as `agents-launch-commands` prescribes, with nothing reshaped.
- Every decision is in a GPUI-free model or pure function with unit tests; views and the reader thread gather, call and apply (`fcis R1`, `R5`, `R9`).
- No caller of zone content, launch or tab kind can forget the agent kind: every `match` is exhaustive (`types R2`, `railway R3`).
- The whole zone is keyboard-operable and scriptable through the existing debug hooks, with e2e cases against the fake ACP agent on every build, the archives included.

**Non-Goals,** each deferred to its proposal-only change where one exists: markdown (`agents-markdown`); in-zone login (`agents-login`); recovery after an agent exits (`agents-session-recovery`); paging the conversation log (`agents-log-paging`); ACP v2 (`agents-acp-v2`). Also out: ACP `fs`, `terminal`, modes, slash commands; mixing kinds in a tab; find and text selection in a conversation; creating or editing agent profiles in the settings dialog; images or attachments in the prompt; a per-zone UI zoom.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `ProfileKind::Agent`, the `type` key, the agent profile's checks, its description | pure (boundary parse) | `brindle`, `config.rs` |
| `Asked::Agent(AgentLaunch)`, `TabKind::Agent`, the `agent` parser arm, help text | pure | `brindle`, `main.rs` |
| `Request`'s `agent` object, `Forward::Agent`, transcript tests | pure codec | `brindle`, `instance.rs` |
| `profile_for`'s agent arm and default-of-kind rule | pure | `brindle`, `workspace.rs` |
| What an agent spawn asks for (`agent_spawn_request`) | pure | `brindle`, `src/agent/spawn.rs` (new) |
| `split_plan`, kind-aware `layout::group` and `reattach` profile choice, `close_plan` | pure | `zone_view.rs`, `layout.rs`, `reattach.rs`, `src/agent/close.rs` (new) |
| Brindle's one door to the transcript (`agent::transcript`, a re-export) and the activity mappings | pure | `src/agent/transcript.rs` (new) |
| The chat model (`ChatModel`, `AgentPhase`, `Block`, `PromptStatus`, queue, first prompt, status and notice rules, dump and wait text) | GPUI-free model | `src/agent/model.rs` (new) |
| The fold shared with the reader thread (`AgentShared`: the `Transcript` and its `Changes`) | GPUI-free model | `src/agent/shared.rs` (new) |
| Agent sessions in the client (`Start::SpawnAgent`, `Session::{prompt, unqueue, cancel, answer}`, the sink's `transcript`/`reply`, the protocol gate) | shell (threads) | `src/remote/local/session.rs`, `connection.rs`, `local.rs` |
| The one platform-neutral entry point to it (`remote::agent_link`) | shell, gated | `src/remote/mod.rs` (already on `ports R4`'s row) |
| `Agent` entity and `AgentView` | shell (entities) | `src/agent/mod.rs`, `src/agent/view.rs` (new) |
| `ZoneContent::Agent` and its callers, the close confirmation overlay | shell | `zone_view.rs`, `workspace.rs` |
| `Source::Agent` | pure types | `brindle-term-core`, `notice.rs` |
| `EditState`'s lines mode, vertical motion | pure | `brindle-ui`, `text_input.rs` |
| `TextArea`, `Textarea`, `MessageScroller` (+ pure `Follow`), `Message`/`Bubble`, `PromptInput`, `Reasoning`, `Tool`, `Confirmation`, `Alert` | components | `brindle-ui`, `ui/src/text_area.rs`, `ui/src/components/*` |
| The fake agent's lookup and freshness check | test harness | `tests/common/fake_agent.rs` (new) |

No crate reference changes in this change (`ports R1`): `agents-sessions` already makes Brindle depend on `brindle-transcript`; the ACP reducer and the schema crate live in `brindle-remote-agent`, which Brindle never references. `brindle-ui` stays on `gpui` and `log` (`ports R2`, `R3`). The new whole pure modules (`src/agent/{spawn, close, transcript, model, shared}.rs`) join `fcis`'s pure-module table and `PURE_MODULES`.

## Decisions

### 1. What the user's confirmed decisions mean here

The user confirmed every decision of this section (rounds 1 to 4 of the brief); the alternatives are kept as considered and rejected.

- **Hosted by the terminal server; the core is the ACP client.** Brindle never runs an ACP agent itself and never parses ACP: it starts an agent session with `SPAWN_AGENT`, then only sends prompts, resends, unqueues, cancels and permission answers and folds transcript events. An agent zone has the lifetime of a terminal zone through the same `Session` machinery (decision 7), and a remote agent profile runs its agent on the endpoint through the existing relay with no new path.
- **Attach replays the log; the log is Brindle's own transcript events.** The zone's state is a fold of those events with `Transcript::apply`, the core's own fold; a replay folds to the server's log. Nothing here keys on ACP's method names, so `agents-acp-v2` changes nothing in this change's code.
- **ACP v1 only,** through `agents-sessions`; the transcript is shaped so v2 fits (`agents-acp-v2`).
- **One ACP agent process per zone;** a split starts another agent session in the split zone's directory (decision 4). *Rejected:* several zones sharing one agent process, which ACP's one-session-per-`session/new` model doesn't need and which would tie zones' lifetimes together.
- **Permissions and tool calls.** A permission request is a row with one button per option the agent offered and nothing chosen for the user; a tool call is one status line. Requests wait with no timeout; `agents-sessions` caps them at 16 pending per session and answers further ones cancelled with a notice, which the zone shows as any notice.
- **Closing asks when the agent is mid-turn; quitting never asks** (decision 12), and the dialog's default button is Cancel: enter and escape keep the agent. *Rejected:* never asking, as terminals don't (round 1's proposal), since an agent's turn is work the user can't get back by reattaching; Close as the default, which an enter pressed by reflex would turn into lost work.
- **A queue of one prompt, kept by the server in a slot** (decision 9): a prompt sent during a turn waits and is sent only after a turn that ends normally (`EndTurn`, shown as `done`); every other ending, a stop, the agent stopping and a truncated restart hold it, shown as not sent with send-again and discard buttons. **A prompt that isn't sent is kept apart from the draft:** nothing from the slot ever goes into the prompt input. **A new prompt while one is held** is sent when no turn runs (the held one stays held) and refused while a turn runs. *Rejected:* refusing every prompt during a turn (round 1); handing an unsent prompt back into the input as a draft (round 3), which mixed it with what the user was typing and couldn't survive a detach; sending a queued prompt after any ending but a stop (round 2); refusing new prompts until a held one is dealt with, or letting a new one replace it.
- **Status mapping:** a normal end of turn is `done`; a refusal or a token or request limit, a failed turn, a login demand, an ACP failure and an exit are `error`; a cancelled turn is `idle`; a waiting permission is `blocked`; `done` and `error` leave the dot once the user types into that zone's prompt input (decision 10).
- **Failures:** a login demand, at start or from a prompt after the agent was ready, shows the agent's message and points at its own command line, with the prompt input disabled and the conversation kept readable until the profile is opened anew (in-zone login is `agents-login`); an agent that doesn't speak ACP version 1 is ended by the server and its zone keeps the version message as its final state; an agent that exits leaves its zone open with the exit status and the end of its standard error, with no recovery (`agents-session-recovery`); a Windows agent that is a batch file needs `cmd /c …` in the profile, and the zone shows the spawn error's hint with that form (decision 13).
- **Plain text** for messages (`agents-markdown`); `ui/`'s closed list (`gpui`, `log`) can't take a markdown parser without that change's decision.
- **The log** is capped at 16 MiB by the core with the oldest entries trimmed and a marker; the zone mirrors it (decision 8); paging is `agents-log-paging`.
- **The keys:** enter sends, shift-enter adds a line, escape stops the turn, tab walks the prompt, the queue row's buttons (remove, or send again and discard), the permission buttons and stop; shift-pageup/pagedown scroll, in the `AgentZone` key context (escape, the page keys) and the `PromptInput` and `TextArea` contexts (enter, shift-enter); thought triggers, copy buttons and the scroll button are pointer-only, with palette actions as their keyboard path (decisions 9 and 11).
- **The settings dialog** lists agent profiles, marked, and never alters or drops them; editing is config-file only (decision 14). *Rejected:* an agent editor in the dialog, left for later.
- **A new `notifications.agent` key,** on by default (decision 10). *Rejected:* reusing `osc7501`, which would make that key silence something that never prints an OSC.
- **An agent tab whose agent profile is gone** shows its conversation and can't be split (decision 4). *Rejected:* starting splits from the listed session's command, which the listing doesn't carry with its environment.
- **An older Brindle reading a config with an agent profile** rejects the whole config and runs on defaults (`process R15`); the release notes say so (decision 2).
- **`brindle agent [PROFILE] [-d DIR] [-p PROMPT]`** on top of `agents-launch-commands`; bare `brindle` opens the default profile whatever its kind; a profile of the other kind is a usage error; debug, informational options and the hand-off work with either subcommand (decision 3). *Rejected:* round 1's `-e` with an agent profile running the given command as the agent.
- **An unknown profile name is a usage error** under either subcommand: status 2, listing the profile names, checked by the Brindle that would open the tab (`agents-launch-commands` decision 5). *Rejected:* falling back to a default profile with a note in the zone.
- **`-p -` reads the first prompt from standard input** (decision 3); `-p TEXT` is for ordinary prompts. *Rejected:* only `-p TEXT`, which can't carry a long prompt (Linux refuses one argument over 128 KiB; Windows caps the command line near 32 KiB); a `--prompt-file`, which `-p - < file` already covers.
- **Directory:** an agent launched by `brindle agent` with no `-d`, of a profile with no `cwd`, starts in the working directory of the `brindle agent` command; one opened from the profile menu, the picker, a shortcut or as the default profile starts in the profile's `cwd`, else the home directory. *Rejected:* the home directory for `brindle agent` too, which would make `brindle agent` from a project's shell start somewhere else.
- **A `-p` prompt not yet sent when Brindle quits is dropped,** never recorded and never sent after a restore. *Rejected:* recording it in the session state with a rule for when a restored zone may send it.
- **Two changes plus a third.** Everything server-side is `agents-sessions`', the launch grammar `agents-launch-commands`'. Where this change needs more from them, it says so ("What the sibling changes lack").

### 2. The profile kind (`types R1`, `R6`, `R7`; `process R15`)

`agents-launch-commands` adds `ProfileKind { Terminal }` and `Profile::kind()`. This change adds the variant and the key:

```rust
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind { #[default] Terminal, Agent }

pub struct Profile { #[serde(rename = "type")] kind: ProfileKind, /* the existing fields */ }
impl Profile { pub fn kind(&self) -> ProfileKind { self.kind } }
```

- **Parsed at the boundary.** `type` is a closed enum, so `type = "chat"` is a serde error naming `terminal` and `agent`, on the config line, with defaults used, as any invalid value is. An agent profile with no `command` is rejected in `Config::parse` (`profile "X": an agent profile needs a command`), the same treatment as an SSH target without `host`. Past the boundary, code asks `profile.kind()`, never `command.is_none()`. The `type` key doesn't clash with `remote = { type = "ssh" }`: that one is inside the inline table. Every exhaustive `match` `agents-launch-commands` wrote on `ProfileKind` (its `profile_for`) fails to compile until it handles `Agent`.
- **Not a struct per kind.** Every other field means the same for both kinds, and the settings dialog, `config_edit`, restore and the picker all handle `Profile`; splitting it would touch all of them for no invariant beyond "an agent has a command", which the parse holds. The removed `tmux*` keys stay accepted on either kind.
- **The description** becomes `[<target> · ]agent · <command>` for an agent, unchanged for a terminal.
- **The environment.** An agent spawn sends the same complete environment a terminal spawn does (Brindle's, then the profile's `env`, merged case-insensitively on Windows by `cmdline::merge_environment`), minus the variables that describe a terminal (`TERM`, `COLORTERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, and Brindle's shell-integration variables). `agent_spawn_request` is pure and tested.
- **An older Brindle and a config with an agent profile.** `deny_unknown_fields` makes `type` an unknown key there: that Brindle shows the error on its config line and runs on the default config, as `process R15` chooses for unknown keys. Its settings dialog lists the profile as unreadable and offers no edit, so it doesn't damage it either. README's release note says so.

### 3. `brindle agent` (`types R1`, `R2`; `fcis R1`; `process R4`–`R6`; `command-line`)

Added exactly where `agents-launch-commands` left room for it.

**Grammar and parsed form.**

```
SUBCOMMAND := terminal [PROFILE] [-d DIR] [-e CMD [ARG]...]
            | agent [PROFILE] [-d DIR] [-p PROMPT]           (any order)

enum Asked { Window, Terminal(TerminalLaunch), Agent(AgentLaunch), Attach(u32) }
struct AgentLaunch { profile: Option<String>, cwd: Option<PathBuf>, prompt: Option<PromptArg> }
enum PromptArg { Text(FirstPrompt), Stdin }   // the parser's form; main turns Stdin into Text
struct FirstPrompt(String);   // non-blank UTF-8, ≤ 256 KiB; built only by FirstPrompt::new (types R6)
enum TabKind { Terminal, Agent }
```

- The parser's `agent` arm replaces the reservation refusal. Under `agent`: `-d`/`--working-directory`, `-p`/`--prompt` (one value, so it can come anywhere), PROFILE as the first bare word. `-p -` is `PromptArg::Stdin`; any other value goes through `FirstPrompt::new`. `-e`/`--command` under `agent` is a usage error ("an agent runs its profile's command; -e belongs to brindle terminal"). `-p` before the subcommand stays `agents-launch-commands`' removed-option error, and `-p` under `terminal` its "name the profile directly" error, so `-p` means a prompt only after `agent`. A blank prompt, one over 256 KiB (`agents-sessions`' prompt limit), a second PROFILE, and a second `-p` (two `-p -`, or `-p -` with `-p TEXT`, in either order) are usage errors naming them. Every `match` on `Asked`, `TabKind` and `PromptArg` gains its arm with no `_ =>`.
- **Help text:** `brindle agent [PROFILE] [-d DIR] [-p PROMPT] [OPTIONS]  open an agent tab` under USAGE, and an `AGENT:` section (`PROFILE` an agent profile's name, default as below; `-d, --working-directory <DIR>`; `-p, --prompt <PROMPT>` a first prompt, sent once the agent is ready, `-` to read it from standard input).

**`-p -`: the prompt from standard input (confirmed by the user).** The launching process reads it, in `main`, after the parse and before the config check, the hand-off or any window, so every outcome below is a usage error (status 2) with nothing opened:
- **Reading** is the shell's (`process R13` doesn't apply: no window exists yet). `main` checks the handle first: on Linux `isatty(0)` (through `std::io::IsTerminal`), so `-p -` in an interactive shell is refused at once rather than waiting for typing ("-p - reads the prompt from standard input, which is a terminal; redirect a file or pipe into it"). It reads at most 256 KiB + 1 bytes, so a huge or endless input stops there ("the prompt is over the 256 KiB limit"); the limit is `agents-sessions`' prompt limit, so nothing a launch accepts is refused by the server for its size.
- **Validating** is pure: `FirstPrompt::from_stdin(bytes) -> Result<FirstPrompt, PromptError>` with `PromptError::{Absent, TooLong, NotUtf8, Blank}`, unit-tested; one trailing line ending (`\n` or `\r\n`) is dropped, since files and here-strings end with one; nothing else is changed.
- **The hand-off.** A launch handed to a running Brindle carries the prompt in its request, which must pass `agents-launch-commands`' `instance::fits` (1 MiB, line ending included, not raised). The size counted is the encoded request, so JSON escaping counts: 256 KiB of ordinary text fits with room; a prompt of control characters, escaped as `\u00XX`, can exceed it (6 × 256 KiB). Then `fits` refuses before connecting, status 1, with its own message, as for any oversized launch. This change adds no size check of its own.
- **Windows.** `brindle.exe` is a GUI-subsystem program, but standard handles a parent passes reach it: `CreateProcess` with `STARTF_USESTDHANDLES` gives a GUI child the redirected handles, and `win::attach_parent_console` already keeps a handle that is set rather than replacing it (its comment: "A handle that is already set (output redirected to a file or pipe) is kept"). So cmd's `brindle agent A -p - < task.md`, PowerShell's `Get-Content -Raw task.md | brindle agent A -p -` (PowerShell passes a pipe to a native program's standard input) and `std::process::Command::stdin` all reach it. With nothing redirected, a GUI program started from a console usually has no standard input handle (`GetStdHandle` null or invalid): that is `PromptError::Absent`, refused as "-p - needs a file or pipe on standard input". A parent that passes its console's input handle instead is caught by the same `IsTerminal` check as on Linux (a console handle is a terminal), so neither case waits for typing. Both checks are the shell's, in `main`, through `std::io::IsTerminal` and a `win.rs` helper for the absent handle; no new platform `cfg` outside `win.rs`. The e2e case `windows-agent-prompt-stdin` runs through the harness's `Command::stdin(file)` and through `cmd /c "… < file"`; PowerShell's pipeline is a manual check, since the harness can't run it without asking PowerShell to wait for a GUI program (`CLAUDE.md`'s Windows notes).

**The profile check and the default of a kind (`agents-launch-commands` decision 5).** `profile_for(config, kind, name, config_error) -> Result<usize, LaunchRefusal>` gains the agent arm of its exhaustive `match (kind, profile.kind())` and a third refusal: `LaunchRefusal { UnknownProfile { name, known, config_error }, KindMismatch { profile, kind }, NoProfileOfKind { kind, known, config_error } }`. A named profile of the other kind is `KindMismatch`, naming the profile, its kind and the subcommand that opens it. An unknown name is `UnknownProfile`, listing the profile names, with no fallback (confirmed by the user). Only a launch that names no profile uses a default: the default profile when its kind matches, else the first profile of the kind in config order, else `NoProfileOfKind` ("the config has no agent profile (add one with type = \"agent\")", listing the profiles it has). `agents-launch-commands` leaves "default of the kind" to this change; this is the answer. Every refusal is a usage error: status 2 before any window for a launch that would become the instance, and the instance's `usage: TEXT` answer for a handed-off one, which a launch of this version prints and exits 2 on. Bare `brindle`, ctrl-shift-t and the `+` button open the default profile whatever its kind, as confirmed. A launch never adds a note to a zone: `ZoneContent::note` and the dump's `--- note` line exist only for the restore's notes (a restored tab whose recorded profile is gone, `missing_profile_text`), which an agent zone shows as a client-side notice row at the top of its conversation.

**The directory (confirmed by the user).** `-d` wins; else the profile's `cwd`; else, for `brindle agent`, the working directory of the `brindle agent` command, sent with the request as a terminal's `-e` command's is. A tab opened from the profile menu, the picker, a shortcut, ctrl-shift-t or as the default profile at launch, with no launching shell of its own, uses the profile's `cwd`, else the home directory (on a remote endpoint, the home directory there).

**The first prompt.** `ChatModel` gets `first: Option<FirstPrompt>` at construction, from the launch only. The model sends it (`Submit::Send`) on the first live `Agent { Ready }` it folds, never from a replay, and drops it once sent. If the phase becomes `NotStarted`, `LoginRequired`, `Failed` or `Exited` first, the text becomes the prompt input's draft, unsent, so nothing is lost and nothing is sent on its own later; this is the one place text goes into the input that the user didn't type, and it never comes from the slot. It is never recorded: the session state file has no field for it, and a restored or attached zone has no `first`. Once sent it is a `User` entry in the transcript, which a replay shows as history and nothing sends again. A prompt not yet sent when Brindle quits is dropped (confirmed by the user). A refused send (the server's `ERROR`) leaves it as the draft with the refusal shown, as any prompt.

**The hand-off.** `Request` gains `agent: Option<AgentRequest { profile: Option<String>, cwd: Option<PathBuf>, prompt: Option<String> }>` with `"kind":"agent"`, and fills none of `profile`, `cwd`, `command`, as `agents-launch-commands` decision 6 requires. The prompt is always text by then (`-p -` was read by the launch). `Request::asked()` decodes it to `Forward::Agent(AgentLaunch)`, checking the prompt again through `FirstPrompt::new`. The instance runs `profile_for` against its own config, as for terminals, and answers any `LaunchRefusal` with `usage:`. The launch exits 0 once the instance accepts the request; the first prompt is the instance's to send. What meets:

| Launch → instance | Result |
| --- | --- |
| this, `brindle agent …` → this | agent tab; the instance sends the first prompt when ready; a refusal is `usage:`, exit 2 |
| this, `brindle agent …` → `agents-launch-commands` only | refused by name ("can't open agent tabs"), `error:`; launch exits 1 |
| this, `brindle agent …` → older than that | no field it reads: it raises its window only; exit 0 |
| this, `brindle terminal …` → any | as `agents-launch-commands` says |

Transcript tests in `instance.rs` pin an agent request with each field, its decoding, a request with `kind` `agent` and no `agent` object (refused: "an agent request needs its agent fields"), each `LaunchRefusal` as a `usage:` answer, and the older-instance decoding of the agent request (string literals from `agents-launch-commands`' tests). The `process R4` exception row `agents-launch-commands` adds covers this additive field unchanged.

**Debug hooks with it.** The debug options work with `brindle agent` as with `terminal` (`agents-launch-commands`' "Options around the subcommand"), so an e2e case runs `brindle agent Fake -p hi --wait-for 'Hello!' --dump-screen-after 2`; `Run` gains `.agent(profile)`, `.prompt(text)` and `.stdin(file)` beside its `terminal` form.

### 4. A tab's kind, and what a split starts (`types R1`, `R8`, `fcis R1`)

```rust
pub enum SplitPlan { Terminal, Agent, Refused(SplitRefusal) }
pub enum SplitRefusal { ProfileGone, ProfileKindChanged }
pub fn split_plan(tab: TabKind, profile: Option<ProfileKind>) -> SplitPlan
```

- `ZoneTreeView` gains `kind: TabKind` (`agents-launch-commands`' type; one kind type for launches and tabs), fixed when the tab opens, as its `host` is: from the profile for a new tab, from the session (`SessionInfo.agent` is set) for an attached or swept tab, from the profile for a restored tab.
- A split asks `split_plan(self.kind, the profile as the config has it now)`. A tab whose profile changed kind, or is gone (an agent session attached under a terminal profile because no agent profile exists), refuses: `can_split` returns false, so the header buttons and menu items are disabled.
- `layout::group` (pure) keeps a recorded layout's zones only when their sessions are of the active zone's kind; the others become single tabs.
- `reattach::session_profile` and `local_profile` take the session's kind: for an agent session they prefer, among the profiles the existing rules would consider, the first that is an agent profile, and fall back to the existing answer otherwise.

### 5. `ZoneContent::Agent` and every caller (`types R2`, `R12`)

`ZoneContent` gains `Agent(Entity<AgentView>)`, and `as_terminal_view()` is removed: each question becomes a method of `ZoneContent` with an exhaustive `match`. `active_view()` becomes `active_terminal_view() -> Option<Entity<TerminalView>>`, and `active_terminal()` stays `Option`, both `None` for an agent zone.

| Caller (today) | Agent zone |
| --- | --- |
| `on_zone_terminal_event` `Status` → zone status, progress, badge | from `Agent::status()` (decision 10); progress and badge `None` |
| `focus_handle` | the `AgentView`'s, which hands focus to the prompt input |
| `active_view` / `active_terminal` / `active_grid` (dump, `--send`, `--find`, pointer steps, the find bar, `describe_focus`, `select_all`) | `None` for the terminal paths; the dump, `--send`, `--wait-for` and the focus line get agent paths (decision 15). `--find`, `--hover`, `--click`, `--press`, `--release` log "no active terminal" |
| `zone_working_directory` | the directory the session was started in (`Agent::directory`, from the spawn request or the listed `SessionInfo.cwd`) |
| `zone_records` / `shows_only_message` | an agent zone whose session has ended (`SessionLife::Ended`: an exit, or a version mismatch's `Failed`) or whose phase is `NotStarted`, `Lost` or `Broken` isn't recorded; it shows only a message when it has no conversation |
| `zone_server_session` | `Agent::server_session()` |
| `refresh_zone_title` | `ChatModel::title(profile name)`: the agent's name from `Ready`, else the profile's |
| `status_summary` | the zone's shown status joins the max |
| `set_profile` | the view takes the theme for its card and the font size for its text |
| `terminals()` → `shutdown_all`, `quit_all`, `send_layout`, `detection_roots` | `ZoneContent::{close, quit_with(layout), set_layout(layout)}`; `detection_roots` terminals only |
| `zone_outer_size` | the estimate from the tree's rectangles |
| `close_zone_by_id` | goes through `close_plan` first (decision 12), then `ZoneContent::close` |
| `render_leaf` | the card holds the `AgentView`; header, ring, status dot and buttons as for a terminal |
| `on_modifiers_changed` | terminals only |
| `insert_terminal_zone` | `insert_zone(id, ZoneContent, …)`, subscriptions per variant |
| `on_zone_terminal_event` `Exited` | never emitted by an `Agent`; the zone stays |
| workspace's `restore` note (`terminal.note`) | `ZoneContent::note`: a client-side notice row |
| palette `PaletteFacts::prompt_marks` | `false` |
| latency probes | none |
| right-click on the content | none; paste reaches the prompt input through its keys |

The `Agent` entity emits the existing `TerminalEvent` (`TitleChanged`, `Status(Option<Notice>)`), so `ZoneTreeView` and `Workspace::on_attention` keep one path for both kinds.

### 6. Brindle's offer goes to 4, and when (`process R4`, `R5`, `R12`)

This change raises `BRINDLE_PROTOCOLS` to `client::PROTOCOLS` (1–4) in task 6.5, after the sweep, restore, `--attach` and the sessions palette tell agent sessions apart (6.4): a Brindle offering 4 lists agent sessions, and one that can't show them would open them as broken terminal tabs.

- A newer Brindle and a core speaking 1–4 agree on 4. With an older core that can't be upgraded they agree on 3; that connection never sends `SPAWN_AGENT`, and an agent zone on it is `NotStarted(TooOld { endpoint })`. The gate is pure: `agent_start(protocol) -> Result<(), StartFailure>`, on the session's thread, before anything is sent.
- No message of Brindle's own is added to the client protocol; its transcripts are `agents-sessions`'.

### 7. Agent sessions in the client (`process R3`, `R9`, `R13`, `R16`, `R17`)

An agent session uses `remote::local::Session`, so connecting, reconnecting, upgrading, the request table, ownership, `QUIT_FLUSH` and the abandon path are shared.

- `Start` gains `SpawnAgent(Box<dyn FnOnce() -> AgentSpawnRequest + Send>)`; the thread checks the gate, sends `SPAWN_AGENT`, and handles `SPAWNED`/`ERROR` as for `SPAWN`. `Start::Attach` is unchanged (the listed 0×0 size goes back in `ATTACH`, which the core ignores).
- `Session` gains `prompt(text)`, `resend()`, `unqueue()`, `cancel()` and `answer(entry, option)` (`AGENT_PROMPT`, `AGENT_RESEND`, `AGENT_UNQUEUE`, `AGENT_CANCEL`, `AGENT_PERMISSION`), sent only while `Running`. `input` and `resize` aren't called for an agent session (the core refuses `RESIZE` for one).
- `SessionSink` gains `transcript(&mut self, events: Vec<serde_json::Value>)` and `reply(&mut self, request, outcome)`. The reader routes `TRANSCRIPT` by session id under `Connection::deliver`'s `catch_unwind` (`process R3`). The agent's sink decodes each value with `Event::from_value` and folds it; a `DecodeError`, a `FoldError` or a panic makes the session `Broken`, detached and still running, as a terminal's emulator failure is. Since `agents-sessions` freezes event forms per protocol version (unknown fields ignored, an unknown kind or value a `DecodeError`), a correct core never sends a protocol 4 client an event it can't decode, so a `DecodeError` means a bug or a damaged stream, never a newer core. `agents-sessions` leaves the reaction to the client; Brindle leaves the reattach to the user (Attach Session…), which can't loop on a fold that keeps failing.
- **The fold runs on the reader thread, the view reads a copy** (decision 8), through one merged `Changed` marker plus life events (replay start and end, exit, lost, failed, broken, replies) in an `EventGate`, paced by `pace::step` (`process R13`). At most one `Changed` waits, and life events are bounded by the session's requests and its one lifetime: `process R9`'s list gains this queue with that bound.
- **The platform gate.** `src/remote/mod.rs` (on `ports R4`'s row, already gating `local`) gains one item, `agent_link(host, start, sink) -> Result<AgentLink, Unsupported>`, whose other-platform arm fails with "agent sessions run in the terminal server, which needs Linux or Windows". `src/agent/` has no platform `cfg`.

### 8. The GUI-free chat model, and where `brindle-transcript` is named (`fcis R1`–`R3`, `R5`, `R8`; `types R1`, `R3`; `process R14`)

**One door to the transcript crate.** Brindle names `brindle_transcript` in exactly one module, `src/agent/transcript.rs`: `pub use brindle_transcript::{…}` for the types the Context lists, plus the two pure activity mappings, `status_of(Activity) -> State` (live zones) and `listed_status(client::AgentActivity) -> State` (the sessions palette, sweep), both exhaustive. Every other file uses `crate::agent::transcript::…`. `tests/architecture.rs` gains a check that no other file in `src/` names `brindle_transcript`, so the crate's name and its paths change in one place.

**`AgentShared`** (`src/agent/shared.rs`), behind an `Arc<Mutex<_>>` the sink and the entity share (a leaf lock: nothing is taken under it):
- `transcript: Transcript`, `replaying: bool`, `changes: Changes { added_from, changed: BTreeMap<EntryId, Dirty>, trimmed_through, slot_changed, turn_or_agent, reset }`, where `Dirty::Appended { from_byte }` or `Dirty::Replaced` lets the UI copy only a streaming entry's new tail. `REPLAY_START` sets `reset`.
- `apply(values) -> Result<(), Broken>` decodes and folds; `take_changes()` hands them over. An `Event::Queue { slot }` sets `slot_changed`; the model then reads `Transcript::slot()`, the whole slot, so live and replayed slots take one path (a replay emits the slot whenever it isn't `Empty`).

**`ChatModel`** (`src/agent/model.rs`), on the UI thread:
- `phase: AgentPhase`:
  ```rust
  enum AgentPhase {
      Connecting,
      NotStarted(StartFailure),            // no server, refused (with the server's text, a Windows hint included), too old
      Live(AgentState),                    // Starting, Ready, LoginRequired, Failed, Exited
      Lost { reason: String },
      Broken { reason: String },
  }
  enum StartFailure { NoServer(String), TooOld { endpoint: String }, Refused { command: String, cause: String } }
  enum Link { Up, Reconnecting }            // orthogonal: a reconnect keeps the conversation shown
  enum SessionLife { Open, Ended }          // orthogonal: Ended on the session's exit life event
  ```
  The agent state shown is always the last `Agent` event folded; the session's exit only sets `SessionLife::Ended`. So an agent `Failed` for a version mismatch, which `agents-sessions` follows with no `Exited` (decision 4 there), stays `Live(Failed)` with its message after the session ends, and its standard error arrives as an ordinary `Notice` entry in the conversation. `Live(Exited)` comes only from an `Exited` event. Nothing here infers why a `Failed` happened: the zone words it from `SessionLife` (decision 13).
- `blocks: Vec<Block>`: an entry becomes one or more blocks; a message is split at blank lines and, past 4 KiB, at the next line break (`blocks_of`), so a long streaming message re-lays out only its last block. `BlockBody` is `Text`, `Thought { open }`, `Tool { kind, title, status }`, `Permission { title, options, outcome }` or `Notice { level, text }`, holding `SharedString` (plain data, no entity or context, as `fcis R5` means).
- `queue: QueueRow`, the view of the slot: `QueueRow::{None, Waiting { text }, Held { text, why: SharedString, resend: Resend }}`, built by the pure `queue_row(slot, phase, life)`. `why` is the hold reason in words, from an exhaustive `match` on `HoldReason` with no `_ =>` (`railway R3`): `Cancelled` "you stopped the turn", `Failed` "the turn failed", `Stopped { stop }` "the turn ended: max_tokens", `LoginRequired` "the agent needs a login", `AgentStopped` "the agent stopped", `OutputLost` "output was lost while the terminal server restarted". `Resend::{Available, Unavailable(why)}`, from an exhaustive `match` on the phase and its `AgentState`: available only for `Live(Ready)` with `SessionLife::Open`; unavailable while starting ("the agent is starting"), for `LoginRequired` ("the agent needs a login; open the profile again after logging in"), for `Failed` ("the agent failed"), for `Exited` and any ended session ("the agent has exited"), and for `Connecting`, `NotStarted`, `Lost` and `Broken`. So a held prompt with `LoginRequired` shows send again disabled with that tooltip, never a server refusal; discard stays available.
- `absorb(&mut self, shared, changes) -> Absorbed { splices, status, notice, title_changed, queue_changed }`. The view applies the splices to its `ListState` and the row to its `PromptInput`. Nothing in `Absorbed` touches the prompt input's text. `render` reads the model only, never the lock (`process R14`).
- **Prompt decisions:** `prompt_status(draft_blank, composing) -> PromptStatus { Ready, Submitted, Streaming, Disabled(DisabledWhy) }` and `submit(draft, composing) -> Submit { Send(String), Nothing(Why) }`. During a turn with an empty slot, `Send` (the server queues it). While a turn runs and the slot isn't empty (waiting or held), `Nothing(AlreadyQueued)`; with no turn running, a held prompt doesn't block a new one (`Send`, the held one stays held). The local check is the server's rule (`agents-sessions` decision 10), so the server's refusal is only the backstop. Blank, composing, not ready, a prompt in flight, the link down: `Nothing`. `replied(request, Ok)` clears the draft; `Err(why)` keeps it with `refusal: Some(why)`. The user's message and the row appear from `Added { User }` and `Queue { slot }`, never optimistically.
- **The queue's requests:** `unqueue()` (a waiting or held prompt) and `resend()` (a held one, when `Resend::Available`) return the request to send, or `None` when there is nothing to do; the row changes only when the server's `Queue { slot }` arrives. After the session has ended (decision 9), `unqueue()` clears the row locally and sends nothing.
- **First prompt:** decision 3.
- **Thoughts** open while the latest entry and streaming, closed once another follows, unless the user toggled it.
- **Permissions:** options in the agent's order with allow kinds first; buttons keyed by `(EntryId, option id)` (`types R3`); `answer` refuses an answered entry.
- **Title, status, notices, dump and wait text:** decisions 10 and 15.

**The last decisions of `agents-sessions` (rounds 3 and 4, confirmed by the user).**
- *The sixth crate holds only the transcript* (`brindle-transcript`, no feature); the reducer and the schema crate are the helper's. Brindle depends on that crate alone, so the schema crate never enters Brindle's build graph through it. *Rejected:* one shared crate holding the ACP code as well, behind a cargo feature only the helper enables.
- *A queued prompt that isn't sent is held in the server's slot, apart from the draft* (round 4), decision 9.

Memory: the zone holds a copy beside the fold, so at most about twice the core's 16 MiB `LOG_LIMIT`; `Trimmed` drops blocks through the id.

### 9. The queue of one, in the zone (`agent-zones`)

The server keeps the slot and decides every transition (`agents-sessions` decision 10, confirmed by the user); the zone shows `Transcript::slot()` and sends requests. No code here encodes which endings send or hold.

- **The row** sits above the prompt input inside `PromptInput` (decision 11), from `QueueRow` (decision 8):
  - `Waiting`: muted, the first line of the text cut with an ellipsis, a `queued` label, and a ghost `IconXs` remove button (lucide `x`, tooltip "Remove queued prompt") that sends `AGENT_UNQUEUE`.
  - `Held`: the text as above, a `not sent` label in the warning colour with the reason after it ("not sent: you stopped the turn"), then two ghost `IconXs` buttons: send again (lucide `send`, tooltip "Send again") sending `AGENT_RESEND`, and discard (lucide `x`, tooltip "Discard") sending `AGENT_UNQUEUE`.
  - Every button is a focus stop while the row shows, between the prompt input and the permission buttons (decision 1's key order), activated by enter or space; the palette actions are `agent_unqueue` ("Remove Queued Prompt", which also discards a held one) and `agent_resend` ("Send Held Prompt Again"), each disabled when it has nothing to act on.
- **Nothing goes to the draft.** The row never writes the prompt input's text, and the input never reads the slot. To reuse a held prompt's words in another prompt, the palette action `copy_queued_prompt` ("Copy Queued Prompt") puts its whole text on the clipboard, for the user to paste where they choose.
- **A new prompt beside the slot** follows the server's rule, which `submit` repeats locally: during a turn with the slot taken (waiting or held), enter keeps the text with "a prompt is already queued" beside it; with no turn running, a held prompt doesn't block: the new one is sent and the held one stays.
- **Send again when the agent isn't ready** is disabled (`Resend::Unavailable`) with its tooltip saying why ("the agent isn't ready"); discard still works. The server's refusal ("the agent isn't ready", "no prompt is held") is the backstop, shown as the prompt input's refusal.
- **After the session ends** (`SessionLife::Ended`: the agent exited, or was hung up for a version mismatch), the session has ended on the server, so nothing can be sent to it: the zone keeps the held row from its own copy of the transcript, send again is unavailable ("the agent has exited"), and discard clears the row locally (`ChatModel::unqueue` sends nothing). The same holds for `Lost`. The zone and its row go when it is closed.
- **Restored, replayed and quit:** the slot is the server's, so a replay emits it when it isn't empty, a quit and relaunch shows a waiting or held prompt as it was, and a reconnect or upgrade keeps it.
- **Closing** a zone with a waiting or held prompt: the slot alone doesn't make the zone busy (decision 12); a waiting prompt exists only during a turn, which does. A held prompt goes with the session and isn't mentioned.
- **A login demanded by a prompt** (`agents-sessions` decision 17): the turn ends `Failed` with the agent's message, a waiting prompt becomes `Held { LoginRequired }`, then the agent becomes `LoginRequired`. The row shows "not sent: the agent needs a login", send again disabled, discard working (`AGENT_UNQUEUE` is still accepted).
- **The dump** prints `--- queued waiting "next"` or `--- queued held reason=cancelled|failed|stopped:<stop>|login-required|agent-stopped|output-lost resend=yes|no "next"` (decision 15).

### 10. Status and notifications (`fcis R1`; `tabs`, `desktop-notifications`)

- **Mapping:** `status_of` maps `Activity` one-to-one onto `State`; `listed_status` maps `AgentActivity` the same way for listings. Phases outside `Live` have no status. `Idle` draws no dot.
- **Seen:** `done` and `error` are dropped from the shown status when the user types into the prompt input after they arrived, until the activity changes (`ChatModel::seen()`, from the text area's `Changed`). The server's activity is unchanged.
- **Notices:** `ChatModel::notice(prev, next, replaying)`: on a shown status becoming `blocked`, `done` or `error` from another, not while replaying. Title: the agent's name (else the profile's); body: the permission's title, `Done`, or the failure's message. `Workspace::on_attention` lays it out with the tab title and applies the focus rule and rate limits unchanged.
- **A source of its own:** `notice::Source::Agent` (`"agent"` in `BRINDLE_NOTIFY_LOG`) and `notifications.agent` (default `true`).

### 11. The `brindle-ui` components (`ports R2`; `ui-components`)

All take tokens and plain values, bind no keys themselves (`default_bindings()` gains theirs), know nothing of agents or ACP, and list every difference from shadcn in `ui/README.md`. Names follow shadcn/ui's AI Elements (Message Scroller, Message/Bubble, Prompt Input, Reasoning, Tool, Confirmation) and shadcn's Textarea and Alert; anatomy and behaviour come from there, no code does.

- **`TextArea` and `Textarea`.** `EditState` gains `Lines { One, Many }` (`EditState::multiline`): in `Many`, normalizing keeps `\n` (from `\r\n` and `\r`) and tab and drops other controls; every other edit is shared. Vertical motion is pure over a `WrapMap` the element hands back: `motion_vertical(state, wrap, dir, goal_x)`. The entity binds the line keys and shift-enter in its `TextArea` context, wraps with `shape_text`, grows to `max_lines` then scrolls to the cursor, places the IME window at the cursor (`bounds_for_range`), reports `Changed` and exposes `composing()`. It binds no enter, escape, tab or shift-tab and propagates edge up/down. `Textarea` is `Input`'s frame around it.
- **`MessageScroller`** over GPUI's `list` with `ListAlignment::Bottom`; a pure `Follow { AtEnd, Away }` from its scroll events; a ghost `IconSm` scroll button (lucide `arrow-down`) while `Away`.
- **`Message` and `Bubble`** (End: muted; Start: card), drawable in parts so a message split into several list items reads as one bubble, with hover actions (a copy button).
- **`PromptInput`:** a queue row (optional: `Queued::Waiting { text }` or `Queued::Held { text, reason, resend_enabled }`), a `Textarea`, and a toolbar whose end holds the submit button (`Ready`: lucide `arrow-up`, disabled while blank; `Submitted`: a spinner; `Streaming`: lucide `square`, stop; `Disabled`), and a reason line. It emits `Submit`, `Stop`, `RemoveQueued` and `ResendQueued`, and the row never touches the textarea; enter is `PromptSubmit` in its context (emits only when ready, not blank, not composing), escape `PromptStop` while streaming.
- **`Reasoning`**, a pointer-only trigger (lucide `brain`, label, chevron) over muted content; keyboard path through the app's `toggle_agent_thoughts`.
- **`Tool`**, one row, no focus. **`Confirmation`**, the permission row (requested with one small button per action, each a focus stop; answered with the outcome). **`Alert`** (`default`, `destructive`), no focus.
- **The close confirmation uses the existing `Dialog`**, not `Confirmation` (decision 12): it is modal and window-level, which is `Dialog`'s job; `Confirmation` is an inline row.
- **Tokens:** only the existing set.

### 12. Closing with confirmation (`agent-zones`, `zones`, `tabs`; `fcis R1`)

**When it asks.** A pure `close_plan(zones: &[(ZoneId, AgentBusy)]) -> ClosePlan { Now, Ask(Vec<BusyAgent>) }`, where `AgentBusy` is taken from each agent zone's shown activity (`working` or `blocked`; terminals and other states are not busy), decides for every close path: Close Zone (ctrl-shift-x, the header ×, the header menu), Close Tab (ctrl-shift-w, the tab's ×, a middle click, the tab menu). It is the activity the zone shows, which `agents-sessions`' boundary names as all Brindle needs. Quit, Close Window and the window manager's close are quits, which never ask (`tabs`' "Window closes with its last tab"). Closing the last tab after confirming closes the window, which quits. `brindle-remote kill` and `stop` never ask (`agents-sessions`).

**What it shows.** `Overlay::CloseConfirm { target: ZoneId | TabId, busy: Vec<BusyAgent> }` in `Workspace`, rendered with `brindle-ui`'s `Dialog`: title "Close a working agent?" (or "Close N working agents?"), description one line per busy agent ("Fake Agent is working" / "is waiting for permission"), and "Closing ends it; the conversation can't be got back." Footer: Cancel (outline) and Close (destructive), Cancel focused when it opens, as shadcn's AlertDialog does, so an enter pressed by reflex doesn't end work. Tab and shift-tab move between the two (the dialog's focus trap); enter and space activate the focused button; escape and a backdrop press cancel. Focus returns to the zone's prompt input (or to the tab strip's focus, as the tab's close came from) on cancel, to the zone that inherits focus on close. A close of a background tab doesn't activate it.

**What changes while it is open.** Nothing is re-asked: if the agent goes idle while the dialog is open, Close still closes. If the target disappears (the zone's tab closed by another path, a session lost), the dialog closes itself.

**Dump and e2e.** While open, the dump prints `--- confirm close target=zone:ID|tab:N busy=ID:working|ID:blocked,…` after the zones lines, and the focus line `--- focus confirm stop=cancel|close modality=…`. Cases drive it with `--action close_zone` / `--action close_tab` and `--key enter`, `--key tab --key enter`, `--key escape`; `agent-close-confirm` covers all three, an idle agent closing at once, and Close Tab naming only the busy zone; `agent-quit-restore` covers quit not asking.

### 13. Lifecycle and failures (`process R2`, `R16`; `railway R6`)

| Moment | What happens |
| --- | --- |
| open (profile, shortcut, picker, menu, `brindle agent`) | `Workspace::launch_on` asks `profile.kind()`: an agent profile makes an `Agent` (`Agent::spawn(profile, host, cwd, first_prompt, config)`) in a tab of kind `Agent`. The request is built by `agent_spawn_request` on the session's thread |
| split | decision 4's `split_plan`; spawns in the split zone's directory on the tab's host |
| close (zone or tab) | `close_plan` (decision 12), then `Session::close` → `CLOSE`; the core hangs the agent up, answers its pending permissions `cancelled`, and the slot goes with it |
| quit | `ZoneContent::quit_with(layout)`: `DETACH` with the tab's layout; `QUIT_FLUSH` as for terminals. Every agent session is `Closing::Keeps`: there is no agent `-e` |
| crash | the session's `detach` policy keeps it running |
| restore | `restored_zone` makes an `Agent::attach` for a recorded agent zone whose session is listed as an agent session, an `Agent::spawn` in the recorded directory when the listing failed (no first prompt), and drops a zone whose listed session is of the other kind |
| sweep, `--attach`, sessions palette | the tab's kind from `SessionInfo.agent`, the profile from decision 4's rule |

What the zone shows for each failure (`railway R6`, `R8`: the action, then the cause; never cached, so reopening the profile retries):

| Failure | Phase | Shown |
| --- | --- | --- |
| no helper, no connection | `NotStarted(NoServer)` | "Brindle could not start the agent <command>: <cause>" (destructive alert) |
| command not found, spawn refused | `NotStarted(Refused)` | the same, with the server's error verbatim (the program and the OS error; on Windows, a batch file's hint with its `cmd /c <command>` form, shown in the monospace font on a line of its own when the text holds it) |
| server too old (protocol 3) | `NotStarted(TooOld)` | "the terminal server on <endpoint> is too old for agent sessions; it will update once its terminals have ended" |
| endpoint disconnected | `Link::Reconnecting` | the conversation stays, a "reconnecting to <endpoint>…" banner, prompts held back (draft kept) |
| session can't be got back | `Lost` | "the agent session on <endpoint> was lost: <reason>"; the zone stays until closed |
| login demanded, at start or by a prompt | `Live(LoginRequired)` | "<agent> needs you to log in. Log in through the agent's own command line, then open the profile again." and the agent's message, above the prompt input, which is disabled with that reason; the conversation so far stays readable and scrollable, a prompt that got the demand shows as a failed turn, and a waiting prompt is held (decision 9). Nothing is sent again in this zone; the session stays open until the zone is closed (`agents-login` brings login into the zone) |
| ACP handling failed | `Live(Failed)`, `SessionLife::Open` | "Brindle's terminal server failed handling <agent>: <message>"; the agent is left running until the zone is closed |
| agent doesn't speak ACP version 1 | `Live(Failed)`, `SessionLife::Ended` | "<agent> can't be used: <message>" (the server's text naming the versions, such as "the agent supports ACP versions 2; Brindle speaks 1") as a destructive alert, the agent's standard error as a notice row in the conversation; no exit status, since the server sends no `Exited` after this `Failed`. The zone stays until closed and isn't recorded |
| agent exited | `Live(Exited)`, `SessionLife::Ended` | "<agent> exited with status <code>" and the end of its standard error in a scrollable block; the zone stays until closed and isn't recorded (`agents-session-recovery` brings it back) |

The alert is chosen by an exhaustive `match (phase, life)` (`failure_text`, pure, in `model.rs`), with no `_ =>`: the two `Failed` rows differ only by `SessionLife`. Every other `match` on `AgentState` or `HoldReason` in this change is exhaustive too: `status_of` and `listed_status` (decision 8), `queue_row`'s reason and `Resend`, `prompt_status`'s `DisabledWhy`, `describe`'s `state=` and `reason=`, and the first prompt's draft rule (decision 3).
| Brindle can't follow it | `Broken` | "Brindle can't show this agent session any more; attach it again from Attach Session…"; detached, running |

A first prompt not yet sent becomes the draft in every one of these (decision 3).

### 14. Settings dialog (`settings-dialog`)

`ProfileEntry.profile` stays `Result<Profile, String>`; an agent profile reads as `Ok` with `kind() == Agent`. `ProfileRow` gains `agent: bool` (a muted `agent` badge) and the agent description. Edit and Duplicate are disabled for an agent entry, with "Agent profiles are edited in the config file" under the list; New makes terminal profiles only. Delete and the move buttons work. `ConfigEdit::apply`'s profile save also refuses a table whose `type` is `agent`. A unit test runs every `ConfigEdit` variant over a fixture holding a commented agent table and checks it byte for byte (moved whole by a reorder). The dialog's dump line gains `agent` on such rows.

### 15. Debug hooks and the dump (`testing R2`; `command-line`)

When the active zone is an agent zone, `dump_active_screen` prints the tab lines, fonts, progress and badges, the zone lines, then `ChatModel::describe()` in place of the terminal's block:

```
--- agent zone=1 state=ready name="Fake Agent" turn=working activity=working status=working link=up session=open
--- entry 1 user "hi"
--- entry 2 agent "Hello!"
--- entry 3 thought open=false "Considering"
--- entry 4 tool call="call_1" kind=read status=completed "Read main.rs"
--- entry 5 permission call="call_2" options=allow-once:"Allow once"|reject-once:"Reject" outcome=pending "Run cargo test?"
--- entry 6 notice level=warn "1 line was not an ACP message"
--- queued waiting "next"
--- conversation entries=6 at=end
--- prompt status=streaming
```

`state=` is `connecting`, `not-started`, `starting`, `ready`, `login-required`, `failed`, `exited:<code>`, `lost` or `broken`, and `failed`/`login-required` add `message="…"`; `session=` is `open` or `ended`; `turn=` is `idle`, `working`, `blocked`, `cancelling`, `ended:<stop>` or `failed`; `outcome=` is `pending`, `selected:<option>` or `cancelled`; text is escaped onto one line (`{:?}`). `--- queued` is `waiting` or `held reason=… resend=yes|no` (decision 9). `--- note "…"` comes before the entries only for a restore's note (decision 3); no launch writes one. `--- prompt` adds `refusal="…"` when one is shown. `--wait-for` matches `ChatModel::wait_text()`, those lines. `--send` types into the prompt input through `FocusedInput::{Line(TextInput), Area(TextArea)}`; the draft shows in the existing `--- input` line. `--key` drives keys; the focus line reports `agent stop=prompt|queued:remove|queued:send-again|queued:discard|permission:<entry>:<option>|stop`, or `confirm stop=cancel|close` (decision 12). The `--action` names are decision 11's and the view's.

### 16. The fake ACP agent in the harness (`testing R13`, `R17`, `R18`)

`agents-sessions` ships `brindle-fake-acp-agent` as a second `[[bin]]` of `brindle-remote-agent`, built into `target/<profile>/` by every `cargo build`, never packaged.

- **Lookup** (`tests/common/fake_agent.rs`, shared by e2e and, if ever needed, perf): `BRINDLE_FAKE_ACP_AGENT` if set; else `target/<profile>/brindle-fake-acp-agent[.exe]` beside `CARGO_BIN_EXE_brindle`'s directory, the suite's own build; never beside `BRINDLE_E2E_BIN` (an archive doesn't carry it). It has no version to match: it speaks ACP only.
- **Freshness:** like `check_helper`, a found binary older than any file under `remote-agent/src/fake_acp/` is stale. Missing or stale stops the run, when a selected case needs it, with the command to build it: `cargo build -p brindle-remote-agent --bin brindle-fake-acp-agent` (with `--release` when the suite's build is a release one). A case declares it with `Case::needs_fake_agent()` (a declaration beside `Case::needs`, not a fault switch), so `--list` and filters that select no agent case never look for it.
- **No skip by build kind.** The agent cases run on debug and release builds and against archives. The one exception is `agent-broken`, which needs the fault switch `SinkPanic` and is skipped on release builds by the existing `Case::needs` rule, as every fault-switch case is.
- **How a case uses it:** the case writes a script into its case directory and a config whose agent profile is `command = "<fake agent path>"`, `args = ["<script path>"]`; gates are files the case creates (`wait-file`), never sleeps. The scripts are written with `agents-sessions`' steps (decision 16 there); the login and version cases use:
  - *a login demanded by a prompt* (`agent-login`): `expect initialize`, `respond` with version 1, `expect session/new`, `respond`, `expect session/prompt` (the first turn), its updates and `respond {"stopReason":"end_turn"}`, `expect session/prompt` (the second), `wait-file` (so the case can queue a third prompt), `respond-error -32000 "session expired"`, then `wait-file` until the case is done, keeping the agent running;
  - *a refusal of version 1* (`agent-acp-version`): `expect initialize`, `respond-error -32022 "Unsupported protocol version" {"requested":1,"supported":[2]}`, `stderr "only ACP v2"`, `expect` nothing more; the core hangs it up. The message checked is the one the core builds from `data` ("the agent supports ACP versions 2; Brindle speaks 1"); the form without `data` ("the agent supports no ACP version Brindle speaks (1)") is covered by the reducer's fixtures in `agents-sessions`;
  - *an answer with another version* (`agent-acp-version`): `expect initialize`, `respond {"protocolVersion":2,"agentCapabilities":{}}`, `stderr "v2 only"`; the check is "the agent speaks ACP version 2; Brindle speaks 1" and that no `session/new` was recorded.
- **CI.** The `e2e` job's existing `cargo build` builds it, so nothing changes there beyond the freshness check passing. `e2e-archive` and `windows-e2e-archive` build it (`cargo build --release -p brindle-remote-agent --bin brindle-fake-acp-agent`) and set `BRINDLE_FAKE_ACP_AGENT` before running, and add the agent cases to their filters (`agent-` on Linux, `windows-agent-` on Windows). `windows-e2e` builds it with the helper. The remote case (`remote-agent-ssh`) copies the static musl build (built with the helper's README command) into its container and skips, as the other `remote-*` cases do, when it is missing unless `BRINDLE_REMOTE_E2E_REQUIRED=1`.

### 17. Tests (`testing R1`, `R13`, `R16`, `R17`, `R18`)

- **Unit tests** beside each model: `ProfileKind` and the command check, `description`, the parser's agent arm and its refusals (`-p -` twice or beside `-p TEXT`), `FirstPrompt::from_stdin` for each `PromptError` and the trailing line ending, `profile_for`'s agent arm, every `LaunchRefusal` and the default-of-kind rule, the `agent` request's encoding and decoding, `agent_spawn_request`, `split_plan`, `close_plan`, `layout::group` by kind, `reattach` choice, `agent_start`, the activity mappings, `AgentShared::apply` (including a value `from_value` rejects), every `ChatModel` rule (blocks and splices, thoughts, permissions, `submit` during a turn with the slot empty, waiting and held and with no turn beside a held prompt, `replied`, `queue_row` for every `Slot` and `HoldReason` and each phase and `SessionLife`, `failure_text` for every phase with both `SessionLife` values (a `Failed` with the session ended after a version mismatch never reading as an exit), a login demanded by a prompt (failed turn, held prompt, prompt input disabled, entries kept), `unqueue` and `resend` including after an exit, the draft never touched by a slot change, the first prompt sent once and turned into a draft on each failure, phases, `shown_status` and seen, `notice`, `title`, `describe`, `wait_text`), `EditState`'s lines mode, `motion_vertical`, `Follow`, the settings rows and the byte-for-byte write test.
- **E2E cases** (`tests/e2e/cases/agent.rs`): `agent-turn`, `agent-first-prompt` (`brindle agent … -p`, sent once; not again after a relaunch), `agent-first-prompt-login` (left as a draft), `agent-launch-kinds` (an unknown name and the kind refusals at status 2, alone and handed off, and the default-of-kind rule), `agent-handoff` (a second `brindle agent -p` handed to the running instance, starting in the launching command's directory), `agent-prompt-stdin` (`-p -` from a file and a pipe; a terminal is simulated by no case, so that refusal is unit-tested; over 256 KiB, empty and not UTF-8 refused at status 2), `agent-multiline`, `agent-queue` (queue, remove, a third prompt kept, `end_turn` sending it, a stop and a `max_tokens` ending holding it, send again idle and during a turn, discard, a new prompt beside a held one, a held prompt surviving quit and restore, the row's buttons from the keyboard), `agent-cancel`, `agent-permission-keys`, `agent-seen`, `agent-thoughts`, `agent-scroll`, `agent-copy`, `agent-palette-commands`, `agent-split`, `agent-close-confirm`, `agent-close-idle`, `agent-quit-restore`, `agent-sweep`, `agent-sessions-palette`, `agent-exit`, `agent-no-command`, `agent-login` (a login demanded by `session/new`, then by a prompt after the agent was ready while another prompt waits: the turn failed, the waiting prompt held with `reason=login-required resend=no`, discard working, the prompt input disabled, the earlier turn still in the dump), `agent-acp-version` (an `initialize` answered with `-32022`, and one answered with protocol version 2: each zone shows `state=failed session=ended` with the version message, a notice entry with the agent's standard error, and no exit status), `agent-notify`, `agent-settings-profiles`, `agent-broken`. Windows: `windows-agent-turn`, `windows-agent-close-confirm`, `windows-agent-batch-hint` (a `.cmd` on `PATH`), `windows-agent-prompt-stdin` (a file as the child's standard input, and `cmd /c "brindle agent … -p - < file"`; with no standard input refused at status 2). Remote: `remote-agent-ssh`. The server-too-old path has no e2e case (no older core can be started from this build); its gate is unit-tested.
- **Manual checks** (`CLAUDE.md`'s list, `testing R16`), Linux and Windows: IME in the prompt input (IBus ctrl-shift-u `e9` space gives `é`; enter while composing commits and doesn't send; the candidate window at the cursor on a wrapped line); wheel scrolling a conversation, following and leaving the end, and the scroll button; pointer presses on permission buttons, the stop button, the queue row's remove, send-again and discard buttons, a thought's trigger, a message's copy button and the close dialog's buttons and backdrop; a real ACP agent end to end (a turn, a tool call, a permission, a queued prompt, cancel, close with confirmation, quit and relaunch), on Windows through `cmd /c npx …`; `brindle agent … -p -` in an interactive shell refused at once without waiting (Linux); on Windows, `Get-Content -Raw task.md | brindle agent … -p -` in PowerShell 7 and 5.1, `brindle agent … -p -` with nothing redirected in PowerShell and in cmd (refused, status 2), and the same launch handed to a running Brindle.

### 18. Docs

`README.md` (agent profiles with an example, `brindle agent` and its help, the keys and actions, closing with confirmation, what is deferred and to which change, the release note about older Brindles and agent profiles), `docs/code-map.md` (a new `agent/` entry; `zone_view.rs`, `workspace.rs`, `instance.rs`, `remote/local`, `reattach`, settings dialog, `brindle-ui` and `text_input.rs` entries), `CLAUDE.md` (the dump's agent and confirm lines and focus values, the fake agent's lookup and build command, the manual checks), `ui/README.md`, `docs/architecture/functional-core-imperative-shell.md` (pure-module table) and `process-architecture.md` (R9's queue list). Each lands with the task group that makes it true.

## What the sibling changes lack

This change doesn't edit them; each is written here to what they say.

What this change relies on, now stated there:
- **`agents-sessions`: event forms are frozen per client protocol version:** within protocol 4 an event may only gain optional fields, which `from_value` ignores; an unknown kind or enum value is a `DecodeError`, and a new one comes with a new protocol version. Brindle treats a `DecodeError` as a broken session (decision 7).
- **`agents-sessions` decision 10: the queue slot.** `Event::Queue { slot }`, `Transcript::slot()`, `Slot` and `HoldReason`, `AGENT_RESEND` (type 20) with its refusals, its transition table, a replay emitting a non-empty slot, the slot surviving detach, restarts and upgrades, and an exit ending the session with the client keeping its copy (decisions 8 and 9).
- **`agents-launch-commands` decisions 5, 6 and 6a:** `profile_for` returning `Result<usize, LaunchRefusal>`, refusals as `usage:` answers with status 2, the agent request's own fields, and `instance::fits` against the 1 MiB `MAX_REQUEST` before connecting (decision 3).

What this change also relies on from `agents-sessions` decisions 4 and 17: a login demanded by a prompt (the turn `Failed`, a waiting prompt `Held { LoginRequired }`, then `LoginRequired`, in that order, with `AGENT_UNQUEUE` still accepted), and a version mismatch's `Failed` as the last agent state, with no `Exited` after it, the standard error as a `Notice` and the session's `EXIT` (decisions 8, 9 and 13).

Nothing is still missing. The one gap found last, that the fake agent's `respond-error` carried no `data`, is closed there: its step is `respond-error <code> <message> [<data-json>]`, which `agent-acp-version` uses to script a `-32022` with `{ requested, supported }`.

Two points to keep in step, neither a gap:
- `agents-launch-commands`' `LaunchRefusal` has two variants; this change adds `NoProfileOfKind`, which that change's text leaves to "default of the kind" here. Its `usage:` answer and status 2 follow that change's rule unchanged.
- `agents-sessions` refuses `AGENT_RESEND` on a waiting prompt with "no prompt is held"; Brindle never sends it then (send again shows only on a held row), so the zone shows that text only if the slot changed in flight, as the prompt input's refusal.

## As built

**Section 1, shared contracts (2026-11-25).** The prerequisites the tasks list were checked against the code: all are there. Where what `agents-sessions` and `agents-launch-commands` built differs from this design's Context, the later sections use what was built:
- `profile_for` is `profile_for(config, kind, name, config_error) -> Result<usize, LaunchRefusal>` (as decision 3 already says, not the Context's `Result<Chosen, KindMismatch>`).
- `SessionInfo.agent` is `Option<client::AgentInfo { activity: AgentActivity, name: Option<String> }>`.
- `Event::from_value` takes `&serde_json::Value`. The transcript crate also exports `PermissionKind` (an option's kind), `TEXT_CHUNK` and `chunks`; `agent::transcript` re-exports `PermissionKind` beside the Context's list, since permissions put allow kinds first.
- The core already resolves an agent's directory (`platform::agent_dir`: the request's directory with `~` expanded when it is absolute and a directory, else the home the request's environment names, else the core's own) and refuses a blank command ("an agent session needs a command"); `agent_spawn_request` (2.5) need not duplicate the fallback.

Choices the artifacts left open:
- **`Profile.kind` is `pub(crate)`,** not private: a private field breaks `Profile { name, ..Default::default() }` in other modules (the settings dialog builds profiles that way). Only `config.rs` sets it; everything else reads `kind()`.
- **A blank command is missing too:** `command = "  "` on an agent profile is the same config error as no command (the core refuses a blank one anyway).
- **The kind mismatch's words** take an article: `"Fake" is an agent profile: open it with brindle agent Fake`. Until 2.3, a terminal launch naming an agent profile is the only mismatch, and a launch naming no profile still opens the default profile whatever its kind.
- **`src/agent/transcript.rs` joined `PURE_MODULES` and `fcis`' pure-module table in 1.4,** with the module (the rule for a new whole pure module); 2.11 adds the other four. The check that no other file names `brindle_transcript` is `only_the_agent_transcript_module_names_the_transcript_crate`, cited under `ports R3`, whose text now says so.
- **`remote::agent_link<Start, Sink>(host: Host, start, sink) -> Result<AgentLink, Unsupported>`** leaves `start` and `sink` generic until group 4 gives them their types (`Start::SpawnAgent`'s request and `agent::shared::Sink`). `AgentLink` is an uninhabited enum until group 4 gives it the session on Linux and Windows. `Unsupported` is a struct holding its reason (`Display`, `Error`): elsewhere "agent sessions run in the terminal server, which needs Linux or Windows"; the Linux and Windows stub answers "agent sessions aren't connected to the terminal server in this build", which group 4 removes.
- **The fake agent's freshness check also applies to one `BRINDLE_FAKE_ACP_AGENT` names** (unlike `check_helper`'s `Chosen`, taken as it is): decision 16 calls any found binary older than its sources stale, and the archive jobs build it from the same checkout. The pure parts are `fake_agent::{locate, build_command, verdict}`; `find` reads the variable and `check` the files. `Ctx::fake_agent` carries the path to every case, and the runner checks it only when `wants_fake_agent` (a selected case that opens windows declares it, with a display to run it on).

**Section 2, pure models (2026-11-25).** Choices the artifacts left open:
- **Until 6.3 an agent launch stops after its profile check:** `brindle agent` reads `-p -`, checks the profile (usage errors, status 2), then prints "agent tabs aren't connected to the window in this build of Brindle" and exits 1; a forwarded agent request is profile-checked (`usage:`) and then answered `error:` with the same words; `Args::opening()` gives no opening for it. 6.3 replaces all three.
- **`PROMPT_LIMIT` (256 KiB) is defined in `src/main.rs`,** since Brindle can't name the helper's `MAX_PROMPT`. `FirstPrompt::from_stdin` takes `Option<&[u8]>` (`None` is `Absent`); a standard input that is a terminal is the shell's refusal ("-p - reads the prompt from standard input, which is a terminal; redirect a file or pipe into it"), not a `PromptError`. `TooLong` counts the bytes before the line ending is dropped, matching the read of at most 256 KiB + 1 bytes. `--prompt` outside `agent` gets its own message; `-p` there keeps `agents-launch-commands`' removed-option messages.
- **`NoProfileOfKind`'s words** name how to add one: "the config has no agent profile (add one with type = \"agent\"; the profiles are: …)", and the broken-config wording as `UnknownProfile`'s. `instance::Answer::line` became `pub` for the tests.
- **The agent's environment:** Brindle sets no shell-integration variables, so only `TERM`, `COLORTERM`, `TERM_PROGRAM` and `TERM_PROGRAM_VERSION` are dropped from the inherited environment; one the profile's `env` sets itself is kept. A remote agent gets the profile's `env` only, and with neither `-d` nor a profile `cwd` no directory (the launch directory and the local home mean nothing there; its server picks the home). `agent_spawn_request` returns an `AgentSpawnRequest` the `SpawnAgent` message is built from in group 4.
- **`split_plan` refuses for terminal tabs too** when the profile is gone or changed kind, as the zones spec says. `close_plan` takes `Closing::{Terminal(zone), Agent { zone, name, busy }}` rather than `(ZoneId, AgentBusy)` pairs, so its `close_text` can name the agents ("Close N working agents?", "<name> is working" / "is waiting for permission").
- **`local_profile` for an agent session** also falls back to the first local agent profile after the recorded and default ones (a renamed profile); `layout::group` drops the zones of the other kind from a mixed layout into flat tabs of their own.
- **The protocol gate** is `agent_start(protocol, endpoint) -> Result<(), StartFailure>` with `AGENT_PROTOCOL = 4`, in `model.rs`.
- **`AgentShared::apply`** stops at the first event that doesn't decode or fold, keeping those before it; `Broken`'s text is "an event from the terminal server couldn't be read: …" or "… couldn't be applied: …". `Changes` gains `from_replay` (set for everything folded between `REPLAY_START` and `REPLAY_END`), which the model needs for notices and the first prompt.
- **`ChatModel`'s shapes:** `prompt_status()` takes no arguments (blank and composing are `submit`'s); `unqueue()` returns `Unqueue::{Send, Cleared, Nothing}` and `resend()` a `bool`, the view sending the request; the zone records each request with `sent(req, Sent::{Prompt, FirstPrompt(text), Resend, Unqueue, Answer(entry)})` and `replied` answers `Replied::{ClearDraft, SetDraft(text), Nothing}`. `Absorbed` gains `first: Option<First::{Send, Draft}>`, the one way the first prompt reaches the view; life events (`not_started`, `ended`, `lost`, `broken`, `set_link`) return an `Absorbed` too. A first prompt is sent on the first live `Ready` with the link up (held back while reconnecting); when a replay is what shows the agent ready (a reattach), it becomes the draft, never sent.
- **The title keeps the agent's name** from the last `Ready` that gave one after the agent exits or fails, so a tab's title doesn't fall back to the profile's on exit.
- **Blocks:** a message part never starts with a line break (breaks between parts belong to none), and a streaming message redraws only its last part from that part's start. `failure_text` takes `Names { agent, command, endpoint }` and returns an `Alert { text, destructive, detail, mono }`; a `Live(Starting | Ready)` whose session ended without an `Exited` or `Failed` (killed from `brindle-remote kill`) reads "the agent session on <endpoint> has ended". A Windows batch file's `cmd /c …` form is taken from the server's text between backticks into `mono`.
- **Windows:** `win::stdin_absent` (null or invalid standard input handle) can't be built or clippy-checked on this Linux machine; the Windows CI job checks it.

**Section 3, brindle-ui components (2026-11-25).** Choices the artifacts left open:
- **`TestAppContext` tests need gpui's `test-support`,** added to `brindle-ui` as a dev-dependency (gpui's own feature, so the closed dependency list is unchanged). `Cargo.lock` gains only the entries that feature pulls in (additive; nothing else moved). `ui/src/test_support.rs` builds the app with the crate's bindings and the light tokens.
- **Enter in a text area is `TextAreaEnter`,** which propagates unless composing, so the owner's enter binding runs next; the input handler's fallback text for an unbound enter or tab (`\n`, `\t`) is dropped (`is_unbound_key_text`). Up and down at the edges propagate; with shift they extend to the text's start or end. End on a wrapping row stops before the wrap.
- **`MessageScroller` is an entity** (`new(count, render_item, cx)`, `splice`, `reset`, `follow`, `scroll_to_end`, `scroll_page(PageDirection)`, `scroll_top`); `Follow::after_scroll(is_scrolled)` from the list's scroll handler, and `Follow::after_draw` returns to `AtEnd` when a page down shows the last item's end. **The scroll button is outline, not ghost** (decision 11 said ghost): it floats over the messages, and AI Elements' is outline. It takes no focus.
- **`PromptInput`'s API:** `PromptStatus::{Ready, Submitted, Streaming, Disabled}` (the app maps its `Disabled(DisabledWhy)` onto `Disabled` plus `set_reason`), `Queued::{Waiting { text }, Held { text, reason, resend: Resend::{Available, Unavailable(why)} }}` (decision 11's `resend_enabled` with its tooltip), `set_text`, `set_status`, `set_queued`, `set_reason`, `set_held_color` (the tokens have no warning colour; the default is destructive), `area()`, `row_stops()` and `stop_button()` for the zone's focus order, and `Focusable` giving the text area's handle. **Enter submits while streaming too** (the app's `submit` decides to queue or refuse); `Submitted` and `Disabled` keep the enter and ask nothing. Escape propagates unless streaming. `Disabled` makes the text area read-only. The submit button is a focus stop only while it is the stop button. The toolbar sits under the frame, the reason line at its start.
- **`Message::action`** elements are drawn only under the pointer, so take no focus; `BubblePart` borders and pads only the outer edges. **`Confirmation`** takes `ConfirmationAction`s keyed by `ItemKey` with owner focus handles and reports `on_action(&ItemKey)`; `ConfirmationState::{Requested, Accepted { label }, Rejected { label }, Cancelled}`. **`Tool`** takes `ToolStatus::{Pending, Running, Done, Failed}`. **`Alert`**'s description is its children (for the monospace command line of decision 13), and its title wraps rather than clamping to one line.
- **Icons:** the lucide paths were flattened into polylines once (arcs about every 15 degrees, curves in eight chords), the path data quoted beside each; `IconElement::turn` and `Button::icon_turn` draw the spinner's frames under `with_animation`.
- **Gallery:** an "AI elements" section, and `--check` gains six key-driven checks (text area line keys, blank and submitting enter, stop while streaming, the queue row's stops, the confirmation, the scroller while away). Under `scripts/with-xvfb` the run is 56 ok, 51 skipped (pointer, key-release and pixel checks, Windows only), 0 failed. `dropdown-enter-space-down-open-below-the-trigger-other-keys-do-not` failed before this section (the sections above put the trigger low enough that the menu flipped above it); it now scrolls the trigger near the top first.

**Section 4, agent sessions in the client (2026-11-25).** Choices the artifacts left open:
- **The sink is `agent::sink::Sink`, not `agent::shared::Sink`** (task 4.4's name): `shared.rs` is a whole pure module (`PURE_MODULES`), and the sink holds a channel, a lock and the fault switch's environment read, so it is a shell module of its own beside it. `remote/local/agent.rs` implements `SessionSink` for it, so `src/agent/` never names `remote::local`. Its zone's end is `Feed { shared, events, took() }`; the events are `AgentEvent::{Changed, Replay(bool), LinkDown, Exited, Lost, Failed(StartFailure), Broken, Reply { request, outcome }, Note}`, `Changed` merged as a wakeup and the rest life events. A replay's start also sends `Changed`, so the zone resets its blocks at once.
- **The offer passed to the dial:** `Connected` carries the offer its `HELLO` made, and the `Connection` keeps it for reconnects (`hello_offering`) and relay greetings (`hello_message(token, relay, offer)`); `connect_offering` and `relay_with(…, offer, …)` take it, and `connect`, `hello` and `shared_with` pass `BRINDLE_PROTOCOLS`, still 1–3. Only the tests offer `client::PROTOCOLS`.
- **The gate runs in the writer,** just before `SPAWN_AGENT` is written, on the link's negotiated protocol (`State.link.protocol()`), so a spawn queued while disconnected is checked against the core it would reach. A refused one is never written and fails its `Waiting::Spawn` with the typed `StartFailure::TooOld`. `SPAWN_AGENT` reuses `Waiting::Spawn` (the core answers `SPAWNED`), always with the `detach` policy and the profile's name; a relay's carries the endpoint's environment as `SPAWN` does.
- **Start failures are typed on the session's thread:** a connect error becomes `NoServer`, the gate's refusal stays `TooOld`, any other spawn error becomes `Refused { command, cause }`; `SessionSink::failed`'s `anyhow::Error` carries it (`StartFailure` gained `Display` and `Error`), and the sink downcasts it. The connection's request replies became `anyhow::Result`, so typed errors survive them.
- **Every agent request gets an answer:** `Waiting::Agent { req, session, sink }` holds it in `State.requests` like any request, so `OK`, `ERROR` and a link lost with it written (or a connection ended) all reach `SessionSink::reply` under its id, which `ChatModel::replied` needs to leave `Submitted`. A request is sent only while the connection shows the session by id and key, and not held while it starts: `None` (or `false` for `cancel`) means nothing went.
- **Breaking:** an `Err` from `SessionSink::transcript` and a panic share `Connection::break_session` (forget, `DETACH`, `broken`), factored out of `deliver`. A dropped link tells each shown sink `link_down` after failing the written requests.
- **`agent_link` returns at once** and never fails on Linux or Windows: connecting, the gate and the spawn or attach run on the session's thread, and their failure reaches the sink. `AgentStart::{Spawn(request), Attach { session, key, recorded }}`; an attach asks at 0×0 (`Attaching::Now`) or waits for the listing (`Recorded`). The seam's items carry `allow(dead_code)` until group 5's `Agent` entity calls them.
- **Tests:** the client's scenarios run against scripted protocol 4 cores (`MiniAgent` stands in for the core's slot and turn; the core's own decisions are `remote-agent`'s `agents.rs` tests), and one ignored test runs a real core with `brindle-fake-acp-agent`: a turn, then the fault marker in the agent's answer breaking the session, which stays listed and detached. `TestSwitch::SinkPanic`'s marker is given to the sink directly (`Sink::with_fault`), since tests set no process environment; its e2e case needs group 5's zone.
- **One commit for 4.1–4.3:** their client changes share the request table, the reply plumbing and the sink trait, so they were committed together (after the sink's own commit), with the stub's replacement from 4.4.

**Section 5, the agent entity and view (2026-11-25).** Choices the artifacts left open:
- **A `Requests` port between the entity and its session:** `Agent` holds `Option<Box<dyn agent::Requests>>` (prompt, resend, unqueue, answer, cancel, close, detach_with, set_layout, id_and_key), which `remote::AgentLink` implements; the tests give a fake that records the calls. `Agent::spawn` and `Agent::attach` call `agent_link` and build on `Agent::from_parts(model, feed, requests, command, host, directory)`, which the tests call with a real `Sink::with_fault(None)`. Where `agent_link` is `Unsupported`, the zone is `NotStarted(NoServer)` with its reason. `Feed::split` hands the entity's loop the receiver apart from the gate's end (`Taker`).
- **What the model gained:** `ChatModel::unsent(Sent)` answers a request that couldn't be sent (no link, or the link fell between the decision and the send) as a refusal with `UNSENT` ("the agent session isn't connected; nothing was sent"), so a first prompt becomes the draft; `server_note` shows `AgentEvent::Note` (a refused cancel) as the prompt's refusal line; `keeps_session` (open, or still connecting) is what `server_session` and `set_layout` follow. A `Replay(false)` while `Reconnecting` is the link up again.
- **The entity emits the terminals' events:** `TerminalEvent::Status` carries the model's notice as a `Notice` with `Source::Agent` (group 7 lays it out with the tab's title and applies the policy), and `TitleChanged`; the view hears `AgentUpdate::{Splices, ClearDraft, SetDraft}` and observes the entity for the rest.
- **The directory** is worked out on the UI thread by `spawn::directory` (now `pub`; it reads only the home directory, as a terminal's spawn does), and the request with the environment on the session's thread. The Windows case-insensitive merge reads `terminal::process::LOCAL_ENV_IGNORES_CASE`, a new constant in that seam, since `cfg!` outside a seam is a platform cfg to `ports R4`'s check.
- **Focus:** the view's `Focusable` is the prompt's text area; `FocusStops` are the prompt, the queue row's buttons, the pending permissions' buttons (one handle per `(EntryId, option)`, kept across refreshes), then stop. On every change of the entity the view compares its stops before and after and moves focus to the prompt when the focused one went. Enter on a permission button answers it; escape there reaches the `AgentZone` context's `agent_cancel`.
- **Keys and actions:** the `AgentZone` context binds escape to `agent_cancel` and shift-pageup/pagedown to the existing `ScrollPageUp`/`ScrollPageDown` (the view maps them onto `scroll_page`; it also answers `scroll_to_bottom`). The eight agent actions are in the `ACTIONS` table with palette labels ("Send Prompt", "Stop Agent Turn", "Remove Queued Prompt", "Send Held Prompt Again", "Copy Queued Prompt", "Copy Last Agent Message", "Toggle Agent Thoughts", "Scroll Conversation to Bottom"); only the view handles them, so the palette lists them only in an agent zone.
- **Theme and size (5.3):** the card takes the profile theme's background and foreground and its yellow (`ansi[3]`) as the held row's warning colour; the `brindle-ui` components keep the chrome's tokens (GPUI has no scoped global). The zone lays out, prepaints and paints inside a rem scope (`Window::with_rem_size`) of its text size, the profile's font size else the terminal size in effect, so the rem-sized components follow it; `set_profile` measures every block again.
- **Dead code until group 6:** nothing opens an agent zone before `ZoneContent::Agent`, so `Agent`, `AgentView` and the agent modules keep their dead-code allows, and the `remote::agent_link` seam items keep theirs (section 4 expected them to go here); group 6 removes them.
- **Tests:** `gpui`'s `test-support` became a dev-dependency of the app crate too (no `Cargo.lock` change). `src/agent/test_support.rs` sets the app up as Brindle does; `settle` advances the test clock past the loop's pacing. "Render takes no lock" holds the fold's lock on another thread while a frame is drawn, the holder giving up after 10 s so a failing run fails rather than hangs. **5.1 and 5.2 are one commit**, since the entity's render check needs the view.

## Risks / Trade-offs

- [GPUI 0.2.2's `list` has no public "is at end" query] → `Follow` from its scroll events and the visible range; a unit-tested rule, and `agent-scroll` checks it through the dump.
- [A long agent message re-laid out on every chunk] → blocks of at most a paragraph or 4 KiB.
- [Two copies of a long log per zone] → bounded by the 16 MiB cap; `agents-log-paging` shrinks both.
- [Enter during IME composition sending a half-composed prompt] → `PromptSubmit` refuses while composing; a manual check.
- [An older Brindle on the same config loses the whole config to an agent profile] → `process R15`'s rule; the release note.
- [Raising the offer before the sweep knows agent sessions] → task 6.5 after 6.4.
- [A confirmation dialog nobody expects interrupts a scripted close] → it appears only for a busy agent; scripts that close tabs through Brindle are its own e2e cases, which drive it; `brindle-remote kill` never asks.
- [A first prompt lost on a quit before the agent was ready] → rare (a start takes seconds); the user chose to drop it (decision 1).
- [A held prompt the user forgets] → it stays in its row, survives quits, and costs nothing until discarded or the zone closed.
- [`-p -` on Windows depends on the parent passing handles] → the e2e case covers `Command::stdin` and cmd's `<`; PowerShell's pipeline is a manual check.
- [Permission requests waiting forever when nobody attaches] → by design; listed `blocked`, dot and notification once live.

## Migration Plan

Additive apart from what `agents-launch-commands` already broke. No state-file change, no client-protocol message of Brindle's own, one additive field in the instance request, one new config key per profile (`type`) and one under `[notifications]` (`agent`). Order: `agents-launch-commands` and `agents-sessions` are implemented and archived first (either order); this change's Brindle offers 1–4 and upgrades older cores as today; one that can't be upgraded shows the "too old" message only in agent zones. Rollback: an older Brindle ignores agent sessions (they stay detached and listed by `brindle-remote list`) and rejects a config with an agent profile as decision 2 says.

## Open Questions

None. Every question from the brief is decided (rounds 1 to 4) and written in decision 1 with its rejected alternatives. Smaller choices this design makes on its own (the hold reasons' wording, the `copy_queued_prompt` action, the trailing line ending dropped from `-p -`) are stated where they are made and can change without reshaping the plan.
