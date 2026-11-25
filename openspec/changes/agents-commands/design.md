# Design

## Context

See proposal.md for why. Facts this design rests on, from the code on `feat/agents-profiles` (2026-11-25), `agents-tool-content`'s design, the pinned schema crate's sources and the captured agent traffic:

- **The reducer ignores both updates today.** `wire.rs` maps `available_commands_update`, `config_option_update` and `current_mode_update` to `Update::Ignored`, and reads only `sessionId` from `session/new`'s answer. `initialize` sends `clientCapabilities: {}`.
- **What real ACP v1 agents send** (`remote-agent/tests/fixtures/acp/`):

  | Agent | Commands | `configOptions` in `session/new` | `modes` | Updates |
  | --- | --- | --- | --- | --- |
  | claude-agent-acp 0.112 | 4 (`debug`, `compact`, `context`, …), some with an input hint | `model` (14 choices), `effort` (thought level), `fast` (speed), `mode` (5) | yes | `config_option_update` |
  | codex-acp 2.12 | 2 (`review`, `compact`) | `model`, `reasoning_effort`, `mode`, `collaboration_mode` | yes | `config_option_update` |
  | copilot 1.0.115 | 4 | `mode`, `model`, `allow_all` (category `permissions`) | yes | `config_option_update` |
  | antigravity 1.5.16 | 1 (`compact`) | `model`, `thought_level`, `mode` | yes | `config_option_update` |
  | claude-code-acp 0.16.2 (deprecated) | 4 | none | yes | none |

  Every option captured is a `select` with a flat choice list. Every agent with config options has one of category `mode` that mirrors its `modes`. Four agents also send an unstable `models` list, which the pinned crate has no type for.
- **ACP v1** (schema crate 1.22.0): a command is `{ name, description, input? }` whose only input kind is free text with a `hint`; commands are invoked by including `/name` in a prompt. A config option is `{ id, name, description?, category?, type }` with `type: "select"` (`currentValue`, `options` flat or in named groups) or `type: "boolean"` (`currentValue`); categories are `mode`, `model`, `model_config`, `thought_level` and any other string. `session/set_config_option { sessionId, configId, value }` (with `type: "boolean"` for a toggle) answers with the full option list. An agent may send boolean options only to a client that advertised `clientCapabilities.session.configOptions.boolean`. Session modes are `modes { currentModeId, availableModes }`, `session/set_mode { sessionId, modeId }` and `current_mode_update`.
- **ACP v2** (Preview): config options and `session/set_config_option` are the same shape; session modes are gone; a command list entry has a `type` (`command`, an unstable `skill`, or unknown, which a client "MUST NOT display as a slash command"), and its input a `type` (`text` or unknown).
- **`agents-tool-content`** gives `acp/forms.rs`, `acp/dialect.rs` with `Dialect { V1 }`, `acp/v1/{decode, encode}.rs`, `Request` for the lines the reducer asks to be written, and the rule that only a version's module names its methods and fields.
- **Brindle** (`agents-profiles`): `brindle-ui`'s `PromptInput` holds a `TextArea`, a toolbar and the queue row, with pure helpers (`enter_submits`, `escape_stops`); `ui/src/filter.rs` has `fuzzy_match` and `filter`; `src/picker.rs`'s `Palette` draws lists and groups of `PaletteRow`s. Agent actions (`agent_cancel`, `toggle_agent_thoughts`, …) are handled only by `AgentView`. Tab and shift-tab move among the zone's stops ("Keyboard path through an agent zone").

## Goals / Non-Goals

**Goals:**
- A user sees an agent's commands as they type `/`, and sees and changes the mode, model and other options of a conversation without leaving it.
- Options are Brindle's own form: session modes, config options and whatever v2 brings fill it inside their version's module, and Brindle never learns which.
- The server keeps commands and options, so a restored or reattached zone has them without asking the agent again.
- Nothing is set that the agent didn't offer, and a change of mode is on the record.
- No new keyboard stop in the zone and no change to a requirement another stacked change modifies.

**Non-Goals:** starting values for options in a profile, and remembering a choice across sessions; ACP's unstable `models` list and `session/set_model`; command inputs other than free text, and v2's unstable skills; running a command from the command palette; completing anything but a command's name (arguments, file paths, `@` mentions); option controls as tab stops; the conversation's title and context usage (`agents-session-info`). No config key and no state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Command`, `SessionOption`, `OptionCategory`, `OptionKind`, `Choice`; `Event::Commands`, `Event::Options`; the transcript's state and replay | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| `AGENT_SET_OPTION` and `OptionValue` | wire | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| The forms: `Offered` (commands, options with their keys), `OptionKey`, `Request::SetOption` | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| ACP v1: commands, config options, modes and their updates; `session/set_config_option`, `session/set_mode`; the boolean capability | pure | `acp/v1/{decode, encode}.rs` |
| Keeping, bounding and setting options; the mode notice | pure | `acp/reduce.rs` |
| The set request, its checks and reply | shell (loop handler) | `remote-agent/src/server/core/` |
| `completion`, `option_items`, `next_mode`, the options palette's rows, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| The suggestion list, the footer, the palette, the two actions | shell (entity) | `src/agent/view.rs`, `src/agent/mod.rs`, `src/actions.rs` |
| `PromptInput`'s suggestions and footer items | components | `brindle-ui`, `ui/src/components/prompt_input.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`).

## Decisions

### 1. Brindle's own forms (`types R1`, `R2`, `R7`)

```rust
pub struct Command { pub name: String, pub description: String, pub hint: Option<String> }

pub struct SessionOption {
    pub id: String,                       // Brindle's id for it within the session, which a set names
    pub name: String,
    pub description: Option<String>,
    pub category: OptionCategory,
    pub kind: OptionKind,
}
pub enum OptionCategory { Mode, Model, ModelConfig, ThoughtLevel, Other { name: String } }
pub enum OptionKind {
    Select { current: String, choices: Vec<Choice> },
    Toggle { on: bool },
}
pub struct Choice { pub value: String, pub name: String, pub description: Option<String>, pub group: Option<String> }
```

- **What the types rule out.** An option is a list with a current value or a toggle, never both. Grouped choices are one list whose items name their group, so "grouped" and "flat" aren't two shapes to handle. `OptionCategory::Other { name }` keeps a category Brindle doesn't know (`speed`, `permissions`, `collaboration_mode`), as `StopReason::Other` does. No hint and no description are `None`.
- **A `Select` whose current value isn't among its choices** is kept as sent: the zone shows the value itself where it would show the choice's name. Agents do this while a model list loads.
- **Commands and options are session state, not log entries**, like the queue slot and the agent's state: the transcript holds the latest list of each, whole.
- **Ids.** `SessionOption::id` is the agent's config option id for a config option. For the option made from session modes (decision 3) it is `mode`, or `mode` followed by `'` when a config option already has that id, so ids are unique within the list.

### 2. Events, the fold and replay (`brindle-transcript`; `process R4`–`R6`)

```rust
Event::Commands { commands: Vec<Command> }      // the whole list, replacing the last
Event::Options  { options: Vec<SessionOption> } // the whole list, replacing the last
```

- Both replace, so no fold error is possible beyond decoding. `Transcript` gains `commands` and `options`, each starting empty; `replay()` sends each after the agent's state when it isn't empty; `fold(replay(t)) == t` gains them.
- The core sends an event only when the list differs from the one it holds (agents repeat them).
- **Bounds** (`railway R5`), applied by the reducer before the event: at most `MAX_COMMANDS` = 256 commands and `MAX_OPTIONS` = 32 options with `MAX_CHOICES` = 256 choices each; names, values, ids, hints and group names bounded as ids are, descriptions at 1 KiB; every text cleaned. A list is also cut so its event stays under `TEXT_CHUNK` (64 KiB). What is dropped is said once in a notice ("the agent offered N commands; the first 256 are kept").
- **A command's name must be typeable:** a name that is empty, or holds whitespace, a control character or `/`, is left out.
- **Versions.** Client protocol 4 is unreleased and amended, as `agents-tool-content`'s decision 8 says: two event kinds, with new `tests/fixtures/v4/` files and none edited. The checkpoint keeps format 1, with the two lists and the option keys (decision 3) in its state; a checkpoint from before this change decodes with both empty.

### 3. Options come from the dialect, each with its key (`acp/forms.rs`, `acp/v1/`)

```rust
pub struct OfferedOption { pub option: SessionOption, pub key: OptionKey }
pub enum OptionKey { Config { id: String }, Modes }
Request::SetOption { session: &str, key: &OptionKey, value: OptionValue }
```

- **Config options** (v1 and v2): `session/new`'s `configOptions`, `config_option_update` and the answer to `session/set_config_option` each give the whole list, decoded item by item from the raw JSON so one unreadable option loses only itself. Each gets `OptionKey::Config { id }`. `select` with flat or grouped choices and `boolean` are read; another `type` is left out.
- **Session modes** (v1 only): when `session/new` gives `modes` and no config option has category `mode`, the v1 module makes one option from them (name "Mode", category `Mode`, a `Select` of the modes) with `OptionKey::Modes`; `current_mode_update` changes its current value. When a config option has category `mode`, the `modes` and `current_mode_update` are ignored: the agent mirrors them in its config options, and one control is enough.
- **The key is opaque to the reducer** (`ports R4`, `agents-tool-content`'s rule): it stores each offered option's key beside the option and hands it back in `Request::SetOption`; only `acp/v1/encode.rs` reads it, writing `session/set_config_option` (with `type: "boolean"` for a toggle) or `session/set_mode`. `OptionKey::Modes` exists in `forms.rs` because a key must be storable; nothing outside `acp/v1/` constructs or matches it, which `acp_versions_stay_in_their_modules` gains a line for.
- **The boolean capability:** the v1 module's `initialize` advertises `clientCapabilities.session.configOptions.boolean: {}`, since the zone can show a toggle.
- **The unstable `models` list** is not read: it is outside the pinned crate, and every agent that sends it also sends a `model` config option, except the deprecated claude-code-acp, whose model then can't be changed from Brindle while its mode can.
- **Commands:** `available_commands_update` gives the whole list. v1's only input kind gives the hint.

**What ACP v2 maps to** (for `agents-acp-v2`): config options and `session/set_config_option` as above; no modes, so `OptionKey::Modes` is never produced; a command entry of `type: "command"` is a `Command`, any other type is left out; an input of `type: "text"` gives the hint, any other gives none.

### 4. Setting an option (`acp/reduce.rs`, the core; `railway R1`, `R6`)

- **The message.** `AGENT_SET_OPTION { session, option, value }` with `OptionValue::{ Choice { value: String }, Toggle { on: bool } }`, a new client message of protocol 4 (amended; its number is the next free one when the stack is assembled, after `AGENT_RESTART`, `AGENT_LOGIN` and `AGENT_FETCH`). Transcript tests pin its encoding, and it joins the protocol's fuzzing.
- **Checked before anything is sent** (the outcome is `Result<(), SetRefusal>`, answered `OK` or `ERROR` with the reason): `NotReady` (the agent isn't ready), `UnknownOption { id }`, `NotOffered { id, value }` (a choice the option doesn't list, or the wrong kind of value), `Busy` (an earlier set for this session hasn't been answered). A set naming the current value is `OK` and sends nothing.
- **Any time the agent is ready**, during a turn too: ACP lets an agent take a change mid-turn or refuse it.
- **The agent's answer is the truth.** Its result carries the whole list, which becomes `Event::Options` if it differs. For `session/set_mode`, whose result is empty, the reducer sets the mode option's current value itself. Brindle shows nothing as changed until the event arrives.
- **The agent refuses** (a JSON-RPC error): the options stay, and the log gets a notice, "the agent didn't change NAME to VALUE: MESSAGE". The client's `OK` meant "asked", so the failure is shown where the session's other failures are, in the conversation.
- **The agent never answers:** `Busy` holds until the agent's exit or the session's end clears it, as for any pending request.

### 5. A mode change is on the record

When an option of category `Mode` gets a different current value, the reducer adds an info notice to the log, "Mode: NAME" (the choice's name, else its value), whoever caused it: this client, another, or the agent itself. The first list a session receives adds none. Other categories add none: a model or thinking level changes nothing about what the agent may do unasked, and some agents adjust them on their own at the start of a session.

### 6. The chat model (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **Whitespace before the slash makes no difference** (the user, 2026-11-25: an accidental space mustn't change behaviour). `command_text(text) -> Option<&str>` gives the text from its first character that isn't whitespace when that character is `/`, else `None`. Completion, `local_command` and what is sent all start from it: an option command after a space is carried out, and an agent's command after a space is sent without the space. A prompt that doesn't begin with `/` after its whitespace is sent exactly as typed, indentation included.
- **Completion.** `completion(text, cursor, commands) -> Option<Completion>`: `Some` when `command_text` is `Some`, the cursor is inside its first word, and at least one command matches the word after `/` with `ui`'s `fuzzy_match`; names that start with the word come first, then the rest in the agent's order. `Completion { range, items, highlighted }`; `accept(completion) -> (String, usize)` replaces the range with `/NAME ` and gives the new cursor. The model keeps `dismissed: bool`, set by escape and cleared when the text stops being a command word, so a dismissed list doesn't come back on the next key.
- **Option commands** (the user's condition for the labels, 2026-11-25: options must be changeable by slash command too). `option_commands(options, agent_commands) -> Vec<OptionCommand>` gives one command of Brindle's own per option: its name is the option's name in lower case with each run of characters other than ASCII letters and digits as one `-`, trimmed (`mode`, `model`, `reasoning-effort`, `fast-mode`); an empty name is skipped, a second option with the same name gets `-2`, and a name the agent's own commands hold is dropped, so the agent's command is the one offered. Its description is the option's name and current value, its hint `<choice>` or `on|off`. They are listed with the agent's commands, after them.
- **The second stage.** When the text is an option command, a space and anything after it on one line, `completion` returns that option's choices matching the rest of the line (by name or value, case-insensitive, fuzzy with prefix matches first), with the highlight starting on the current choice when nothing is typed. A toggle offers `on` and `off`.
- **`local_command(text, options, agent_commands) -> Option<Local>`** (pure) decides what a submit does before anything is sent: `None` (send the text as a prompt), `Set { id, value }`, `Choose { id }` (an option command alone: show its choices; a toggle alone flips), or `NoSuchChoice { option, text }`. Only text whose `command_text` is one line and names an option command is local. `Set` clears the input and sends `AGENT_SET_OPTION`, during a turn too, and never touches the queue; `NoSuchChoice` keeps the text and shows its reason beside the input, where a refused prompt's reason shows. A local command is never a user message: only the server's mode notice records it.
- **What stays text:** a first prompt from `brindle agent -p` and a queued or held prompt are sent as they are, option command or not; so is text with more lines after the command's line. There is no way to send an option command's own text to the agent as a prompt, unless the agent offers a command of that name, in which case the agent's is the one that runs.
- **Keys while the list shows** (`completion_key(key) -> CompletionAction`, pure): up and down move the highlight, wrapping; enter and tab accept (a command's name in the first stage; in the second, tab writes the choice's name into the text and enter sets it); escape dismisses. They are taken before the prompt input's own handling, so enter doesn't send and tab doesn't move focus. With no list, every key does what it did.
- **Option items.** `option_items(options) -> Vec<OptionItem>`: the options in the order mode, model, thinking level, model config, then others in the agent's order, each `{ id, label }` where the label is the current choice's name, or the option's name followed by `on` or `off`.
- **`next_mode(options) -> Option<(String, OptionValue)>`:** the first option of category `Mode` that is a `Select`, and the choice after its current one, wrapping; `None` with no such option or fewer than two choices.
- **The options palette's rows:** one row per option ("Model: opus", its description as the detail); choosing a `Select` shows its choices (grouped under their group names, the current one marked), choosing a `Toggle` flips it.

### 7. The zone and `brindle-ui` (`ports R2`)

- **`PromptInput`** gains `set_suggestions(Option<Suggestions>)` (plain rows: a label, a hint, a description, and the highlighted index), drawn as a list above the textarea that takes no focus, a press on a row asking the owner to accept it; and `set_footer(Vec<FooterItem>)` (a label and a key each), drawn under the textarea as small ghost buttons separated by `·`, a press asking the owner with the item's key. Neither is a keyboard stop, so `row_stops` and "Keyboard path through an agent zone" are unchanged.
- **A press on a footer item** opens that option's choices in the zone's palette (a `Select`), or flips it (a `Toggle`).
- **Actions:** `agent_options` ("Agent Options…") opens the options palette; `agent_next_mode` ("Next Agent Mode") sends `next_mode`'s set. Neither has a default key; both are handled only by `AgentView`, so the palette lists them only in an agent zone, and only when the session has options (`agent_next_mode`: a mode option with two or more choices).
- **A set that is refused before it is sent** (`SetRefusal`) shows its reason on the window's notice line, as a refused action does elsewhere.
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

- `--- commands` and the `--- option` lines come after the `--- agent` line, whenever the lists aren't empty. `--- completion` and its rows (as palette rows print) come after the `--- prompt` line while the list shows.
- `wait_text` holds them, so `--wait-for 'current="plan"'` works. `--key` drives the list (`down`, `enter`, `tab`, `escape`), and `--action agent_options`, `agent_next_mode` and the palette's own actions drive the options.

### 9. Failures (`railway R6`, `R8`)

- An option or command that can't be read, or past a bound: left out, one notice.
- A set the server refuses before sending: `ERROR` with the reason, shown on the notice line; nothing changes.
- A set the agent refuses: a notice in the conversation; the options stay.
- A set to an agent on an endpoint whose server predates this change: the client refuses it before sending, naming the endpoint, as `agent_start` does for protocol 3.

### 10. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Commands, options and their keys are in the session's checkpoint. A core restart adopts them; a set in flight at the restart is lost with its `Busy` mark, and the option shows its old value until the user sets it again or the agent reports it. A Brindle crash loses nothing: both lists replay on attach. An upgrade's next core reads the same format 1.

### 11. Tests

- **Unit:** the fold and replay over both events; `v1::decode` over every captured `session/new`, `available_commands_update` and `config_option_update`, grouped choices, a boolean, an unknown `type`, an unreadable option, modes with and without a mode option, `current_mode_update`; `v1::encode` for each key and value; the reducer's bounds, equal lists emitting nothing, every `SetRefusal`, the agent's refusal, the mode notice (not for the first list, not for other categories); the model's completion, keys, items, `next_mode` and dump lines.
- **Captures:** the replay test asserts each agent's commands and options.
- **Protocol:** fixtures and fuzzing for `AGENT_SET_OPTION`.
- **Real roles** (the fake ACP agent): a set reaches the agent and its answer reaches every attached client; a refusal; a restart keeps both lists.
- **E2E:** `agent-command-completion`, `agent-option-set`, `agent-option-modes`, `agent-option-refused`, `agent-option-restore`.
- **Manual:** pointer presses on a suggestion row and a footer item; a real agent's `/compact` and a mode switch.

## Open decisions

Each has a recommendation, which the text above follows. They are the user's to settle before implementation.

Decided on 2026-11-25: options show as small labels under the prompt input, with palette actions, on the user's condition that slash commands change them too, which the option commands of decision 6 do.

Also decided on 2026-11-25: every mode change leaves a notice in the conversation (decision 5); whitespace before a slash command makes no difference (decision 6).

Still open:

3. **Starting values per profile** (for example always start one agent in its plan mode): left out of this change (recommended, as a follow-up), or included.
4. **Completion matching:** fuzzy, with names that start with the typed word first (recommended), or prefix only.
5. **The unstable `models` list:** not read (recommended), so the deprecated claude-code-acp adapter's model can't be changed from Brindle.

## Risks / Trade-offs

- Options changed through pointer buttons and palette actions only → no tab stop is added, at the cost of one more step for a keyboard user than a focusable selector would take.
- A set shows no change until the agent answers → the value on screen is never a guess; a slow agent makes the control feel slow.
- An option command is named from what the agent calls the option, so the same setting is `/effort` with one agent and `/reasoning-effort` with another → the list shows the names as the user types `/`, and the labels under the input use the same words.
- An agent's own command hides Brindle's option command of the same name → the label and the palette still set that option.
- Fuzzy matching can list a command the user didn't mean → prefix matches come first, and enter takes only the highlighted one.
- `OptionKey::Modes` puts a v1 idea in the version-free forms → it is opaque outside `acp/v1/`, and the architecture test says so.

## Migration Plan

Nothing to migrate. A session checkpointed before the change has no commands or options until its agent next sends them, which most do only at the start; such a session shows neither until it is started anew.
