# Design

## Context

See proposal.md for why. What the code does today:

- **Parsing.** `src/main.rs`'s `parse_args_from` walks the arguments once, in a `match` on each, into a flat `Args { command, profile, cwd, steps, …, attach }`. `-e` swallows every argument after it. It is pure in shape (it returns `Result<Args, String>`, and `usage_error` prints the error with `HELP` and exits 2), but two arms are not: the informational options print and `process::exit(0)` from inside it, and `--config` calls `std::env::set_var`. `Args::asks_for_a_tab` is "any of profile, cwd, command"; `--attach` silently wins over them (`startup_launch`, and `forwarded_launch` for a hand-off).
- **Profile choice.** `workspace::profile_for(config, name) -> Chosen { index, missing }` is pure and is the one rule for the first launch and a forwarded one: an unknown name falls back to the default profile, and `Chosen::note` is the tab's dim line.
- **Hand-off.** `src/instance.rs`'s `Request` is one JSON line, `#[serde(default)]`, every field optional and omitted when unset, unknown fields ignored: `profile`, `cwd` (made absolute by the launch), `command` (argv), `attach`, `launch_dir`, `activation_token`. A plain launch is `{}`. The instance answers `ok` or `error: …`; the launch exits 0, or prints the error and exits 1 (`session_lock`). `forwarded_launch` returns `None` (raise only) when `profile`, `cwd`, `command` and `attach` are all unset. There is no version and no negotiation: the line goes one way and the answer back.
- **Nothing records a launch's options.** `src/session.rs` records each tab's profile name, directory and zones, never a command (an `-e` tab comes back as its profile's program, `session-restore`). `src/desktop.rs` writes `Exec=<path>` with no arguments, and the Windows Start Menu shortcut sets only the path; WinGet's `brindle` link points at the exe. None of them change.
- **Order in `main`.** `parse_args` → desktop install (exits) → `launch_state_dir` → `session_lock` (which forwards and exits) → `Config::load_or_default()` → GPUI. The config is read after the hand-off.

Callers of the flat options, found by grep (excluding `.claude/worktrees/`, `openspec/changes/archive/`, and `-p`/`-d`/`-e` that belong to `ssh`, `docker`, `sshd`, `test`, `htop` or `cargo`):

| File | Uses | What changes |
| --- | --- | --- |
| `src/main.rs` | parser arms, `HELP`, 9 unit-test lines (`-p`, `--profile`, `-d`, `--working-directory`, `-e`, `--command`) | the parser, help and tests (decisions 1–4) |
| `src/instance.rs` | field docs (`-p`, `-d`, `-e`), the round-trip test | `kind`, `asked()`, transcript tests (decision 6) |
| `src/workspace.rs`, `src/workspace/launch.rs`, `src/terminal/mod.rs`, `src/remote/local/session.rs` | doc comments naming `-p`/`-d`/`-e` | wording only |
| `tests/e2e/brindle.rs` | `Run`: `-p PROFILE --no-restore` first, `-e` + command last (2) | `terminal PROFILE --no-restore`; `terminal` before `-e` when no profile was passed |
| `tests/e2e/cases/cli.rs` | `usage_errors`'s `-p` case (1) | removed-option, `agent` and bare-word cases |
| `tests/e2e/cases/launch.rs` | `launch-unknown-profile` runs `.profile("wrk")` and checks the fallback's dim line | rewritten as the refusal (decision 5) |
| `tests/e2e/cases/instance.rs` | `second_launch`'s forwarded `-e` (1) | `terminal -e`; a `brindle terminal` hand-off and a refused flat option |
| `tests/e2e/cases/win.rs` | `-p cmd`/`-p pwsh` in 18 runs, and `run_with`'s `args.contains(&"-p")` that adds `--no-restore` (19 in all) | `terminal cmd`/`terminal pwsh`; the check looks for `terminal` |
| `tests/perf/cases.rs` | `latency_shell` (`-p Shell`), `throughput_brindle` (`-p Shell -e sh -c …`) (3) | `terminal Shell …` |
| `scripts/run-throwaway.sh`, `scripts/run-throwaway.ps1` | usage examples `-- -d /tmp`, `-BrindleArgs '-d','C:/Temp'` | `-- terminal -d /tmp`, `'terminal','-d','C:/Temp'` |
| `README.md` | Usage block, the hand-off paragraph, Session restore, Remote tabs, Closing (`brindle -e`, `-p`, `-d`, `-e`) | the subcommand, a migration note |
| `CLAUDE.md` | the throwaway example, "`Run::command` passes `-e`", the bad-steps list's "`-e` without a command" | the subcommand |
| `docs/code-map.md` | notice line (`-p`), reattach (`-p`/`-d`/`-e`), `instance.rs` (`-p`, `-d`, `-e`) | the subcommand, the `kind` field |
| `docs/manual-checks/clipboard.md` | `-BrindleArgs '-d','C:/Temp'` | as the script |
| `docs/architecture/README.md` | none today | the `process R4` exception (decision 6) |

No workflow under `.github/`, no installer (`scripts/get.sh`, `get.ps1`, `install.*`, `uninstall.*`), no packaging file (`packaging/`, `scripts/package-*`, `winget-manifest.ps1`) and no `.claude/rules` or skill passes the flat options; they call `--version`, `--export-ui-tokens`, `--install-desktop`/`--uninstall-desktop` and `--list-*`, which keep working. `tests/common/` builds no launch arguments.

## Goals / Non-Goals

**Goals:**
- A parsed launch that can't hold a terminal launch with a prompt, an agent launch with a command, or `--attach` with a subcommand's tab.
- `agents-profiles` adds `brindle agent` by adding a variant and a parser arm, with every `match` on the launch failing to compile until it handles it, and nothing else reshaped.
- Every refusal is a usage error before any window opens, naming the option and the form to use.
- A hand-off between this version and an older or newer running Brindle opens a terminal tab when one is asked for, and never opens the wrong kind.

**Non-Goals:**
- Profile kinds in the config (`agents-profiles`), agent tabs, `-p PROMPT`.
- A general argument-parsing library: the parser stays hand-written (no new dependency, `CLAUDE.md`'s `Cargo.lock` rule).
- Negotiating the hand-off's version (decision 6 says why not, and records the exception).
- Subcommands for the informational, desktop or debug options (`brindle list-actions`): they stay options.

## Decisions

### 1. The grammar

```
brindle [GLOBAL]... [SUBCOMMAND [SUBARG]...] [GLOBAL]...
SUBCOMMAND := terminal [PROFILE] [-d DIR] [-e CMD [ARG]...]     (PROFILE and -d in either order; -e last)
            | agent …                                              (reserved: refused until agents-profiles)
GLOBAL     := --config FILE | --no-restore | --attach ID (no subcommand)
            | the debug options | the informational options | the desktop install options
```

- **One left-to-right pass, two option sets.** Global options are recognised anywhere, before or after the subcommand, so `Run` can keep putting `terminal PROFILE --no-restore` first and the debug steps after (`brindle --send x terminal Shell` works too). Subcommand options (`-d`, `--working-directory`, `-e`, `--command`) are recognised only after the subcommand word; before it they are the removed flat options (decision 3).
- **The subcommand is the first bare word.** Every global option that takes a value consumes it in its own arm, so the first argument not starting with `-` that no option consumed is the subcommand. `terminal` and `agent` are matched exactly, case-sensitively, as every option is (confirmed by the user: `brindle Terminal` is an unknown word). Any other word is refused: `unknown argument "work": to open a profile, run brindle terminal work`. So bare `brindle PROFILE` is not a shorthand: it would make a profile named `terminal` or `agent` ambiguous, and it would read every future subcommand word as a profile in an older Brindle.
- **PROFILE is the first bare word after the subcommand.** A profile named `terminal` or `agent` is `brindle terminal terminal`: the position decides, so there is no ambiguity. A second bare word before `-e` is refused (`unexpected argument "Shell": brindle terminal takes one profile`).
- **`-e` takes the rest, debug options included.** As today, every argument after `-e`, however it looks, is the command's: `brindle terminal -e grep --send x` runs `grep --send x`. So `-e` comes last, and debug options go before it. `Run` already places it last.
- **No `--`** (confirmed by the user). `-e` already ends option parsing for the only place where arbitrary words appear. A profile whose name starts with `-` can't be named on the command line (it is refused as an unknown option); adding `--` later would be additive.
- **Long forms** (confirmed by the user): `--working-directory` and `--command` stay as long forms of `-d` and `-e` under the subcommand; `--profile` is gone everywhere.
- **`--attach` excludes a subcommand** (confirmed by the user). Both ask for the launch's one tab; today `--attach` silently wins. Now `brindle terminal Work --attach 7` is a usage error naming `--attach`. The type makes the combination unrepresentable (decision 2), so refusing it is the parser's only option.
- **Informational options after a subcommand.** `--help`, `--version`, `--list-*`, `--export-ui-tokens` and `--shell-integration` print and exit wherever they appear, as today. `--help` prints the one help text (decision 7). The desktop install options with a subcommand do their work and exit; the subcommand is still parsed, so a malformed one is still refused first.

### 2. The parsed form (`types R1`, `R2`, `fcis R1`)

```rust
/// What the launch asks for beyond the window.
enum Asked {
    /// Bare `brindle`: restore, else one default-profile tab; a hand-off only raises.
    Window,
    /// `brindle terminal …`: a new terminal tab, even with nothing named.
    Terminal(TerminalLaunch),
    /// `--attach ID`.
    Attach(u32),
}
struct TerminalLaunch { profile: Option<String>, cwd: Option<PathBuf>, command: Option<Command> }
struct Command { program: String, args: Vec<String> }

/// The subcommands, for the kind check and the hand-off's `kind`.
enum TabKind { Terminal }

/// The parse's outcome: run, or print and exit (no IO in the parser).
enum Parsed { Run(Args), Print(Info) }
enum Info { Help, Version, Actions, FaultSwitches, Themes, UiTokens, ShellScript(&'static str) }
```

`Args` keeps the debug steps, step interval, dump, latency, desktop request and `no_restore`, and gains `asked: Asked` and `config: Option<PathBuf>`, replacing `profile`, `cwd`, `command` and `attach`.

- What it rules out: `--attach` with a terminal's profile (one variant), a terminal launch with a prompt (`TerminalLaunch` has no such field), an agent launch with a command (`agents-profiles` adds `Asked::Agent(AgentLaunch { profile, cwd, prompt })`, which has none), and a command without a program (`Command.program` is not optional; today's `(String, Vec<String>)` becomes a named type).
- `Asked::Window` vs `Asked::Terminal(TerminalLaunch::default())` is the one new distinction (confirmed by the user): `brindle terminal` asks for a new default-profile tab, bare `brindle` doesn't. The subcommand is an explicit request for a tab, and it gives scripts a way to open a default tab in a running Brindle. `asks_for_a_tab` becomes a `match` (no `_ =>`, `railway R3`).
- `parse_args_from` becomes wholly pure (`fcis R1`–`R3`): informational options return `Parsed::Print`, which `main` prints after `console_output()` and exits 0; `--config` is returned and `main` sets `BRINDLE_CONFIG` before anything reads it, so no unit test needs a process-wide `set_var` (`testing R7`). `--shell-integration`'s lookup stays in the parser (a pure table lookup). `home` stays an argument.

### 3. The removed flat options

Each is refused by its own arm, which returns the usage error; `usage_error` prints it with the help and exits 2 before `launch_state_dir`, so a hand-off never happens and no window opens:

| Given | Error |
| --- | --- |
| `-p NAME` / `--profile NAME` (anywhere) | `-p was removed: name the profile after the subcommand, as brindle terminal NAME` (with NAME when given) |
| `-d DIR` / `--working-directory DIR` before a subcommand | `-d belongs to a subcommand now: brindle terminal -d DIR` |
| `-e CMD…` / `--command CMD…` before a subcommand | `-e belongs to a subcommand now: brindle terminal -e CMD…` |

The message carries the user's own values (quoted for the shell when they need it), so a script's author can copy it. No deprecation period (the user's decision): accepting the old form while warning would hide a warning nobody sees in desktop launchers. `-p` under `terminal` gets the first row's message, so it isn't confused with `agent`'s future `-p PROMPT`.

On Windows `brindle.exe` is a GUI program: a usage error reaches a console only through `console_output()`. A shortcut or `Run` dialog entry with `-p work` has none, so `usage_error` also calls `show_unprinted` (a message box when there is no console), as the hand-off's refusals already do. That is new for every usage error, not just the removed options, and is the only Windows-specific part.

### 4. `brindle agent` before `agents-profiles`

The parser recognises the word `agent` as a subcommand and returns `agent tabs aren't in this version of Brindle (run brindle terminal for a terminal tab)`, status 2, whatever follows. It is never a profile name or an unknown word, so a script written for `agents-profiles` fails loudly on this build rather than opening a terminal of a profile named like its first argument. `TabKind` has no `Agent` variant yet: adding one is `agents-profiles`' first step, and every `match` on it then fails to compile until it is handled.

### 5. The profile check: unknown names and kinds (`fcis R1`, `types R1`, `railway R6`)

- `config.rs` gains `ProfileKind { Terminal }` and `Profile::kind(&self) -> ProfileKind`, which returns `Terminal` (no config key yet; `agents-profiles` adds the `type` key and the `Agent` variant and makes `kind()` read it).
- `workspace::profile_for(config, kind: TabKind, name: Option<&str>, config_error: bool) -> Result<usize, LaunchRefusal>` (pure), with `LaunchRefusal { UnknownProfile { name, known: Vec<String>, config_error: bool }, KindMismatch { profile, kind } }`, a closed enum whose `Display` is the usage error. No name gives the default profile; a name matched case-insensitively gives that profile, refused as `KindMismatch` when `match (kind, profile.kind())` (exhaustive) says the kinds differ; any other name is `UnknownProfile`, listing every profile name in config order.
- **An unknown name is a usage error (the user's decision).** There is no fallback for a launch, so `Chosen`'s `missing` field, `Chosen::note`, `LaunchRequest.note` and the code in `Workspace::launch` that writes it into the new terminal all go: nothing else sets a launch's note. Three things stay as they are: `missing_profile_text` (a restored tab whose recorded profile is gone, a separate path in the restore code), `no_such_profile_text` (a profile shortcut past the last profile) and `open_profile_keyed` (the profile menu and picker fall back to the row's listed place when its profile was renamed or removed after they were shown). Those come from the user's own state or from a list they saw, not from a name typed for this launch.
- **Which process checks.** The Brindle that will open the tab checks, against the config it opens the tab with, and nobody else:
  - A launch that becomes the instance has read its config before GPUI starts (`Config::load_or_default()` in `main`, already before `Application::new`). It checks there; a refusal goes through `usage_error` (status 2, the message box on Windows without a console), before any window opens.
  - A launch handed to a running Brindle doesn't check. Its own config can differ from the instance's: `--config` and `BRINDLE_CONFIG` aren't part of the request, and the instance reloads the file about a second after an edit. A check in the launcher would refuse a profile the instance has, or pass one it lacks. The instance checks in `forwarded_launch` against `Settings::get(cx).config`, the config the tab would open with. A profile added and reloaded is found, and one added a moment before the launch, before the reload, is refused, saying which profiles the running Brindle has.
  - The instance answers a refusal on a new answer line, `usage: TEXT`, beside `ok` and `error: TEXT`. A launch of this version prints TEXT, shows it in the message box when it has no console (the path `session_lock` already has), and exits 2. An older launch, which knows only `ok` and `error:`, prints the whole line and exits 1. That is the only cost, and it happens only during an upgrade. The answer is one more value on an unnegotiated channel, covered by the `process R4` row (decision 6) and pinned by a transcript test.
- **A config that fails to parse.** Brindle then runs on the default config and shows the error on its config line (`process R15`). The check runs against the profiles in use, the defaults, because those are what the tab would open with. The refusal says so: `no profile is named "Work" (the config has an error, so Brindle is using the default config, whose profiles are: Shell)`. `config_error` comes from `ConfigReport.error` at launch and from the instance's `Settings` (which keeps the last report) for a hand-off. A desktop launcher naming a profile then fails until the config is fixed. That is the user's decision, and the message names the cause.
- Nothing in this change can fail the kind half (one kind). Unit tests cover `UnknownProfile`, case-insensitive matching, no name, the config-error wording and `KindMismatch`'s text built directly; `agents-profiles` adds the mismatch cases.

### 6. The hand-off across versions (`process R4`, `R5`, `R6`, `R11`)

- **Wire form.** `Request` gains `kind: Option<String>`, serialized as `"kind":"terminal"` for any `brindle terminal` launch and omitted otherwise. `profile`, `cwd` and `command` keep their meaning and are still filled for a terminal launch. A bare launch stays `{}` byte for byte; the other launches gain one optional field (`process R5`: additive, omitted when unset).
- **Decoding at the boundary.** `Request::asked(&self) -> Result<Forward, String>` turns the line into a closed `Forward { Raise, Terminal(TerminalLaunch), Attach(u32) }`: `attach` set wins (older launches may send it with a profile); `kind` `"terminal"` is a terminal tab; no `kind` with a profile, directory or command is a terminal tab (an older launch); no `kind` and nothing else raises; any other `kind` is `Err("this Brindle (VERSION) can't open KIND tabs; quit it and launch again")`. `kind` is a `String` on the wire so that an unknown value reaches `asked()` with its name, rather than failing serde's decode with an opaque `unknown variant`.
- **Which versions meet:**

| Launch → instance | Request | Result |
| --- | --- | --- |
| older (flat options) → this | `{"profile":…}` etc., no `kind` | terminal tab, as before |
| this → older | `{"profile":…,"kind":"terminal"}` | `kind` ignored; terminal tab, as before |
| this, `brindle terminal` alone → older | `{"kind":"terminal"}` | older sees nothing asked: raises only (documented in README) |
| this, `brindle terminal wrk` (unknown) → older | `{"kind":"terminal","profile":"wrk"}` | older falls back to its default profile with its dim line, as it always did; launch exits 0 |
| older → this, unknown name | `{"profile":"wrk"}` | refused with `usage: …`; the older launch prints the line and exits 1 |
| `agents-profiles`' agent launch → this | `{"kind":"agent",…}` | refused by name; launch exits 1 |
| `agents-profiles`' agent launch → older than this | must carry no `profile`/`cwd`/`command` (their own fields instead) | older raises only, never opens a terminal of the agent's profile |

  The last row is a constraint on `agents-profiles`, written here so the word can't be "silently taken for something else": an agent request puts its profile, directory and prompt in new fields (`agent_profile`, … or one `agent` object), never in `profile`/`cwd`/`command`.
- **`process R4` exception (confirmed by the user).** The instance request has never been versioned or negotiated: a launch writes one line and reads one answer, and an older instance speaks no greeting a launch could wait for. This change keeps that and relies on `R5`'s additive rule plus refusal by name. It is written into `docs/architecture/README.md`'s exceptions table as a `process R4` row ("the instance request: one unnegotiated JSON line, additive fields only, an unknown `kind` refused by name; answers `ok`, `error:` and `usage:`"), ended by a negotiated hand-off if a change ever needs a non-additive one.
- **Transcript tests** (`process R6`) in `instance.rs`: the exact encodings of a bare launch (`{}`), a terminal launch with profile, directory and command, and `brindle terminal` alone; the decoding of today's lines (pinned as string literals, so an older launch's request keeps decoding) to `Forward::Terminal`/`Raise`/`Attach`; an unknown `kind` to its refusal; and the three answer lines (`ok`, `error: …`, `usage: …`) both ways, including `usage:` read as a usage refusal. `Request::decode` is already fuzz-free JSON parsing of a bounded line (`MAX_REQUEST`); `asked()` never panics (`railway R5`).
- **`R11`** is untouched: the lock, the loser's hand-off and exit 0.

### 6a. The request's size (`process R7`)

- **Today.** `MAX_REQUEST` is `1 << 20`, 1 MiB (`src/instance.rs`). The instance reads the request through `take(MAX_REQUEST)` and `read_line`, and the launch reads the answer the same way. A request over 1 MiB isn't refused cleanly. The instance stops reading at 1 MiB with no line end, `serde_json` fails, and it answers `error: bad request: EOF while parsing …` and closes. Meanwhile the launch may still be writing the rest and get `sending the request: Broken pipe`, then fail to take the lock and exit 1. Either way the launch exits 1 with a message that doesn't name the cause.
- **This change.** `Request::encode` stays as it is, and a pure `instance::fits(line: &str) -> Result<(), String>` checks the encoded line, its `\n` included, against `MAX_REQUEST` before the launch connects. One over it sends nothing and exits 1 with `the launch is N bytes, over the 1 MiB a running Brindle takes`, which shows in the message box on Windows without a console. On the instance side, `read_request` tells a line cut off at the limit from a malformed one and answers `error: the request is over the 1 MiB limit`. Both are pinned by unit tests.
- **The limit stays 1 MiB, in this change and in `agents-profiles`.** Raising it would not help a newer launch handed to an older Brindle, whose limit stays 1 MiB, and no single argument a launch receives can come near it:
  - On Linux the kernel caps each argument string at 128 KiB (`MAX_ARG_STRLEN`, 32 pages). JSON's worst case, `\u00XX` for every control byte, is 6× that: 768 KiB.
  - On Windows the whole command line is at most 32,767 UTF-16 units, under 100 KiB of UTF-8.
  - Only a Linux launch with very many long arguments (`ARG_MAX` is about 2 MiB of argv and environment together) can exceed 1 MiB. The pre-send check refuses it with a clear message.
- **For `agents-profiles`.** A 256 KiB `-p PROMPT` can't arrive as one argument at all: Linux refuses to `exec` with a single argument over 128 KiB (`E2BIG`, before Brindle runs), and Windows' command line is smaller still. That change should cap the prompt at what can be passed, or read a long prompt from a file or stdin. Its "refuse an oversized launch before sending" is this change's `fits`, so it needs no limit change and no check of its own.

### 7. Help text

`HELP` becomes:

```
USAGE:
    brindle [OPTIONS]                          open the window (the default profile, or the restored tabs)
    brindle terminal [PROFILE] [-d DIR] [-e COMMAND [ARGS]...] [OPTIONS]
                                               open a terminal tab
    brindle --attach ID [OPTIONS]              open a tab on a detached session
TERMINAL:
    PROFILE                      a profile's name (default: config's default_profile)
    -d, --working-directory <DIR>
    -e, --command <CMD> [ARGS]   run CMD instead of the profile's program (must be last)
OPTIONS: … (as today, without -p/-d/-e)
DEBUGGING: … (as today)
```

`agents-profiles` adds its line and an `AGENT:` section.

### 8. Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `parse_args_from`, `Asked`, `TerminalLaunch`, `Command`, `Parsed`, `Info`, `TabKind` | pure (`fcis`), unit-tested in `main.rs` | `brindle` |
| `profile_for`, `LaunchRefusal`, `ProfileKind`, `Profile::kind` | pure | `brindle` |
| `Request::{encode, decode, asked}`, `Forward`, `fits`, the answer codec (`ok`/`error:`/`usage:`) | pure codec, transcript tests | `brindle` (`instance.rs`) |
| printing, `set_var("BRINDLE_CONFIG")`, the profile check's call before GPUI, exit codes, `show_unprinted` | shell (`main`) | `brindle` |
| `take_launches` answering a refusal (`usage:` for a `LaunchRefusal`) | shell (UI-thread task) | `brindle` |

No platform `cfg` is added: `show_unprinted` already sits in `main.rs`'s `ports R4` exception and calls `win.rs`. No shared crate changes.

## Risks / Trade-offs

- [Every script, desktop entry, shortcut and alias with `-p/-d/-e` breaks at once] → the error says exactly what to type, with the user's values; the release note gives the mapping; the `!` title puts it in the changelog's breaking section. Brindle's own launchers pass no arguments.
- [A Windows shortcut with `-p` fails with no console] → the message box (decision 3).
- [`brindle terminal` alone, handed to an older running Brindle, only raises it] → documented; it only happens while an older instance runs during an upgrade.
- [An agent launch handed to a Brindle from before this change] → constrained in decision 6: such a request carries no field an older instance acts on, so it only raises.
- [A launcher naming a profile stops working when the config has an error] → the user's decision; the refusal names the config error and the profiles in use, and the window's config line already shows the error.
- [A profile added to the config a moment before a launch is refused until the running Brindle reloads (about a second)] → the refusal lists the running Brindle's profiles, so the cause is visible; the launch can be repeated.
- [An older launch refused by this version for an unknown name exits 1, not 2] → only during an upgrade; it still prints the reason.

## Migration Plan

- Ships as one breaking release (`feat(cli)!: …`), before 1.0, so release-please bumps the minor version.
- The release note maps `brindle -p NAME` → `brindle terminal NAME`, `brindle -d DIR` → `brindle terminal -d DIR`, `brindle -e CMD…` → `brindle terminal -e CMD…`, says `brindle terminal` alone opens a new default tab, and says an unknown profile name is now refused (status 2) instead of opening the default profile.
- Rollback is reverting the commit; nothing is persisted in a new form (the request's `kind` lives for one exchange).

## Open Questions

None. The decisions that were open (a bare `brindle terminal` opening a tab, `--attach` refused with a subcommand, the long forms, no `--`, the unnegotiated hand-off, case-sensitive subcommand words) were confirmed by the user and are in decisions 1, 2 and 6. The unknown-name fallback was changed by the user into the refusal of decision 5.
