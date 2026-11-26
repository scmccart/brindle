# Tasks

Before starting, check that the changes this one builds on are implemented and archived (the user's decisions are settled; design.md lists them):
- `agents-profiles` (and through it `agents-sessions`): the reducer, `brindle-transcript`, client protocol 4, `src/agent/model.rs`, `brindle-ui`'s `PromptInput`, the dump's agent lines, the fake ACP agent.
- `agents-tool-content`: `acp/forms.rs`, `acp/dialect.rs`, `acp/v1/{decode, encode}.rs`, `Request`, and `acp_versions_stay_in_their_modules`.
- Client protocol 4 is still unreleased. If a release has shipped it, stop and raise design decision 2.
- Refreshed on 2026-11-26 for the built stack (`agents-session-recovery`, `agents-login`, `agents-session-title`, `agents-zone-attention`, `agents-zone-theme-scope`, `agents-log-paging`): design decision 12 holds what they change, and the tasks below marked "(decision 12)" carry it. The function and file names in the other tasks are from before those changes; check each against the code.

## 1. Shared contracts

- [ ] 1.1 Add to `brindle-transcript`: `Command`, `SessionOption`, `OptionCategory`, `OptionKind`, `Choice`; `Event::Commands` and `Event::Options`; `Transcript`'s `commands` and `options` with their accessors; `replay` sending each when not empty; `MAX_COMMANDS`, `MAX_OPTIONS`, `MAX_CHOICES`; verify with unit tests that each form round-trips, that both events replace, `fold(replay(t)) == t` with both lists, `page` emitting both where `replay` does and the page laws holding with them (decision 12), the new forms in `fuzz.rs`, and that every existing `tests/fixtures/v4/` file still passes unedited.
- [ ] 1.2 Write the new forms' encodings as new files in `transcript/tests/fixtures/v4/` (commands with and without a hint; options with flat and grouped choices, a toggle, each category and `other`); verify the fixture tests write them once and read them back, and that no existing fixture changed.
- [ ] 1.3 Add `AGENT_SET_OPTION` (client message 24) and `OptionValue` to `brindle-remote-protocol`, refused by name below protocol 4 as the other agent messages are; verify with transcript fixtures for both value kinds, decoding errors for a missing field and a wrong type, and the message in the protocol's fuzzing.
- [ ] 1.4 Add to `acp/forms.rs`: `OfferedOption`, `OptionKey`, `Offered` (what `session/new` and each update give), `Request::SetOption`; add `Dialect`'s methods for them; add to `acp_versions_stay_in_their_modules` that `OptionKey::Modes` and `OptionKey::Models` are named only in `acp/forms.rs` and `acp/v1/`; verify `cargo test -p brindle --test architecture` passes.
- [ ] 1.5 Capture claude-code-acp 0.16.2 (the agent whose model can be set no other way) answering `session/set_model { sessionId, modelId }` for a model it lists and for one it doesn't, and add the log to `remote-agent/tests/fixtures/acp/`; write down in design.md's "As built" the method, its params and its answers. If the adapter refuses the method, leave the model list unread (tasks 2.1 and 2.2 lose their model list parts) and say so there.

## 2. Pure code in the server

- [ ] 2.1 Decode in `acp/v1/decode.rs`: `available_commands_update`; `configOptions` from the answers to `session/new`, `session/load` and `session/resume` (one function for the three, decision 12), `config_option_update` and `session/set_config_option`'s answer, item by item (`select` flat and grouped, `boolean`, another `type` left out, an unreadable item counted); `modes` and `current_mode_update` as the one mode option when no config option has category `mode`; the unstable `models` list, read by hand, as the one model option when no config option has category `model`; verify with unit tests over every captured line of each kind, each shape, both-offered for modes and for models, a mode update for an agent with a mode config option being ignored, and a model list of the wrong shape being left out.
- [ ] 2.2 Encode in `acp/v1/encode.rs`: `session/set_config_option` for a choice and for a toggle (`type: "boolean"`), `session/set_mode` for `OptionKey::Modes`, `session/set_model` for `OptionKey::Models` as task 1.5 recorded it, and `clientCapabilities.session.configOptions.boolean: {}` in `initialize`; verify with unit tests pinning each line, and that the capture replays still pass with the new `initialize`.
- [ ] 2.3 Extend the reducer: keep commands and offered options with their keys; apply the bounds and the typeable-name rule with their notices; emit `Commands` and `Options` only when a list differs; the set (`SetRefusal::{NotReady, UnknownOption, NotOffered, Busy}`, the current value as a no-op, the pending mark, the agent's answer as the new list, `session/set_mode`'s empty answer setting the value, the agent's error as a notice); the mode notice; verify with unit tests for each, including the first list adding no notice and a model change adding none.
- [ ] 2.3a Extend the reducer for an agent that stops and one that continues (decision 12): every way an agent stops empties both lists (an event each only when it held anything) and drops the keys and the pending mark; the options come from whichever start the run made; commands, options and mode updates during a load's replay apply, muted or shown, without being counted and without a mode notice; the mode values last in effect are kept across a stop, and a run's first list adds a notice only for a mode that differs from them; verify with unit tests for each stop, a resume and a load with options, a muted replay holding two mode updates, a set in flight at an exit, and a continue in the same and in another mode.
- [ ] 2.4 Extend `acp/checkpoint.rs` (format 2, amended) with the keys, the pending mark (cleared on adoption) and the kept mode values as state fields of every record, the two lists travelling as events in the base and the segments; verify a session with options from modes round-trips as a base and as a base with segments, and a format 1 and a format 2 checkpoint from before this change decode with both lists empty.
- [ ] 2.5 Extend the capture replay test to assert each agent's commands and options (Claude's four options with `fast` as `other:speed`, Copilot's `allow_all` as `other:permissions`, claude-code-acp's modes as one mode option and its model list as one model option, and no second model option for the agents that send both).

## 3. Server roles

- [ ] 3.1 Handle `AGENT_SET_OPTION` in the core: gather, call the reducer, write the agent's line, answer `OK` or `ERROR` with the refusal's text; verify with real-role tests through the fake ACP agent: a set reaches the agent and its answer reaches two attached clients, each refusal's text, the agent's error becoming a notice, a set during a turn.
- [ ] 3.2 Add the restart cases: both lists replay after a core restart and an upgrade; a set in flight at a restart leaves the old value and accepts a new set afterwards.
- [ ] 3.2a Add the continue cases (decision 12): an agent's exit empties both lists for two attached clients; a Continue by resume and one by load fill them from the new agent's answer; a continue in another mode adds its notice; an attach to a conversation longer than a page gets both lists.
- [ ] 3.3 Give the fake ACP agent what the cases need (answering `session/new` with options and modes, `session/set_config_option`, `session/set_mode`, sending the two updates) through its existing script steps, adding a step only if a case can't be written without one.

## 4. Brindle's model

- [ ] 4.1 Extend `src/agent/shared.rs` and `transcript.rs` for the two events; add the client's `set_option` request with the protocol gate naming the endpoint; verify with unit tests folding each event, a replay holding both lists, and the gate.
- [ ] 4.2 Add to `src/agent/model.rs` the pure `completion`, `accept`, `completion_key` and the `dismissed` flag; and `command_text`; verify with unit tests for: spaces, tabs and blank lines before the `/` giving the same result as none, indented text that isn't a command left as typed, `/` alone, a prefix, a fuzzy match ordered after prefix matches, no match, the cursor past the first word, a slash later in the text, a second line starting with `/`, each key's action, wrapping, dismissal lasting until the text stops being a command word, and IME composition leaving the list alone.
- [ ] 4.2a Add `option_commands`, the second stage of `completion` and `local_command` (design decision 6); verify with unit tests for: each naming rule (case, runs of other characters, an empty name, two options with one name, a name the agent's commands hold), choices matched by name and by value without regard to case, a choice with a space in its name, the highlight starting on the current choice, a toggle's `on`, `off` and bare form, `NoSuchChoice`, text with more lines after the command's line staying a prompt, an option command after leading whitespace being local, and a bare option command giving `Choose`.
- [ ] 4.3 Add `option_items`, `next_mode` and the options palette's rows; verify with unit tests for the order, a current value not among the choices, a toggle's label, grouped choices, `next_mode` wrapping and its `None` cases.
- [ ] 4.4 Extend the dump (`describe`, `wait_text`) with the `--- commands`, `--- option` and `--- completion` lines; verify with unit tests for each form and escaping.

## 5. brindle-ui

- [ ] 5.1 Add `PromptInput::set_suggestions` and the list above the textarea (no focus; a press asks the owner); verify with component tests for the rows, the highlight, the press and the textarea keeping focus; add it to the gallery.
- [ ] 5.2 Add `PromptInput::set_footer` and the items under the textarea (no keyboard stop; a press asks the owner with the key); verify with component tests that `row_stops` is unchanged and a press reports its key; add it to the gallery.

## 6. The zone

- [ ] 6.1 Wire completion in `src/agent/view.rs`: recompute on every text or cursor change, take the list's keys before the prompt input's own, accept on a row press; verify in the e2e case of 7.1.
- [ ] 6.1a Run `local_command` on every submit before the prompt is sent: send the set and clear the input, open the choices, or keep the text with the reason beside it; a first prompt from `-p` and the queue's prompts skip it; verify in the e2e case of 7.1a.
- [ ] 6.2 Show the footer from `option_items`; a press opens the option's choices in the zone's palette or flips a toggle; send the set, and show a refusal on the notice line.
- [ ] 6.3 Add `agent_options` ("Agent Options…") and `agent_next_mode` ("Next Agent Mode") to `ACTIONS` with no default keys, handled only by `AgentView` and listed only when the session has options (a mode option with two or more choices for the second); verify with unit tests for the palette's listing in an agent zone with and without options and in a terminal zone.

## 7. End-to-end cases (fake ACP agent)

- [ ] 7.1 `agent-command-completion`: the list on `/`, narrowing, down and enter taking a command, escape closing it without stopping the turn, enter then sending `/name` as a prompt, and tab moving focus when no list shows.
- [ ] 7.1a `agent-option-command`: `/mo` lists `/mode` and `/model`; `/model so` and enter sets the model with no user entry and an empty input; `/mode ` lists every choice with the current one highlighted; `/model gpt` keeps the text with its reason; `/mode plan` during a turn sets at once and queues nothing; an agent command named like an option is sent to the agent; ` /mode plan` with a space first sets the mode, and ` /compact` with a space first reaches the agent as `/compact`.
- [ ] 7.2 `agent-option-set`: the options in the dump, `agent_options` choosing a model, the value changing only after the agent's answer (behind a `Gate`), and the footer following an update the agent sends on its own.
- [ ] 7.3 `agent-option-modes`: an agent with modes only shows one mode option; `agent_next_mode` steps and wraps it; the conversation gets its `Mode:` notices.
- [ ] 7.4 `agent-option-refused`: a second set while one is unanswered shows the notice line's reason; the agent's error shows as a notice in the conversation.
- [ ] 7.5 `agent-option-restore`: quit and relaunch, and the restored zone shows the same commands and options without the agent sending them again.
- [ ] 7.5a `agent-option-continue` (decision 12): the agent exits and the dump shows no `--- option` or `--- commands` line and no completion on `/`; Continue brings them back, with a `Mode:` notice when the continued agent starts in another mode.
- [ ] 7.6 Run the agent cases on Windows (one `windows-agent-*` case with the completion list) and in the archive jobs.

## 8. Docs and checks

- [ ] 8.1 Update `docs/code-map.md`, `README.md`'s Agent tabs section (commands, options, the two actions), and `CLAUDE.md`'s dump description and action notes.
- [ ] 8.2 Tell `agents-acp-v2` what it inherits: add to its proposal the v2 mapping of design decision 3.
- [ ] 8.3 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate agents-commands --strict`.
- [ ] 8.4 Manual checks for the user, with `scripts/run-throwaway.sh` and a real agent profile: a pointer press on a suggestion row and on each footer item; a real agent's `/compact`; switching a real agent's mode and model and seeing it take effect in the next turn; IME composition in the prompt input while the list shows.
