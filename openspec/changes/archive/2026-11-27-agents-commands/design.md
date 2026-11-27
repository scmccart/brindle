# Design

## Context

See proposal.md for why. Facts this design rests on, from the code on `feat/agents-session-list` (PR #150, 2026-11-26; first written against `feat/agents-profiles`, 2026-11-25), `agents-tool-content`'s design, the pinned schema crate's sources and the captured agent traffic:

- **The reducer ignores both updates today.** `acp/v1/decode.rs`'s `session_update` maps `available_commands_update`, `config_option_update` and `current_mode_update` to `forms::Update::Ignored { kind }`, which `reduce.rs`'s `Step::update` drops. `new_session_result` reads only `sessionId` (`forms::NewSession { session_id }`), and `continued_result(how, result)` returns `Result<(), BadParams>`, reading nothing. `initialize` sends `clientCapabilities: {"auth":{"terminal":true}}` (`acp/v1/encode.rs`).
- **What real ACP v1 agents send** (`remote-agent/tests/fixtures/acp/`):

  | Agent | Commands | `configOptions` in `session/new` | `modes` | Updates |
  | --- | --- | --- | --- | --- |
  | claude-agent-acp 0.112 | 4 (`debug`, `compact`, `context`, …), some with an input hint | `model` (14 choices), `effort` (thought level), `fast` (speed), `mode` (5) | yes | `config_option_update` |
  | codex-acp 2.12 | 2 (`review`, `compact`) | `model`, `reasoning_effort`, `mode`, `collaboration_mode` | yes | `config_option_update` |
  | copilot 1.0.115 | 4 | `mode`, `model`, `allow_all` (category `permissions`) | yes | `config_option_update` |
  | antigravity 1.5.16 | 1 (`compact`) | `model`, `thought_level`, `mode` | yes | `config_option_update` |
  | claude-code-acp 0.16.2 (deprecated) | 4 | none | yes | none |

  Every option captured is a `select` with a flat choice list, Claude's `fast` and Copilot's `allow_all` included (two choices each, not booleans; the toggles in this design's examples are the fake ACP agent's). Every agent with config options has one of category `mode` that mirrors its `modes`. Four agents also send an unstable `models` list, which the pinned crate has no type for. Copilot sends `available_commands_update` before it answers `session/new` (the reducer's `Phase::Creating`). The capturing client called `session/set_mode` (Claude, Codex, claude-code-acp) and `session/set_model { sessionId, modelId }` (Codex, Antigravity); every one was answered `{}`, Codex and Antigravity then sending a `config_option_update`.
- **ACP v1** (schema crate 1.22.0): a command is `{ name, description, input? }` whose only input kind is free text with a `hint`; commands are invoked by including `/name` in a prompt. A config option is `{ id, name, description?, category?, type }` with `type: "select"` (`currentValue`, `options` flat or in named groups) or `type: "boolean"` (`currentValue`); categories are `mode`, `model`, `model_config`, `thought_level` and any other string. `session/set_config_option { sessionId, configId, value }` (with `type: "boolean"` for a toggle) answers with the full option list. An agent may send boolean options only to a client that advertised `clientCapabilities.session.configOptions.boolean`. Session modes are `modes { currentModeId, availableModes }`, `session/set_mode { sessionId, modeId }` and `current_mode_update`.
- **ACP v2** (Preview): config options and `session/set_config_option` are the same shape; session modes are gone; a command list entry has a `type` (`command`, an unstable `skill`, or unknown, which a client "MUST NOT display as a slash command"), and its input a `type` (`text` or unknown).
- **`agents-tool-content`** gives `acp/forms.rs`, `acp/dialect.rs` with `Dialect { V1 }` (each method one `match self` onto `v1::decode` or `v1::encode`), `acp/v1/{decode, encode}.rs`, `forms::Request` (`Copy`, borrowing its strings) for the lines the reducer asks to be written, and the rule that only a version's module names its methods and fields (`tests/architecture.rs`, `acp_versions_stay_in_their_modules`). `forms.rs` already has an `OptionKind` (a permission option's) and `reduce.rs` a `Command` (a client's) and a `Choice` (how a continue goes), which the names below avoid (flag 1).
- **The reducer and the core** (`agents-sessions` to `agents-session-list`): `reduce::step(session, Input, now) -> Output`, a client's `reduce::Command` answered through `Output::reply: Option<Result<(), Refused>>`; `rpc::Rpc` keeps outgoing requests by id with a `Purpose` (`Purpose::List { req }` is never checkpointed, flag 4 of `agents-session-list`). The phases are `reduce::Phase`'s, `Choosing` included. The core turns a protocol 4 message into an `agents.rs` `AgentCommand` (`AgentCommand::list`, `::choose`, …), refused by name below protocol 4 through `policy::AgentRequest` and `agent_request_refusal`. The checkpoint (`acp/checkpoint.rs`) is format 2 (`FORMAT`), its state a JSON object with optional fields (`lists`, `chose`).
- **Client protocol 4** (`remote-protocol/src/client.rs`, `PROTOCOLS` 1 to 4, unreleased: v0.3.0 predates the terminal server) has one number space for both directions: clients' messages 1 to 31 (`MSG_AGENT_FETCH` 23, `MSG_AGENT_LIST` 31; 24 to 30 kept for this change and its siblings), the core's from 32 (`MSG_WELCOME`), and a second client block from 64 (`MSG_AGENT_CHOOSE`). Its encodings are pinned by `remote-protocol/tests/transcripts.rs` over `tests/fixtures/*.hex`, and fuzzed in `remote-protocol/src/fuzz.rs`.
- **Brindle** (`agents-profiles` to `agents-session-list`): `brindle-ui`'s `PromptInput` holds a `TextArea`, a toolbar, the queue row, a reason line and keyed action buttons (`set_reason`, `set_actions(Vec<PromptAction>)`, `PromptFocus::Action(key)`), with pure helpers (`enter_submits`, `escape_stops`); `ui/src/filter.rs` has `fuzzy_match` and `filter`; `src/picker.rs`'s `Palette` draws lists and `PaletteGroup`s of `PaletteRow`s, and the window's palettes are `src/workspace.rs`'s `PaletteKind`s (`Conversations` opened on an `AgentAsk::Conversations` from the zone). Agent actions (`agent_cancel`, `toggle_agent_thoughts`, `agent_conversations`, …) are rows of `src/actions.rs`'s `ACTIONS`, handled only by `AgentView` (`src/agent/view.rs`); `actions::palette_lists(action, PaletteFacts)` decides which the palette lists. The zone's requests go through `src/agent/mod.rs`'s `Requests` trait onto `remote::AgentLink` and `src/remote/local/session.rs`'s `Session`, and a request awaiting its reply is a `model::Sent`. The dump's agent block is `ChatModel::describe` and `wait_text` (`src/agent/model.rs`): the `--- agent` line, the login lines, the `--- note`s, the entries, `--- working`, `--- queued`, `--- conversation`, then `--- prompt …[ refusal=…][ reason=…][ actions=…]`. A refused link reaches the window's config line as a transient notice (`AgentView` emits `LinkFailed`, `Workspace::show_notice`; the dump's `--- config` line). Tab and shift-tab move among the zone's stops ("Keyboard path through an agent zone").

## Goals / Non-Goals

**Goals:**
- A user sees an agent's commands as they type `/`, and sees and changes the mode, model and other options of a conversation without leaving it.
- Options are Brindle's own form: session modes, config options and whatever v2 brings fill it inside their version's module, and Brindle never learns which.
- The server keeps commands and options, so a restored or reattached zone has them without asking the agent again.
- Nothing is set that the agent didn't offer, and a change of mode is on the record.
- No new keyboard stop in the zone and no change to a requirement another stacked change modifies.

**Non-Goals:** starting values for options in a profile, and remembering a choice across sessions; command inputs other than free text, and v2's unstable skills; running a command from the command palette; completing anything but a command's name (arguments, file paths, `@` mentions); option controls as tab stops; the conversation's title and context usage (`agents-session-info`). No config key and no state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `SlashCommand`, `SessionOption`, `OptionCategory`, `OptionControl`, `OptionChoice`; `Event::Commands`, `Event::Options`; the transcript's state, `replay` and `page`; `MAX_COMMANDS`, `MAX_OPTIONS`, `MAX_CHOICES` | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` (and `fuzz.rs`) |
| `MSG_AGENT_SET_OPTION` (24), `ToCore::AgentSetOption(AgentSetOption)` and `OptionValue` | wire | `brindle-remote-protocol`, `remote-protocol/src/client.rs` (and `fuzz.rs`) |
| The forms: `Offered` (commands, options with their keys), `OfferedOption`, `OptionKey`, `Update::{Commands, Options, Mode}`, `NewSession::offered`, `Request::SetOption` | pure, plain data | `brindle-remote-agent`, `acp/forms.rs`; `Dialect`'s methods in `acp/dialect.rs` |
| ACP v1: commands, config options, modes, the model list and their updates; `session/set_config_option`, `session/set_mode`, `session/set_model`; the boolean capability | pure | `acp/v1/{decode, encode}.rs` |
| Keeping, bounding and setting options; `Purpose::SetOption`; the mode notice; emptying on a stop | pure | `acp/reduce.rs`, `acp/rpc.rs` |
| The keys and kept mode values in the checkpoint | pure | `acp/checkpoint.rs` |
| The set request, its checks and reply | shell (loop handler) | `remote-agent/src/server/core/{clients, agents, policy}.rs` |
| The client's request (`Requests::set_option`, `AgentLink::set_option`, `Session::set_option`) | shell | `src/agent/mod.rs`, `src/remote/`, `src/remote/local/{session, connection, endpoint_env}.rs` |
| `command_text`, `completion`, `local_command`, `option_items`, `next_mode`, the options palette's rows, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` (folding in `src/agent/shared.rs`, re-exports in `src/agent/transcript.rs`) |
| The suggestion list, the footer, the two actions | shell (entity) | `src/agent/view.rs`, `src/agent/mod.rs`, `src/actions.rs` |
| The options palette (`PaletteKind::AgentOptions`, opened on `AgentAsk::Options`) and the refusal's transient notice | shell (entity) | `src/workspace.rs` |
| `PromptInput`'s suggestions and footer items | components | `brindle-ui`, `ui/src/components/prompt_input.rs`, shown in `ui/examples/gallery.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`).

## Decisions

### 1. Brindle's own forms (`types R1`, `R2`, `R7`)

```rust
pub struct SlashCommand { pub name: String, pub description: String, pub hint: Option<String> }

pub struct SessionOption {
    pub id: String,                       // Brindle's id for it within the session, which a set names
    pub name: String,
    pub description: Option<String>,
    pub category: OptionCategory,
    pub kind: OptionControl,
}
pub enum OptionCategory { Mode, Model, ModelConfig, ThoughtLevel, Other { name: String } }
pub enum OptionControl {
    Select { current: String, choices: Vec<OptionChoice> },
    Toggle { on: bool },
}
pub struct OptionChoice { pub value: String, pub name: String, pub description: Option<String>, pub group: Option<String> }
```

(Named `SlashCommand`, `OptionControl` and `OptionChoice` rather than `Command`, `OptionKind` and `Choice`, which `reduce.rs` and `forms.rs` already use; flag 1.)

- **What the types rule out.** An option is a list with a current value or a toggle, never both. Grouped choices are one list whose items name their group, so "grouped" and "flat" aren't two shapes to handle. `OptionCategory::Other { name }` keeps a category Brindle doesn't know (`speed`, `permissions`, `collaboration_mode`), as `StopReason::Other` does. No hint and no description are `None`.
- **A `Select` whose current value isn't among its choices** is kept as sent: the zone shows the value itself where it would show the choice's name. Agents do this while a model list loads.
- **Commands and options are session state, not log entries**, like the queue slot and the agent's state: the transcript holds the latest list of each, whole.
- **Ids.** `SessionOption::id` is the agent's config option id for a config option. For the option made from session modes (decision 3) it is `mode`, or `mode` followed by `'` when a config option already has that id, so ids are unique within the list.

### 2. Events, the fold and replay (`brindle-transcript`; `process R4`–`R6`)

```rust
Event::Commands { commands: Vec<SlashCommand> } // the whole list, replacing the last
Event::Options  { options: Vec<SessionOption> } // the whole list, replacing the last
```

- Both replace, so no fold error is possible beyond decoding. `Transcript` gains `commands` and `options`, each starting empty; `replay()` (today: entries, `Queue`, `Conversation`, `Turn`, `Agent`) sends each after the agent's state when it isn't empty; `fold(replay(t)) == t` gains them.
- The reducer emits an event only when the list differs from the one the transcript holds (agents repeat them).
- **Bounds** (`railway R5`), applied by the reducer before the event: at most `MAX_COMMANDS` = 256 commands and `MAX_OPTIONS` = 32 options with `MAX_CHOICES` = 256 choices each; an option id or choice value over 1 KiB leaves its item out (a cut one would set something else, as `MAX_LISTED_ID` does for a conversation), option and choice names, hints and group names are cut at 256 bytes (`MAX_SESSION_TITLE`'s bound) and a command name over 256 bytes leaves its command out, descriptions at 1 KiB (flag 6); every text cleaned (`reduce::clean`). A list is also cut so its event stays under `TEXT_CHUNK` (64 KiB). What is dropped is said once in a notice ("the agent offered N commands; the first 256 are kept").
- **A command's name must be typeable:** a name that is empty, or holds whitespace, a control character or `/`, is left out. So is a name that starts with `brindle:`, which is Brindle's own (decision 6).
- **Versions.** Client protocol 4 is unreleased and amended, as `agents-tool-content`'s decision 8 says: two event kinds, with new `tests/fixtures/v4/` files and none edited. The checkpoint is format 2 (`agents-log-paging`), amended: the two lists travel as events in its base and segments, and the option keys (decision 3) are in its state; a checkpoint from before this change decodes with both empty (decision 12).

### 3. Options come from the dialect, each with its key (`acp/forms.rs`, `acp/v1/`)

```rust
pub struct OfferedOption { pub option: SessionOption, pub key: OptionKey }
pub enum OptionKey { Config { id: String }, Modes, Models }
pub struct Offered { pub options: Vec<OfferedOption>, pub unreadable: usize }   // NewSession::offered, continued_result's Ok
Update::Commands { commands: Vec<SlashCommand>, unreadable: usize }
Update::Options(Offered)              // config_option_update
Update::Current { key: OptionKey, value: String }  // current_mode_update, keyed OptionKey::Modes
Request::SetOption { id: u64, session: &'a str, key: &'a OptionKey, value: &'a OptionValue }
```

`Request` is `Copy`, so the value is borrowed. `Dialect` gains `set_option_result(key, result) -> Result<Option<Offered>, BadParams>` (the whole list for a config option, `None` for `session/set_mode`'s and `session/set_model`'s empty answers); `new_session_result` returns `NewSession { session_id, offered }`, and `continued_result(how, result)` returns `Result<Offered, BadParams>` (decision 12).

- **Config options** (v1 and v2): `session/new`'s `configOptions`, `config_option_update` and the answer to `session/set_config_option` each give the whole list, decoded item by item from the raw JSON so one unreadable option loses only itself. Each gets `OptionKey::Config { id }`. `select` with flat or grouped choices and `boolean` are read; another `type` is left out.
- **Session modes** (v1 only): when `session/new` gives `modes` and no config option has category `mode`, the v1 module makes one option from them (name "Mode", category `Mode`, a `Select` of the modes) with `OptionKey::Modes`; `current_mode_update` changes its current value (`Update::Current`, which the reducer applies to the held option whose key equals the update's, comparing keys whole and never naming a variant; with no such option it changes nothing). When a config option has category `mode`, the `modes` and `current_mode_update` are ignored: the agent mirrors them in its config options, and one control is enough.
- **The key is opaque to the reducer** (`ports R4`, `agents-tool-content`'s rule): it stores each offered option's key beside the option and hands it back in `Request::SetOption`; only `acp/v1/encode.rs` reads it, writing `session/set_config_option` (with `type: "boolean"` for a toggle), `session/set_mode` or `session/set_model`. `OptionKey::Modes` and `OptionKey::Models` exist in `forms.rs` because a key must be storable; nothing outside `acp/v1/` constructs or matches them, which `acp_versions_stay_in_their_modules` gains a line for.
- **The boolean capability:** the v1 module's `initialize` advertises `clientCapabilities.session.configOptions.boolean: {}`, since the zone can show a toggle.
- **The unstable `models` list** (v1 only; the user's decision, 2026-11-25, for broad coverage): when `session/new` gives `models` (`{ currentModelId, availableModels: [{ modelId, name, description? }] }`, as the claude-code-acp 0.16.2 capture has it) and no config option has category `model`, the v1 module makes one option from it (name "Model", category `Model`, a `Select` of the models) with `OptionKey::Models`, set with `session/set_model { sessionId, modelId }`. With a `model` config option present the list is ignored, as modes are. Today that makes a difference for one agent: the deprecated claude-code-acp, which offers only modes and this list.
  - The pinned crate has no type for any of it, so `acp/v1/decode.rs` reads it by hand from the raw JSON, and a list it can't read is left out without a notice (an agent that sends it has done nothing wrong by ACP).
  - `session/set_model` isn't in the pinned crate. Its shape is in the Codex 2.12 and Antigravity 1.5.16 captures (`{ sessionId, modelId }`, answered `{}`), but not claude-code-acp's, the one agent it matters for. Task 1.5 captures the adapter taking it before anything relies on it. If the adapter refuses the method, the option isn't made, and "As built" says so.
  - No update for the current model is known, so its value changes only through Brindle's own set, whose empty answer the reducer treats as it does `session/set_mode`'s.
- **Commands:** `available_commands_update` gives the whole list. v1's only input kind gives the hint.

**What ACP v2 maps to** (for `agents-acp-v2`): config options and `session/set_config_option` as above; no modes and no model list, so `OptionKey::Modes` and `OptionKey::Models` are never produced; a command entry of `type: "command"` is a `SlashCommand`, any other type is left out; an input of `type: "text"` gives the hint, any other gives none.

### 4. Setting an option (`acp/reduce.rs`, the core; `railway R1`, `R6`)

- **The message.** `AGENT_SET_OPTION { req, session, option, value }` (`MSG_AGENT_SET_OPTION` = 24, `ToCore::AgentSetOption(AgentSetOption)`) with `OptionValue::{ Choice { value: String }, Toggle { on: bool } }`, a new client message of protocol 4 (amended): `AGENT_RESTART` is 21, `AGENT_LOGIN` 22, `AGENT_FETCH` 23, `AGENT_LIST` 31 and `AGENT_CHOOSE` 64; 24 was kept for this message when `agents-session-list` numbered its own (its decision 9). It joins `policy::AgentRequest` (`SetOption`, named `AGENT_SET_OPTION`) for the refusal by name below protocol 4, and the core makes it an `AgentCommand::set_option` carrying `reduce::Command::SetOption { option, value }`. Protocol transcripts (`remote-protocol/tests/fixtures/*.hex`) pin its encoding, and it joins the protocol's fuzzing.
- **Checked before anything is sent** (the outcome is the reducer's `Output::reply`, `Result<(), Refused>`, answered `OK` or `ERROR` with the reason; flag 2): `Refused::NotReady` (the agent isn't ready: every phase but `Ready`), `Refused::NoSuchSessionOption { id }`, `Refused::ChoiceNotOffered { id, value }` (a choice the option doesn't list, or the wrong kind of value), `Refused::OptionBusy` (an earlier set for this session hasn't been answered: an outgoing `rpc::Purpose::SetOption` exists). A set naming the current value is `OK` and sends nothing.
- **The set out** is `Purpose::SetOption { id, value }` in `Rpc`'s outgoing requests, which the answer is read against (the option's name and value for a refusal's notice; the value an empty answer sets). Like `Purpose::List`, it isn't checkpointed, but its id is (`Moment::BeforeRequest`, as `agents-session-list`'s flag 14 has it for a list), so `OptionBusy` reads no state field (flag 3).
- **Any time the agent is ready**, during a turn too: ACP lets an agent take a change mid-turn or refuse it.
- **The agent's answer is the truth.** Its result carries the whole list, which becomes `Event::Options` if it differs. For `session/set_mode` and `session/set_model`, whose results are empty, the reducer sets that option's current value itself. Brindle shows nothing as changed until the event arrives.
- **The agent refuses** (a JSON-RPC error): the options stay, and the log gets a notice, "the agent didn't change NAME to VALUE: MESSAGE". The client's `OK` meant "asked", so the failure is shown where the session's other failures are, in the conversation.
- **The agent never answers:** `OptionBusy` holds until the agent stops, which clears it with the lists (decision 12), as for any pending request.

### 5. A mode change is on the record

When an option of category `Mode` gets a different current value, the reducer adds an info notice to the log, "Mode: NAME" (the choice's name, else its value), whoever caused it: this client, another, or the agent itself. The first list a session receives adds none. Other categories add none: a model or thinking level changes nothing about what the agent may do unasked, and some agents adjust them on their own at the start of a session.

### 6. The chat model (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **Whitespace before the slash makes no difference** (the user, 2026-11-25: an accidental space mustn't change behaviour). `command_text(text) -> Option<&str>` gives the text from its first character that isn't whitespace when that character is `/`, else `None`. Completion, `local_command` and what is sent all start from it: an option command after a space is carried out, and an agent's command after a space is sent without the space. A prompt that doesn't begin with `/` after its whitespace is sent exactly as typed, indentation included.
- **Completion.** `completion(text, cursor, commands) -> Option<Completion>`: `Some` when `command_text` is `Some`, the cursor is inside its first word, and at least one command matches the word after `/` with `ui`'s `fuzzy_match`; names that start with the word come first, then the rest in the agent's order. `Completion { range, items, highlighted }`; `accept(completion) -> (String, usize)` replaces the range with `/NAME ` and gives the new cursor. The model keeps `dismissed: bool`, set by escape and cleared when the text stops being a command word, so a dismissed list doesn't come back on the next key.
- **Option commands** (the user's condition for the labels, 2026-11-25: options must be changeable by slash command too). `option_commands(options, agent_commands) -> Vec<OptionCommand>` gives one command of Brindle's own per option: its name is the option's name in lower case with each run of characters other than ASCII letters and digits as one `-`, trimmed (`mode`, `model`, `reasoning-effort`, `fast-mode`); an empty name is skipped, a second option with the same name gets `-2`, and a short name the agent's own commands hold is the agent's, so the agent's command is the one that name runs. Its description is the option's name and current value, its hint `<choice>` or `on|off`. They are listed with the agent's commands, after them.
- **Every option command also has a long name, `brindle:` and its name** (the user, 2026-11-26: when an agent's command wins a name, Brindle's must still be callable). `/brindle:model sonnet` is always Brindle's, whatever the agent offers, and works with every agent, so it can be typed from habit. `OptionCommand { name, long, shadowed }`: an option command whose short name an agent's command holds isn't dropped any more; it is `shadowed`, reachable by its long name alone. The list shows each option command once: under its short name, or under its long name when it is shadowed or when the word typed starts with `brindle:`. A word matches an option command by either name. The second stage and `local_command` take both names alike. No agent command can hold a long name: the server leaves out names that start with `brindle:` (decision 2).
- **The second stage.** When the text is an option command, a space and anything after it on one line, `completion` returns that option's choices matching the rest of the line (by name or value, case-insensitive, fuzzy with prefix matches first), with the highlight starting on the current choice when nothing is typed. A toggle offers `on` and `off`.
- **`local_command(text, options, agent_commands) -> Option<Local>`** (pure) decides what a submit does before anything is sent: `None` (send the text as a prompt), `Set { id, value }`, `Choose { id }` (an option command alone: show its choices; a toggle alone flips), or `NoSuchChoice { option, text }`. Only text whose `command_text` is one line and names an option command is local. `Set` clears the input and sends `AGENT_SET_OPTION`, during a turn too, and never touches the queue; `NoSuchChoice` keeps the text and shows its reason beside the input, where a refused prompt's reason shows. A local command is never a user message: only the server's mode notice records it.
- **What stays text:** a first prompt from `brindle agent -p` and a queued or held prompt are sent as they are, option command or not; so is text with more lines after the command's line. There is no way to send an option command's own text to the agent as a prompt, unless the agent offers a command of that name, in which case the agent's is the one that runs and Brindle's is reached by its long name.
- **Keys while the list shows** (`completion_key(key) -> CompletionAction`, pure): up and down move the highlight, wrapping; enter and tab accept (a command's name in the first stage; in the second, tab writes the choice's name into the text and enter sets it); escape dismisses. They are taken before the prompt input's own handling, so enter doesn't send and tab doesn't move focus. With no list, every key does what it did.
- **Option items.** `option_items(options) -> Vec<OptionItem>`: the options in the order mode, model, thinking level, model config, then others in the agent's order, each `{ id, label }` where the label is the current choice's name, or the option's name followed by `on` or `off`.
- **`next_mode(options) -> Option<(String, OptionValue)>`:** the first option of category `Mode` that is a `Select`, and the choice after its current one, wrapping; `None` with no such option or fewer than two choices.
- **The options palette's rows:** one row per option ("Model: opus", its description as the detail); choosing a `Select` shows its choices (grouped under their group names, the current one marked), choosing a `Toggle` flips it.

### 7. The zone and `brindle-ui` (`ports R2`)

- **`PromptInput`** gains `set_suggestions(Option<Suggestions>)` (plain rows: a label, a hint, a description, and the highlighted index), drawn as a list above the textarea that takes no focus, a press on a row asking the owner to accept it; and `set_footer(Vec<FooterItem>)` (a label and a key each), drawn under the textarea as small ghost buttons separated by `·`, a press asking the owner with the item's key. Neither is a keyboard stop, so `row_stops` and "Keyboard path through an agent zone" are unchanged.
- **A press on a footer item** opens that option's choices in the window's palette (a `Select`), or flips it (a `Toggle`). The zone asks for the palette as it asks for the conversations palette: `Agent` emits `AgentAsk::Options { option: Option<String> }` (none for the whole list), and `Workspace` opens a `PaletteKind::AgentOptions` over the zone, whose choice the zone then sends (flag 7).
- **Actions:** `agent_options` ("Agent Options…") opens the options palette; `agent_next_mode` ("Next Agent Mode") sends `next_mode`'s set. Neither has a default key; both are rows of `ACTIONS` handled only by `AgentView`, so the palette lists them only in an agent zone, and only when the session has options (`agent_next_mode`: a mode option with two or more choices): `PaletteFacts` gains `agent_options: bool` and `agent_modes: bool`, read by `palette_lists` (flag 8).
- **A set the server refuses** (its `ERROR`, a `Refused` of decision 4) shows its reason on the window's notice line: a transient notice on the config line, as a refused link's is (`AgentView` emits it, `Workspace::show_notice`; the dump's `--- config` line), and nothing in the zone changes. The request is a `model::Sent::SetOption`.
- **Escape** closes the suggestion list first; a second escape does what escape did.

### 8. The dump (`testing R2`)

```
--- commands count=4
--- option id="mode" category=mode kind=select current="auto" name="Mode" choices=5
--- option id="fast" category=other:speed kind=toggle current=off name="Fast mode"
--- completion query="co" highlighted=0
/compact  <optional custom summarization instructions>  Free up context by summarizing the conversation so far
/context    Show current context usage
```

In the second stage the line names the option command, and the rows are its choices: `--- completion command="model" query="so" highlighted=0`, then `Sonnet  …`.

- `--- commands` and the `--- option` lines come after the `--- agent` line, its login lines and its `--- note`s, before the first `--- entry`, whenever the lists aren't empty. `--- completion` and its rows (as palette rows print) come after the `--- prompt` line while the list shows. The `fast` toggle above is the fake ACP agent's; Claude's `fast` is a two-choice select (`kind=select … choices=2`).
- `wait_text` holds them, so `--wait-for 'current="plan"'` works. `--key` drives the list (`down`, `enter`, `tab`, `escape`), and `--action agent_options`, `agent_next_mode` and the palette's own actions drive the options.

### 9. Failures (`railway R6`, `R8`)

- An option or command that can't be read, or past a bound: left out, one notice.
- A set the server refuses before sending: `ERROR` with the reason, shown on the notice line; nothing changes.
- A set the agent refuses: a notice in the conversation; the options stay.
- A set the zone can't send (its link is down): `ChatModel::unsent`'s refusal beside the prompt input, as for any request. There is no client-side gate by server version: an agent session exists only on a protocol 4 connection, and protocol 4 is unreleased and ships with this message (flag 4).

### 10. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Commands, options and their keys are in the session's checkpoint. A core restart adopts them; a set in flight at the restart is lost with its `Purpose::SetOption` (so no `OptionBusy` outlives it), and the option shows its old value until the user sets it again or the agent reports it. A Brindle crash loses nothing: both lists replay on attach. An upgrade's next core reads the same format 2. An agent that stops and is continued is decision 12's.

### 11. Tests

- **Unit:** the fold and replay over both events; `v1::decode` over every captured `session/new`, `available_commands_update` and `config_option_update`, grouped choices, a boolean, an unknown `type`, an unreadable option, modes with and without a mode option, `current_mode_update`; `v1::encode` for each key and value; the reducer's bounds, equal lists emitting nothing, every set refusal, the agent's refusal, the mode notice (not for the first list, not for other categories); the model's completion, keys, items, `next_mode` and dump lines.
- **Captures:** the replay test asserts each agent's commands and options.
- **Protocol:** fixtures and fuzzing for `AGENT_SET_OPTION`.
- **Real roles** (the fake ACP agent; `remote-agent/tests/agents.rs`, the restart and upgrade cases in `upgrade.rs`): a set reaches the agent and its answer reaches every attached client; a refusal; a restart keeps both lists.
- **E2E** (`tests/e2e/cases/agent.rs`, each `Case::needs_fake_agent()`, gated with `Gate` and the fake agent's `wait-file`): `agent-command-completion`, `agent-option-command`, `agent-option-set`, `agent-option-modes`, `agent-option-refused`, `agent-option-restore`, `agent-option-continue`.
- **Manual:** pointer presses on a suggestion row and a footer item; a real agent's `/compact` and a mode switch.

### 12. The built agent stack (refresh, 2026-11-26)

This design was written before `agents-session-recovery`, `agents-login`, `agents-session-title`, `agents-zone-attention`, `agents-zone-theme-scope` and `agents-log-paging` were built. What they change here, each a decision of this design from now on:

- **The lists belong to one run of the agent** (the user's decision, 2026-11-26). An agent zone now outlives its agent: an exited or lost agent's zone stays, with Continue, and a terminal login hangs the agent up and starts it again. When the agent stops, whatever the reason (`Exited`, `Lost`, `Refused`, `Broken`, `Restarting`), the reducer empties both lists, sending `Commands` and `Options` with empty lists when they held anything, and drops the keys and any `Purpose::SetOption` out. `Step::end` (exit and loss), `Step::refuse`, `Step::fail` and `Step::restart_for_login` are where those phases begin. An agent that needs a login mid-conversation (`Phase::LoginRequired` with `AfterLogin::Ready`) or logs in (`Phase::LoggingIn`) hasn't stopped: it keeps its lists, and sets are refused `NotReady` meanwhile (flag 5). The agent that runs next fills them again. So a stopped agent's zone shows no labels and offers no commands, and nothing stale can be set. *Rejected:* keeping the last lists on view, greyed, until the next agent answers.
- **Every start answers with options.** ACP's `session/load` and `session/resume` answers carry `modes` and `configOptions` as `session/new`'s does (schema crate 1.22.0, `LoadSessionResponse`, `ResumeSessionResponse`), and the unstable `models` list is read from them the same way. The v1 module decodes all three answers through one function (behind `new_session_result` and `continued_result`), and the reducer takes the options from whichever start the run made, in `Step::ready`, which today emits the `Conversation`, the notice and `Ready`. A conversation chosen in `Phase::Choosing` (`agents-session-list`) is such a start: a choosing agent has no conversation, so it lists nothing until it has chosen. Commands an agent sends before its start's answer (Copilot's, in `Phase::Creating`) are kept, since the reducer already takes every update in that phase as the session's.
- **A load's replay changes the lists and nothing else.** During `session/load` the agent replays its conversation, `available_commands_update`, `config_option_update` and `current_mode_update` among it. As a replayed title is (`agents-session-title`; `Step::update` handles `Update::SessionInfo` before `Step::mute`), they are state, not log: they apply whether the replay is muted or shown, aren't counted among the muted updates, and add no notice. The load's answer comes after the replay, and its options are the ones that stand.
- **A mode change across runs is still on the record** (decision 5). The reducer keeps, across a stop and in the checkpoint, the current value of each `Mode` option by its id as last in effect. The first list of a run adds "Mode: NAME" for a mode option whose value differs from the one kept, so an agent that comes back from a Continue in another mode says so. A session's first run has nothing kept and adds none, as before. Nothing is compared during a load's replay, only once the run is ready.
- **`NotReady`** (decision 4) is every phase but `Ready`: spawning, initializing, creating, resuming, loading, choosing, needing or running a login, restarting, and stopped. It is the reducer's existing `Refused::NotReady` ("the agent isn't ready") for all of them, not the phase-specific refusals a prompt gets, as the spec's scenarios have it (flag 2). With no options listed there are no option commands, so `/mode plan` typed at a stopped agent is text, refused as any prompt to a stopped agent is.
- **The number** is 24 (decision 4).
- **The checkpoint is format 2** (`agents-log-paging`, a base and appended segments). `Commands` and `Options` are log events: in the base's log as `replay()` emits them, and in a segment's `events`. The option keys and the kept mode values are state fields, written whole in every record and left out when empty (as `lists` and `chose` are), so a state written before reads as none; with at most 32 options they stay small. A set out is not a state field: its `Purpose::SetOption` is left out of the checkpoint as `Purpose::List` is (flag 3). A format 1 or format 2 checkpoint from before this change decodes with empty lists.
- **Pages.** Both lists are state outside the entries, so `Transcript::page` emits them where `replay()` does, after `Agent`, in every page and outside the page's budget (each event is under 64 KiB). A fetch (`AGENT_FETCH`) carries entries only. The page laws (`fold(page(t, b)) == t.suffix(from)`) gain both lists.
- **`initialize`.** `clientCapabilities` is `{"auth":{"terminal":true}}` today; the boolean option capability is added beside it.
- **Drawing.** The suggestion list and the footer are parts of the zone, drawn from the zone's token set in the profile's theme (`agents-zone-theme-scope`); the options palette is the window's. Neither moves, so "Motion only while an agent works" is untouched.
- **The dump.** `--- commands` and the `--- option` lines follow the `--- agent` line and its `--- note`; `agents-zone-attention`'s `--- working`, `--- spinner` and `--- motion` lines and paging's `earlier=` are unaffected.

### 13. Decided while refreshing (flags)

The design was checked against `feat/agents-session-list` (PR #150) on 2026-11-26, and what the code left open was decided as recommended, each the simplest choice the architecture docs allow. For review:

1. **The transcript's forms are `SlashCommand`, `OptionControl` and `OptionChoice`** (decision 1), not `Command`, `OptionKind` and `Choice`: `reduce.rs` has a `Command` (a client's) and a `Choice` (how a continue goes), and `forms.rs` an `OptionKind` (a permission option's), and both files import from `brindle-transcript`, so the plain names would clash where they are used most. `SessionOption`'s field stays `kind`.
2. **The set's refusals are `reduce::Refused` variants**, since a command's answer is `Output::reply: Option<Result<(), Refused>>`: the existing `NotReady` for every phase but `Ready` (the spec's "isn't ready" both before the start's answer and while stopped), and new `NoSuchSessionOption { id }`, `ChoiceNotOffered { id, value }` and `OptionBusy` (`NotOffered` is taken by the list's). There is no separate `SetRefusal` type.
3. **A set out is `rpc::Purpose::SetOption { id, value }`**, and `OptionBusy` is "one is outgoing": no separate pending mark. Like `Purpose::List` it is left out of the checkpoint while its id is counted (`Moment::BeforeRequest`), so an adopted session has none out, and the agent's late answer answers no request and is ignored.
4. **No client-side gate by server version** (decision 9, task 4.1 as first written): an agent session exists only on a protocol 4 connection, and protocol 4, unreleased, ships with `AGENT_SET_OPTION`; the core's refusal by name below protocol 4 (`policy::AgentRequest::SetOption`) stays. *Alternative:* a capability bit in `WELCOME`, a wire change for no released peer.
5. **A login mid-conversation keeps the lists** (decision 12): `LoginRequired` after a prompt (`AfterLogin::Ready`) and `LoggingIn` are the same run of the agent in the same ACP session, so its commands and options stand, and a set is refused `NotReady` until it is ready again. Only the phases where the agent is gone or hung up (`Exited`, `Lost`, `Refused`, `Broken`, `Restarting`) empty them.
6. **The bounds' numbers** (decision 2): an option id or choice value over 1 KiB leaves its item out (a set names it, so it can't be cut, as `MAX_LISTED_ID`); names, hints and group names are cut at 256 bytes, as a title is (`MAX_SESSION_TITLE`); descriptions at 1 KiB. A command name over 256 bytes is left out, since a cut name would run another command.
7. **The options palette is the window's** (decision 7): the zone emits `AgentAsk::Options { option }` (`None` for Agent Options…, an id for a footer press), `Workspace` opens `PaletteKind::AgentOptions` over the zone as it opens `PaletteKind::Conversations`, and the choice goes back to the zone, which sends the set through its own link. A press on a toggle's footer item flips it without a palette.
8. **The palette lists the two actions from new `PaletteFacts` fields**, `agent_options` and `agent_modes`, filled from the active agent zone's model when the facts are gathered (`Workspace`, beside `PaletteZone::Agent`) and read by `actions::palette_lists`, the one place listing is decided.
9. **The forms' JSON follows `StopReason`'s** (internally tagged, snake case): `{"category":"other","name":"speed"}`, `{"control":"select","current":"opus","choices":[…]}` and `{"control":"toggle","on":false}`; `OptionValue` is `{"kind":"choice","value":"sonnet"}` or `{"kind":"toggle","on":true}`. Optional fields are left out when unset, as elsewhere in protocol 4.
10. **`current_mode_update` decodes as `Update::Current { key, value }`** with the key `OptionKey::Modes`, and the reducer applies it to the held option whose key equals it, comparing keys whole. So the reducer never names `OptionKey::Modes` or `::Models`, and the architecture test's new line (only `acp/forms.rs` and `acp/v1/` name them) holds for `reduce.rs` too.
11. **A refused set's notice travels as its own event** from `AgentView` to `Workspace::show_notice` (beside `LinkFailed`, passed on by the zone tree as `LinkFailed` is), not as a `LinkFailed`, whose name says what it is for.
12. **Task 1.5 stays**: the shape of `session/set_model` is pinned by the Codex and Antigravity captures, but claude-code-acp, the one agent whose model only it sets, hasn't been captured taking it.
13. **This change is implemented on top of `agents-session-list`** (the branch is stacked on PR #150); the tasks' check that it is archived first is met by its merge, and its `Phase::Choosing`, `PromptInput::set_actions` and message 64 are taken as given.
14. **The texts** (server half, as built): the refusals read "the agent offers no option \"ID\"", "option \"ID\" offers no choice \"VALUE\"" (a toggle sent to a select, or a choice to a toggle, names `true`/`false` as its value) and "an earlier change of an option hasn't been answered yet", each after the core's `session N: `; the agent's refusal of a set is a warning notice "the agent didn't change NAME to SHOWN: WHY" (SHOWN the choice's name, or `on`/`off`), and an answer Brindle can't read gives WHY "its answer isn't one Brindle can read"; a bounded list is one warning notice, "the agent offered N commands; K are kept" (or "options").
15. **A start's events come in the order conversation, options, notice, ready**: the options are emitted in `Step::ready` after the `Conversation` event and before the start's notice and `Ready`, so a client that folds `Ready` already holds them; commands come whenever the agent sends them, `Phase::Creating` included.
16. **`rpc::Purpose` loses `Copy` and `Ord`** to carry `SetOption { id, value }`; it was matched and cloned, never ordered or copied in a way that mattered.
17. **Reading an option item by item**: a missing or `null` `category` is `Other { name: "" }`, a category that isn't a string makes the item unreadable; a `select` without `currentValue` or `options` is unreadable, its unreadable choices are dropped and the option kept; an unknown `type` is left out uncounted (as the design's "another `type` left out"), and so is a `modes` or `models` value of the wrong shape (the option is just not made). A `configOptions` that isn't an array counts as one unreadable item.
18. **The bounds skip, they don't stop**: an item that alone would push its event past `TEXT_CHUNK` is left out and the next is tried; ids and values over 1 KiB or holding a control character are left out rather than cleaned (a set names them), while names, descriptions and hints are cleaned and cut.
19. **A config list keeps the options made from modes and models** (`Dialect::merge_options`) until it holds an option of that category itself; a kept option whose id the list now uses gets `'` appended, so ids stay unique.
20. **The mode notice compares by phase**: an update while ready compares with the option held (or, with none held, the mode last in effect); a run's first list compares with the mode last in effect; a load's replay and the phases before ready compare with nothing and leave the mode last in effect alone, so a replayed mode update never adds a notice.
21. **A checkpoint's keys must match its options exactly**: `option_keys` holding a key for an option the record lacks, or lacking one it holds, makes the checkpoint unreadable (`Unreadable::Field { name: "option_keys" }`) rather than leaving an option no set can reach.
22. **"Two attached clients" in the real-role tests is an attach after**: a session has one attached client at a time, so task 3.1's and 3.2a's second client attaches once the first has detached, and its replay must hold both lists.
23. **The pure completion code is its own module**: `completion`, `accept`, `completion_key`, `option_commands`, `local_command`, `option_items`, `next_mode` and the options palette's rows live in `src/agent/commands.rs` (a whole pure module, in `PURE_MODULES` and `fcis`'s table), not in `model.rs`, which keeps the list's state (`draft`, `completion`, `dismissed`) and calls them.
24. **A name is matched as it is typed**: a word starting with `brindle:` matches the long names only, any other word the short names only, so `/re` doesn't fuzzy-match `brindle:` letters. A shadowed option command is listed by its long name.
25. **The dump's category words** are `mode`, `model`, `model_config`, `thought_level` and `other:NAME`, the wire's names in snake case.
26. **A refused set's notice reads `NAME wasn't changed: WHY`**, WHY the server's reason without its `session N: ` prefix; a set that couldn't go at all (no link) shows `UNSENT` beside the input as any request's does.
27. **A bare option command opens the options palette on its choices and clears the input** (`Submit::Choose`), as a footer press on a select does.
28. **A press on a suggestion row is enter on it**: a command row writes its name and a space, a choice row sets the choice. The press takes no focus.
29. **The suggestion keys are bindings, not key listeners**: `PromptSuggestionUp`/`Down`/`Enter`/`Tab`/`Escape` in the context `PromptSuggesting > TextArea`, which the prompt input adds only while suggestions show, so they come before the text area's and the zone's own; enter goes on (`propagate`) while an input method composes.
30. **`AgentAsk` is `Clone`, not `Copy`**, since its new `Options { option }` variant carries the option's id; `shared::Changes` gains `lists_changed` for the two list events, which no longer count as a turn or agent change.
31. **The options palette marks the current choice with `current` in its detail**, and its highlight starts on the first row, not the current choice (a palette has no initial highlight to set).
32. **Agent Options… and Next Agent Mode are listed only in an agent zone**: the palette's facts read the active zone's model (`ChatModel::has_options`), and a terminal zone's are always false.
33. **The e2e cases live in `tests/e2e/cases/agent_options.rs`**, not `agent.rs` (already 3,800 lines), as `conversations.rs` holds `agents-session-list`'s; they use `agent.rs`'s script helpers. `windows-agent-command-completion` runs `agent-command-completion` on Windows. Enter on a list takes the highlighted command (the agent-zones spec's "Enter takes the command"), so a case that sends a command it typed in full (` /compact`) closes the list with escape first.
34. **The dump parser's completion rows** are `Dump::completion_rows`: the lines after `--- completion` up to the next `--- ` line, in the self-test.

## The user's decisions

On 2026-11-26, after the stack under this change was built: while an agent is stopped its zone hides the labels and commands, the server dropping both lists and the continued agent filling them again, which the user chose over keeping the last known ones on view (decision 12). And every option command can be called as `/brindle:NAME`, the user's own proposal, so that an agent's command of the same name doesn't put Brindle's out of reach (decision 6).

None is open; what the refresh of 2026-11-26 decided is decision 13's flags, for review. The user settled each on 2026-11-25, one at a time: options show as small labels under the prompt input, with palette actions, on the user's condition that slash commands change them too, which the option commands of decision 6 do.

Also decided on 2026-11-25: every mode change leaves a notice in the conversation (decision 5); whitespace before a slash command makes no difference (decision 6).

Starting values per profile are left out of this change, as the proposal-only `agents-option-defaults`.

Completion matching is fuzzy, with names that start with the typed word first, for commands and for an option's choices (decision 6).

The unstable `models` list is read as one more option, which the user chose over the recommended skip, for broad coverage (decision 3).

## Risks / Trade-offs

- Options changed through pointer buttons and palette actions only → no tab stop is added, at the cost of one more step for a keyboard user than a focusable selector would take.
- A set shows no change until the agent answers → the value on screen is never a guess; a slow agent makes the control feel slow.
- An option command is named from what the agent calls the option, so the same setting is `/effort` with one agent and `/reasoning-effort` with another → the list shows the names as the user types `/`, and the labels under the input use the same words.
- An agent's own command takes the short name of Brindle's option command → the list shows Brindle's under `brindle:`, which always runs it, and the label and the palette still set that option.
- `brindle:` is taken from agents' command names → no captured agent uses it, and a command left out is said in the notice that counts them.
- Fuzzy matching can list a command the user didn't mean → prefix matches come first, and enter takes only the highlighted one.
- `OptionKey::Modes` and `OptionKey::Models` put v1 ideas in the version-free forms → they are opaque outside `acp/v1/`, and the architecture test says so.
- The model list is unstable ACP, read by hand → it is used only when the agent offers no `model` option, lives in `acp/v1/` alone, is pinned by a capture, and a shape that stops decoding only removes that one option.

## As built

- **`session/set_model` (task 1.5, not done):** the capture of claude-code-acp 0.16.2 taking it needs a Claude Code session started from a script, which can't be run from inside the Claude Code session that built this, so the task stays open. The adapter's source (`ClaudeAcpAgent.unstable_setSessionModel`) answers `{}` after setting the model, as Codex 2.12 and Antigravity 1.5.16 are captured doing, so the model list is read and set as decisions 3 and 4 say; the encoding is pinned by those two captures (`acp/v1/encode.rs`'s `the_captured_sets_are_written_as_brindle_writes_them`). A capture is still owed before release.
- **The fake ACP agent** needed no new step (task 3.3): `respond`, `respond-error`, `respond-to`, `send` and `wait-file` write every case.

## Migration Plan

Nothing to migrate. A session checkpointed before the change has no commands or options until its agent next sends them, which most do only at the start; such a session shows neither until it is started anew.
