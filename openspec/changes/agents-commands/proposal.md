# Proposal

**Depends on:** `agents-profiles` (the agent zone and its prompt input, and through it `agents-sessions`' reducer, transcript and client protocol 4) and `agents-tool-content` (the `Dialect` seam that keeps each ACP version in its own module), implemented and archived. It doesn't touch `agents-markdown`, `agents-session-recovery`, `agents-login` or `agents-log-paging`.
**Next:** `agents-session-info` (the conversation's title and context usage, proposal-only), then `agents-plans` and `agents-prompt-context`, still to be proposed.

## Why

Every ACP agent we captured tells its client two things at the start of a session that Brindle throws away. It lists its slash commands (`/compact`, `/review`, `/context`, …), and it lists its session options: the model, the thinking level, and above all the mode, which decides whether the agent asks before it edits files or runs commands. In step 1 the terminal server ignores both updates. So a user must know an agent's commands by heart and type them blind, can't see which model or mode a conversation is in, and can't change either without leaving Brindle. An agent that starts in its "ask first" mode stays there, and one that starts in "accept edits" does too, unseen.

ACP v1 has two ways to offer options: session modes (`session/set_mode`), which v2 drops, and session config options (`session/set_config_option`), which v1 and v2 share. Every current agent we captured sends config options; one deprecated adapter sends only modes. Brindle keeps both ACP versions for good, so both ways are read, into one form of Brindle's own.

## What Changes

- **The server keeps the agent's commands and options** as session state beside the conversation: each command's name, description and input hint; each option's name, category (mode, model, thinking level, or the agent's own), and either its choices with the current one or an on/off value. Both are in every replay and survive a detach, a quit, a core restart and an upgrade.
- **Session modes become an option.** For a v1 agent that offers modes and no mode option, the server presents its modes as one option of category `mode` and sets it with `session/set_mode`, inside the v1 module. Brindle sees options only.
- **A client can set an option.** A new client message asks the server to set one option to one value; the server checks it against what the agent offered, asks the agent, and the agent's answer becomes the new options. Client protocol 4 is amended, as for the rest of the series.
- **A mode change is recorded in the conversation**, whoever made it, since it changes what the agent may do without asking.
- **Slash command completion.** Typing `/` at the start of the prompt input lists the agent's commands, filtered as the user types, with each command's description and input hint. Enter or tab takes the highlighted one; the command is then sent as an ordinary prompt, as ACP has it.
- **Options have slash commands too.** Brindle adds one command per option, named from the option (`/mode`, `/model`, `/fast-mode`), listed with the agent's own. After the command the list shows the option's choices; enter sets one. Brindle carries these out itself, so they are never sent to the agent, and an agent's own command of the same name takes precedence.
- **Options show under the prompt input** as their current values (`auto · opus · medium`), and each opens a list of its choices on a pointer press. From the keyboard, the new `agent_options` action opens the same choices in a palette, and `agent_next_mode` steps the mode option to its next choice.
- **Bounds on everything an agent sends:** 256 commands, 32 options, 256 choices an option, with names and descriptions cut as titles are.

Not in this change: per-profile starting values for options; remembering a choice across sessions; ACP's unstable model list (`session/set_model`); command input kinds other than free text; ACP v2's unstable skills; the conversation's title and context usage (`agents-session-info`); option controls as keyboard focus stops.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: new requirements for the agent's commands, its options, session modes as an option, setting an option, and recording a mode change.
- `agent-zones`: new requirements for slash command completion, the options shown under the prompt input, choosing an option, and the two actions.
- `ui-components`: the prompt input's suggestion list and footer items.
- `command-line`: the dump's lines for commands, options and the completion list, and driving them.

## Impact

- **`brindle-transcript`** (`transcript/`): `Command`, `SessionOption`, `OptionCategory`, `OptionKind`, `Choice`; `Event::Commands` and `Event::Options`; the transcript's state and replay; new `tests/fixtures/v4/` files (none edited).
- **`brindle-remote-protocol`** (`remote-protocol/`): a new client message, `AGENT_SET_OPTION`, with its fixtures and fuzzing. Its number is the next free one when the stack is assembled.
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): the forms and `acp/v1/`'s decoding of commands, config options, modes and their updates; `session/set_config_option` and `session/set_mode`; the boolean option capability in `initialize`; the reducer's option handling. The checkpoint's format 1 is amended (unreleased).
- **`brindle`** (`src/agent/`): the chat model's commands, options and completion; the view's list and footer; two actions in `src/actions.rs`; the options palette.
- **`brindle-ui`** (`ui/`): `PromptInput` gains a suggestion list and footer items. Its dependency list is unchanged.
- **Tests:** the existing captures already hold every agent's commands and options; real-role tests through the fake ACP agent; new `agent-command-*` and `agent-option-*` e2e cases.
- **Docs:** `docs/code-map.md`, `README.md`'s Agent tabs section, `CLAUDE.md`'s dump and action notes.
