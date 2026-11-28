use std::collections::BTreeMap;

use brindle_remote_protocol::client::OptionValue;
use brindle_remote_protocol::serde_json::{self, Value};
use brindle_transcript::{
    Event, MAX_CHOICES, MAX_COMMANDS, MAX_OPTIONS, NoticeLevel, OptionCategory, OptionChoice,
    OptionControl, SessionOption, SlashCommand, TEXT_CHUNK, clean,
};

use crate::acp::envelope::RpcError;
use crate::acp::forms::{Offered, OfferedOption, OptionKey};

use super::lifecycle::Phase;
use super::text::cut_at;
use super::{Moment, Refused, Step};

/// The longest option id or choice value kept (`agents-commands` flag 6):
/// a set names it, so a longer one leaves its item out rather than be cut.
pub const MAX_OPTION_ID: usize = 1024;

/// Option, choice, category and group names, command names and hints are
/// at most this many bytes (as a title is); a longer command name leaves
/// its command out.
pub const MAX_OPTION_NAME: usize = 256;

/// Descriptions are cut at this many bytes.
pub const MAX_OPTION_DESCRIPTION: usize = 1024;

/// Room kept in a [`TEXT_CHUNK`] for an event's own fields around its list.
const LIST_EVENT_ROOM: usize = 64;

/// What a mode option's value is compared with for its notice (decision
/// 5 and 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compare {
    /// Nothing: a load's replay, or a start not yet answered.
    None,
    /// The held option's value, else the one last in effect.
    Held,
    /// The value last in effect: a run's first list.
    Kept,
}

/// Whether a command's name can be typed as `/NAME` and isn't Brindle's
/// own (`brindle:`).
pub(super) fn typeable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_OPTION_NAME
        && !name.starts_with("brindle:")
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '/')
}

/// A text cleaned and cut to `limit` bytes.
pub(super) fn trimmed(text: &str, limit: usize) -> String {
    cut_at(clean(text), limit).0
}

/// How many bytes an encoding took; one that failed takes them all.
fn encoded_len(encoded: Result<Vec<u8>, serde_json::Error>) -> usize {
    encoded.map_or(usize::MAX, |bytes| bytes.len())
}

/// The commands kept of those offered (decision 2): typeable ones, at most
/// [`MAX_COMMANDS`], their texts cleaned and cut, within one event.
fn bounded_commands(offered: Vec<SlashCommand>) -> Vec<SlashCommand> {
    let mut commands = Vec::new();
    let mut len = LIST_EVENT_ROOM;
    for command in offered {
        if commands.len() == MAX_COMMANDS {
            break;
        }
        if !typeable(&command.name) {
            continue;
        }
        let command = SlashCommand {
            name: command.name,
            description: trimmed(&command.description, MAX_OPTION_DESCRIPTION),
            hint: command.hint.map(|hint| trimmed(&hint, MAX_OPTION_NAME)),
        };
        let grown = len.saturating_add(encoded_len(serde_json::to_vec(&command)).saturating_add(1));
        if grown > TEXT_CHUNK {
            continue;
        }
        len = grown;
        commands.push(command);
    }
    commands
}

/// Whether an option id or choice value can be kept whole.
fn settable(value: &str) -> bool {
    value.len() <= MAX_OPTION_ID && !value.chars().any(char::is_control)
}

/// The options kept of those offered and their keys by id (decision 2):
/// at most [`MAX_OPTIONS`] with unique, settable ids, each with at most
/// [`MAX_CHOICES`] settable choices, their texts cleaned and cut, within
/// one event.
fn bounded_options(offered: Vec<OfferedOption>) -> (Vec<SessionOption>, BTreeMap<String, OptionKey>) {
    let mut options: Vec<SessionOption> = Vec::new();
    let mut keys = BTreeMap::new();
    let mut len = LIST_EVENT_ROOM;
    for OfferedOption { option, key } in offered {
        if options.len() == MAX_OPTIONS {
            break;
        }
        if !settable(&option.id) || keys.contains_key(&option.id) {
            continue;
        }
        let kind = match option.kind {
            OptionControl::Select { current, choices } => {
                if !settable(&current) {
                    continue;
                }
                let choices = choices
                    .into_iter()
                    .filter(|choice| settable(&choice.value))
                    .take(MAX_CHOICES)
                    .map(|choice| OptionChoice {
                        value: choice.value,
                        name: trimmed(&choice.name, MAX_OPTION_NAME),
                        description: choice
                            .description
                            .map(|text| trimmed(&text, MAX_OPTION_DESCRIPTION)),
                        group: choice.group.map(|group| trimmed(&group, MAX_OPTION_NAME)),
                    })
                    .collect();
                OptionControl::Select { current, choices }
            }
            OptionControl::Toggle { on } => OptionControl::Toggle { on },
        };
        let category = match option.category {
            OptionCategory::Other { name } => OptionCategory::Other {
                name: trimmed(&name, MAX_OPTION_NAME),
            },
            category @ (OptionCategory::Mode
            | OptionCategory::Model
            | OptionCategory::ModelConfig
            | OptionCategory::ThoughtLevel) => category,
        };
        let option = SessionOption {
            id: option.id,
            name: trimmed(&option.name, MAX_OPTION_NAME),
            description: option
                .description
                .map(|text| trimmed(&text, MAX_OPTION_DESCRIPTION)),
            category,
            kind,
        };
        // One that doesn't fit is left out; a smaller one after it may.
        let grown = len.saturating_add(encoded_len(serde_json::to_vec(&option)).saturating_add(1));
        if grown > TEXT_CHUNK {
            continue;
        }
        len = grown;
        keys.insert(option.id.clone(), key);
        options.push(option);
    }
    (options, keys)
}

/// The current value of each mode option, by id.
fn mode_values(options: &[SessionOption]) -> BTreeMap<String, String> {
    options
        .iter()
        .filter_map(|option| match (&option.category, &option.kind) {
            (OptionCategory::Mode, OptionControl::Select { current, .. }) => {
                Some((option.id.clone(), current.clone()))
            }
            (
                OptionCategory::Mode
                | OptionCategory::Model
                | OptionCategory::ModelConfig
                | OptionCategory::ThoughtLevel
                | OptionCategory::Other { .. },
                OptionControl::Select { .. } | OptionControl::Toggle { .. },
            ) => None,
        })
        .collect()
}

/// The name of the choice `value`, else the value itself.
fn choice_name<'a>(choices: &'a [OptionChoice], value: &'a str) -> &'a str {
    choices
        .iter()
        .find(|choice| choice.value == value)
        .map_or(value, |choice| choice.name.as_str())
}

/// A value as a refusal names it.
fn value_text(value: &OptionValue) -> &str {
    match value {
        OptionValue::Choice { value } => value,
        OptionValue::Toggle { on: true } => "true",
        OptionValue::Toggle { on: false } => "false",
    }
}

/// A value as a notice shows it: a choice's name, or on and off.
fn shown_value(option: &SessionOption, value: &OptionValue) -> String {
    match (&option.kind, value) {
        (OptionControl::Select { choices, .. }, OptionValue::Choice { value }) => {
            choice_name(choices, value).to_owned()
        }
        (OptionControl::Toggle { .. } | OptionControl::Select { .. }, OptionValue::Toggle { on }) => {
            if *on { "on" } else { "off" }.to_owned()
        }
        (OptionControl::Toggle { .. }, OptionValue::Choice { value }) => value.clone(),
    }
}

impl Step<'_> {
    /// The agent stopped (OpenSpec change `agents-commands`, decision 12):
    /// both lists are emptied, with an event each only when it held
    /// anything, and the keys and any set out are dropped. The kept mode
    /// values stay.
    pub(super) fn clear_lists(&mut self) {
        if !self.s.transcript.commands().is_empty() {
            self.emit(Event::Commands {
                commands: Vec::new(),
            });
        }
        if !self.s.transcript.options().is_empty() {
            self.emit(Event::Options {
                options: Vec::new(),
            });
        }
        self.s.keys.clear();
        self.s.rpc.forget_sets();
    }

    // --- commands and options (`agents-commands`) -----------------------

    /// The held options with their keys.
    pub(super) fn held_options(&self) -> Vec<OfferedOption> {
        self.s
            .transcript
            .options()
            .iter()
            .filter_map(|option| {
                let key = self.s.keys.get(&option.id)?.clone();
                Some(OfferedOption {
                    option: option.clone(),
                    key,
                })
            })
            .collect()
    }

    /// How an update's mode is compared: with the held one once the agent
    /// is ready, never before (a load's replay, a start not yet answered).
    pub(super) fn comparing(&self) -> Compare {
        match self.s.phase {
            Phase::Ready { .. } => Compare::Held,
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => Compare::None,
        }
    }

    /// The agent's commands, bounded and cleaned (decision 2): an event
    /// when they differ from the held ones, with one notice for what was
    /// left out.
    pub(super) fn offer_commands(&mut self, offered: Vec<SlashCommand>, unreadable: usize) {
        let total = offered.len().saturating_add(unreadable);
        let commands = bounded_commands(offered);
        if commands.as_slice() == self.s.transcript.commands() {
            return;
        }
        let kept = commands.len();
        self.emit(Event::Commands { commands });
        if kept < total {
            self.notice(
                NoticeLevel::Warning,
                format!("the agent offered {total} commands; {kept} are kept"),
            );
        }
    }

    /// The agent's options with their keys, bounded and cleaned (decision
    /// 2): an event when they differ from the held ones, with one notice
    /// for what was left out, and a notice for each mode option whose
    /// value differs from the one `compare` names (decision 5).
    pub(super) fn offer_options(&mut self, offered: Offered, compare: Compare) {
        let total = offered.options.len().saturating_add(offered.unreadable);
        let (options, keys) = bounded_options(offered.options);
        let kept = options.len();
        self.replace_options(options, keys, compare);
        if kept < total {
            self.notice(
                NoticeLevel::Warning,
                format!("the agent offered {total} options; {kept} are kept"),
            );
        }
    }

    /// Holds `options` and their `keys`: an event when the list differs,
    /// then the mode notices.
    fn replace_options(
        &mut self,
        options: Vec<SessionOption>,
        keys: BTreeMap<String, OptionKey>,
        compare: Compare,
    ) {
        let before: BTreeMap<String, String> = mode_values(self.s.transcript.options());
        self.s.keys = keys;
        if options.as_slice() != self.s.transcript.options() {
            self.emit(Event::Options {
                options: options.clone(),
            });
        }
        if compare == Compare::None {
            return;
        }
        for option in &options {
            let (OptionCategory::Mode, OptionControl::Select { current, choices }) =
                (&option.category, &option.kind)
            else {
                continue;
            };
            let earlier = match compare {
                Compare::Held => before.get(&option.id).or_else(|| self.s.modes.get(&option.id)),
                Compare::Kept | Compare::None => self.s.modes.get(&option.id),
            };
            if earlier.is_some_and(|earlier| earlier != current) {
                let name = choice_name(choices, current);
                self.notice(NoticeLevel::Info, format!("Mode: {name}"));
            }
        }
        self.s.modes.extend(mode_values(&options));
    }

    /// The current value of the option set by `key` changed (v1's
    /// `current_mode_update`): applied to the held option whose key equals
    /// it; with none, or a toggle, nothing changes.
    pub(super) fn set_current(&mut self, key: &OptionKey, value: String) {
        let Some(id) = self
            .s
            .keys
            .iter()
            .find_map(|(id, held)| (held == key).then(|| id.clone()))
        else {
            return;
        };
        self.set_value(&id, &OptionValue::Choice { value });
    }

    /// The held option `id` takes `value`, when it is of that kind.
    fn set_value(&mut self, id: &str, value: &OptionValue) {
        let mut options = self.s.transcript.options().to_vec();
        let Some(option) = options.iter_mut().find(|option| option.id == id) else {
            return;
        };
        match (&mut option.kind, value) {
            (OptionControl::Select { current, .. }, OptionValue::Choice { value }) => {
                current.clone_from(value);
            }
            (OptionControl::Toggle { on }, OptionValue::Toggle { on: value }) => *on = *value,
            (OptionControl::Select { .. }, OptionValue::Toggle { .. })
            | (OptionControl::Toggle { .. }, OptionValue::Choice { .. }) => return,
        }
        let keys = self.s.keys.clone();
        let compare = self.comparing();
        self.replace_options(options, keys, compare);
    }

    /// `AGENT_SET_OPTION` (decision 4): checked against the held options,
    /// then sent with its key, checkpointed first; the current value is
    /// `OK` with nothing sent.
    pub(super) fn set_option(&mut self, id: String, value: OptionValue) -> Result<(), Refused> {
        let Phase::Ready { session } = &self.s.phase else {
            return Err(Refused::NotReady);
        };
        let session = session.clone();
        let Some(option) = self.s.transcript.options().iter().find(|option| option.id == id) else {
            return Err(Refused::NoSuchSessionOption { id });
        };
        let Some(key) = self.s.keys.get(&id) else {
            return Err(Refused::NoSuchSessionOption { id });
        };
        let current = match (&option.kind, &value) {
            (OptionControl::Select { current, choices }, OptionValue::Choice { value: chosen }) => {
                if !choices.iter().any(|choice| choice.value == *chosen) {
                    return Err(Refused::ChoiceNotOffered {
                        id,
                        value: chosen.clone(),
                    });
                }
                current == chosen
            }
            (OptionControl::Toggle { on }, OptionValue::Toggle { on: wanted }) => on == wanted,
            (OptionControl::Select { .. }, OptionValue::Toggle { .. })
            | (OptionControl::Toggle { .. }, OptionValue::Choice { .. }) => {
                return Err(Refused::ChoiceNotOffered {
                    id,
                    value: value_text(&value).to_owned(),
                });
            }
        };
        if current {
            return Ok(());
        }
        if self.s.rpc.setting() {
            return Err(Refused::OptionBusy);
        }
        let key = key.clone();
        let line = self.s.rpc.set_option(&session, &id, &key, &value);
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
        Ok(())
    }

    /// The answer to a set of option `id` to `value`: the agent's list, or
    /// for an empty answer by design the value set; the agent's error, or
    /// an answer Brindle can't read, is a notice and changes nothing.
    pub(super) fn option_set(&mut self, id: &str, value: &OptionValue, result: Result<&Value, &RpcError>) {
        // The options went with the agent that was asked.
        let Some(key) = self.s.keys.get(id).cloned() else {
            return;
        };
        let why = match result {
            Ok(result) => match self.s.rpc.dialect().set_option_result(&key, result) {
                Ok(Some(offered)) => {
                    let merged = self.s.rpc.dialect().merge_options(&self.held_options(), offered);
                    return self.offer_options(merged, self.comparing());
                }
                Ok(None) => return self.set_value(id, value),
                Err(_) => "its answer isn't one Brindle can read".to_owned(),
            },
            Err(error) => clean(&error.message),
        };
        let (name, shown) = match self.s.transcript.options().iter().find(|option| option.id == id) {
            Some(option) => (option.name.clone(), shown_value(option, value)),
            None => (id.to_owned(), value_text(value).to_owned()),
        };
        self.notice(
            NoticeLevel::Warning,
            format!("the agent didn't change {name} to {shown}: {why}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::{self, json};
    use brindle_transcript::Continuation;


    use super::super::harness::*;
    use super::super::*;
    use super::*;

    // --- commands and options (agents-commands) -------------------------

    /// v1's `modes` in mode `current` of `default` and `plan`.
    fn modes_in(current: &str) -> Value {
        json!({"currentModeId": current, "availableModes": [
            {"id": "default", "name": "Default"},
            {"id": "plan", "name": "Plan", "description": "Plans first"}]})
    }

    /// The config options: a model select and a `fast` toggle.
    fn config_options(model: &str, fast: bool) -> Value {
        json!([
            {"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": model, "options": [
                {"value": "opus", "name": "Opus"}, {"value": "sonnet", "name": "Sonnet"}]},
            {"id": "fast", "name": "Fast", "category": "speed", "type": "boolean",
             "currentValue": fast}])
    }

    /// A start's answer for session `session` in mode `mode`, with the
    /// model `opus` and `fast` off.
    fn offering(session: &str, mode: &str) -> Value {
        json!({"sessionId": session, "modes": modes_in(mode),
               "configOptions": config_options("opus", false)})
    }

    fn commands_update(names: &[&str]) -> String {
        let commands: Vec<Value> = names
            .iter()
            .map(|name| json!({"name": name, "description": format!("Runs {name}"), "input": null}))
            .collect();
        update(json!({"sessionUpdate": "available_commands_update", "availableCommands": commands}))
    }

    fn choice(value: &str) -> OptionValue {
        OptionValue::Choice {
            value: value.into(),
        }
    }

    fn option_ids(agent: &Agent) -> Vec<&str> {
        agent
            .s
            .transcript
            .options()
            .iter()
            .map(|option| option.id.as_str())
            .collect()
    }

    fn current_of<'a>(agent: &'a Agent, id: &str) -> &'a OptionControl {
        &agent
            .s
            .transcript
            .options()
            .iter()
            .find(|option| option.id == id)
            .unwrap_or_else(|| panic!("no option {id}"))
            .kind
    }

    fn selected(value: &str) -> String {
        value.to_owned()
    }

    fn current_value(agent: &Agent, id: &str) -> String {
        match current_of(agent, id) {
            OptionControl::Select { current, .. } => current.clone(),
            OptionControl::Toggle { on } => on.to_string(),
        }
    }

    impl Agent {
        /// Ready in ACP session `s1` offering both continues, its
        /// `session/new` answered with [`offering`] in mode `mode`.
        fn with_options(mode: &str) -> Self {
            let mut agent = Self::new();
            agent.step(Input::Spawned);
            agent.feed(&initialized(1, both()));
            agent.feed(&result(2, offering("s1", mode)));
            agent
        }

        fn set(&mut self, option: &str, value: OptionValue) -> Output {
            self.cmd(Command::SetOption {
                option: option.into(),
                value,
            })
        }
    }

    #[test]
    fn a_starts_options_come_between_the_conversation_and_ready_with_no_notice() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        let out = agent.feed(&result(2, offering("s1", "default")));
        assert_eq!(shape(&out.events), ["conversation", "options 3", "agent ready"]);
        // The config options first, then the one made from the modes.
        assert_eq!(option_ids(&agent), ["model", "fast", "mode"]);
        // The mode's key is the dialect's own, which the reducer never names.
        assert!(!matches!(agent.s.keys["mode"], OptionKey::Config { .. }));
        assert_eq!(agent.s.keys["model"], OptionKey::Config { id: "model".into() });
        assert!(agent.notices().is_empty());
        assert_eq!(agent.s.modes, BTreeMap::from([("mode".into(), "default".into())]));
    }

    #[test]
    fn commands_sent_before_the_starts_answer_are_kept_and_an_equal_list_sends_nothing() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        // Copilot's, in `Phase::Creating`.
        let out = agent.feed(&commands_update(&["compact", "review"]));
        assert_eq!(shape(&out.events), ["commands 2"]);
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(agent.s.transcript.commands().len(), 2);
        assert_eq!(agent.feed(&commands_update(&["compact", "review"])).events, []);
        let out = agent.feed(&commands_update(&["compact"]));
        assert_eq!(shape(&out.events), ["commands 1"]);
        assert!(agent.notices().is_empty());
    }

    #[test]
    fn untypeable_commands_and_those_past_the_bound_are_left_out_with_one_notice() {
        let mut agent = Agent::ready();
        let mut names: Vec<String> = ["", "two words", "a/b", "brindle:mode", "tab\tbed"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        names.push("n".repeat(MAX_OPTION_NAME + 1));
        names.extend((0..MAX_COMMANDS + 4).map(|n| format!("c{n}")));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        agent.feed(&commands_update(&refs));
        let commands = agent.s.transcript.commands();
        assert_eq!(commands.len(), MAX_COMMANDS);
        assert_eq!(commands[0].name, "c0");
        assert_eq!(
            agent.notices(),
            [format!("the agent offered {} commands; {MAX_COMMANDS} are kept", refs.len())]
        );
        // An unreadable entry counts, a hint is cut and cleaned, and a
        // description is cut.
        let mut agent = Agent::ready();
        agent.feed(&update(json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "plan", "description": "d".repeat(MAX_OPTION_DESCRIPTION + 9),
             "input": {"hint": format!("\u{7}{}", "h".repeat(MAX_OPTION_NAME))}},
            {"description": "no name"}]})));
        let command = &agent.s.transcript.commands()[0];
        assert_eq!(command.description.len(), MAX_OPTION_DESCRIPTION);
        let hint = command.hint.as_deref().unwrap_or_default();
        assert!(hint.starts_with('\u{FFFD}') && hint.len() <= MAX_OPTION_NAME, "{hint}");
        assert_eq!(agent.notices(), ["the agent offered 2 commands; 1 are kept"]);
    }

    #[test]
    fn options_past_their_bounds_are_left_out_and_their_texts_cut() {
        let mut agent = Agent::ready();
        let mut options: Vec<Value> = (0..MAX_OPTIONS + 2)
            .map(|n| json!({"id": format!("o{n}"), "name": format!("O{n}"), "type": "boolean", "currentValue": true}))
            .collect();
        options.insert(
            0,
            json!({"id": "x".repeat(MAX_OPTION_ID + 1), "name": "Long", "type": "boolean", "currentValue": true}),
        );
        let choices: Vec<Value> = (0..MAX_CHOICES + 3)
            .map(|n| {
                let name = if n == 0 { "n".repeat(MAX_OPTION_NAME * 2) } else { format!("C{n}") };
                json!({"value": format!("v{n}"), "name": name})
            })
            .collect();
        options.insert(
            1,
            json!({"id": "many", "name": "Many", "type": "select", "currentValue": "v0",
                   "description": "d".repeat(MAX_OPTION_DESCRIPTION * 2), "options": choices}),
        );
        agent.feed(&update(json!({"sessionUpdate": "config_option_update", "configOptions": options})));
        let held = agent.s.transcript.options();
        assert_eq!(held.len(), MAX_OPTIONS);
        assert_eq!(held[0].id, "many");
        assert_eq!(held[0].description.as_deref().map(str::len), Some(MAX_OPTION_DESCRIPTION));
        let OptionControl::Select { choices, .. } = &held[0].kind else {
            panic!("{:?}", held[0].kind);
        };
        assert_eq!(choices.len(), MAX_CHOICES);
        assert_eq!(choices[0].name.len(), MAX_OPTION_NAME);
        assert_eq!(agent.s.keys.len(), MAX_OPTIONS);
        assert_eq!(
            agent.notices(),
            [format!("the agent offered {} options; {MAX_OPTIONS} are kept", MAX_OPTIONS + 4)]
        );
        // A list too big for one event keeps what fits.
        let mut agent = Agent::ready();
        let huge: Vec<Value> = (0..MAX_OPTIONS)
            .map(|n| {
                let choices: Vec<Value> = (0..MAX_CHOICES)
                    .map(|c| json!({"value": format!("v{c}"), "name": "n".repeat(MAX_OPTION_NAME)}))
                    .collect();
                json!({"id": format!("o{n}"), "name": "O", "type": "select", "currentValue": "v0", "options": choices})
            })
            .chain([json!({"id": "small", "name": "S", "type": "boolean", "currentValue": true})])
            .collect();
        agent.feed(&update(json!({"sessionUpdate": "config_option_update", "configOptions": huge})));
        assert_eq!(option_ids(&agent), ["small"]);
        let encoded = serde_json::to_vec(&Event::Options {
            options: agent.s.transcript.options().to_vec(),
        })
        .unwrap_or_default();
        assert!(encoded.len() <= TEXT_CHUNK, "{}", encoded.len());
    }

    #[test]
    fn a_set_is_refused_with_its_reason_before_anything_is_sent() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        assert_eq!(agent.set("mode", choice("plan")).reply, Some(Err(Refused::NotReady)));
        let mut agent = Agent::with_options("default");
        let sent = agent.sent.len();
        let refusals = [
            ("speed", choice("x"), "the agent offers no option \"speed\""),
            ("model", choice("gpt-5"), "option \"model\" offers no choice \"gpt-5\""),
            ("fast", choice("on"), "option \"fast\" offers no choice \"on\""),
            ("mode", OptionValue::Toggle { on: true }, "option \"mode\" offers no choice \"true\""),
        ];
        for (option, value, text) in refusals {
            let out = agent.set(option, value);
            let Some(Err(refused)) = out.reply else {
                panic!("{option}: {:?}", out.reply);
            };
            assert_eq!(refused.to_string(), text);
            assert!(out.events.is_empty() && out.stdin.is_empty() && out.checkpoint.is_none());
        }
        // The current value is asked for: nothing is sent.
        let out = agent.set("model", choice("opus"));
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.stdin.is_empty());
        assert_eq!(agent.set("fast", OptionValue::Toggle { on: false }).reply, Some(Ok(())));
        assert_eq!(agent.sent.len(), sent);
        // One set out at a time.
        assert_eq!(agent.set("model", choice("sonnet")).reply, Some(Ok(())));
        let busy = agent.set("mode", choice("plan")).reply;
        assert_eq!(busy, Some(Err(Refused::OptionBusy)));
        assert_eq!(
            Refused::OptionBusy.to_string(),
            "an earlier change of an option hasn't been answered yet"
        );
    }

    #[test]
    fn a_config_options_set_sends_its_key_and_takes_the_agents_list_keeping_the_modes() {
        let mut agent = Agent::with_options("default");
        let out = agent.set("model", choice("sonnet"));
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        let line = agent.last("session/set_config_option").clone();
        assert_eq!(
            line["params"],
            json!({"sessionId": "s1", "configId": "model", "value": "sonnet"})
        );
        // Nothing changes until the agent answers.
        assert_eq!(current_value(&agent, "model"), "opus");
        let id = line["id"].as_u64().expect("an id");
        let out = agent.feed(&result(id, json!({"configOptions": config_options("sonnet", false)})));
        assert_eq!(shape(&out.events), ["options 3"]);
        assert_eq!(current_value(&agent, "model"), selected("sonnet"));
        assert_eq!(option_ids(&agent), ["model", "fast", "mode"]);
        assert!(agent.notices().is_empty(), "a model change adds no notice");
        // A toggle's set names its type.
        agent.set("fast", OptionValue::Toggle { on: true });
        assert_eq!(
            agent.last("session/set_config_option")["params"],
            json!({"sessionId": "s1", "configId": "fast", "type": "boolean", "value": true})
        );
    }

    #[test]
    fn a_modes_set_takes_its_empty_answer_as_the_value_and_says_so() {
        let mut agent = Agent::with_options("default");
        agent.set("mode", choice("plan"));
        let line = agent.last("session/set_mode").clone();
        assert_eq!(line["params"], json!({"sessionId": "s1", "modeId": "plan"}));
        let out = agent.feed(&result(line["id"].as_u64().expect("an id"), json!({})));
        assert_eq!(shape(&out.events), ["options 3", "added notice"]);
        assert_eq!(current_value(&agent, "mode"), "plan");
        assert_eq!(agent.notices(), ["Mode: Plan"]);
        assert_eq!(agent.s.modes["mode"], "plan");
        // And so does the agent's own change of mode.
        let out = agent.feed(&update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "default"})));
        assert_eq!(shape(&out.events), ["options 3", "added notice"]);
        assert_eq!(agent.notices(), ["Mode: Plan", "Mode: Default"]);
        // A mode none of the choices names shows as itself.
        agent.feed(&update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "yolo"})));
        assert_eq!(agent.notices().last().map(String::as_str), Some("Mode: yolo"));
    }

    #[test]
    fn a_mode_update_changes_nothing_when_a_config_option_is_the_mode() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        agent.feed(&result(2, json!({"sessionId": "s1", "modes": modes_in("default"), "configOptions": [
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default",
             "options": [{"value": "default", "name": "Default"}, {"value": "plan", "name": "Plan"}]}]})));
        assert_eq!(option_ids(&agent), ["mode"]);
        assert_eq!(agent.s.keys["mode"], OptionKey::Config { id: "mode".into() });
        let out = agent.feed(&update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"})));
        assert_eq!(out.events, []);
        // Its own update says the mode changed.
        let out = agent.feed(&update(json!({"sessionUpdate": "config_option_update", "configOptions": [
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "plan",
             "options": [{"value": "default", "name": "Default"}, {"value": "plan", "name": "Plan"}]}]})));
        assert_eq!(shape(&out.events), ["options 1", "added notice"]);
        assert_eq!(agent.notices(), ["Mode: Plan"]);
    }

    #[test]
    fn the_agents_refusal_of_a_set_is_a_notice_and_the_options_stay() {
        let mut agent = Agent::with_options("default");
        agent.set("model", choice("sonnet"));
        let id = agent.last("session/set_config_option")["id"].as_u64().expect("an id");
        let before = agent.s.transcript.options().to_vec();
        let out = agent.feed(&error(id, -32602, "model unavailable"));
        assert_eq!(shape(&out.events), ["added notice"]);
        assert_eq!(agent.s.transcript.options(), before.as_slice());
        assert_eq!(agent.notices(), ["the agent didn't change Model to Sonnet: model unavailable"]);
        // The next set goes out.
        assert_eq!(agent.set("fast", OptionValue::Toggle { on: true }).reply, Some(Ok(())));
        let id = agent.last("session/set_config_option")["id"].as_u64().expect("an id");
        agent.feed(&result(id, json!({"nope": 1})));
        assert_eq!(
            agent.notices().last().map(String::as_str),
            Some("the agent didn't change Fast to on: its answer isn't one Brindle can read")
        );
    }

    #[test]
    fn every_stop_empties_both_lists_and_drops_a_set_out() {
        let stops: [fn(&mut Agent); 3] = [
            |agent| {
                agent.exit();
            },
            |agent| {
                agent.step(Input::Lost { message: "gone" });
            },
            |agent| {
                agent.step(Input::Fault {
                    reason: "bug".into(),
                });
            },
        ];
        for stop in stops {
            let mut agent = Agent::with_options("default");
            agent.feed(&commands_update(&["compact"]));
            agent.set("model", choice("sonnet"));
            let before = agent.events.len();
            stop(&mut agent);
            let emptied: Vec<String> = shape(&agent.events[before..])
                .into_iter()
                .filter(|kind| kind.starts_with("commands") || kind.starts_with("options"))
                .collect();
            assert_eq!(emptied, ["commands 0", "options 0"]);
            assert!(agent.s.keys.is_empty() && !agent.s.rpc.setting());
            // The mode last in effect is kept.
            assert_eq!(agent.s.modes["mode"], "default");
            assert_eq!(agent.set("model", choice("sonnet")).reply, Some(Err(Refused::NotReady)));
        }
        // A list held by nothing sends no event.
        let mut agent = Agent::ready();
        let before = agent.events.len();
        agent.exit();
        assert!(!shape(&agent.events[before..]).iter().any(|kind| kind.starts_with("options")));
    }

    #[test]
    fn a_refused_start_and_a_login_restart_empty_the_lists_and_a_login_keeps_them() {
        // A refused start: the commands sent while creating go.
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        agent.feed(&commands_update(&["compact"]));
        agent.feed(&error(2, -32603, "no"));
        assert!(agent.s.transcript.commands().is_empty());
        // A login demanded mid-conversation keeps them, refusing sets.
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, both()));
        agent.feed(&result(2, offering("s1", "default")));
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert!(matches!(agent.s.phase(), Phase::LoginRequired(_)));
        assert_eq!(agent.s.transcript.options().len(), 3);
        assert_eq!(agent.set("mode", choice("plan")).reply, Some(Err(Refused::NotReady)));
        // Its restart for a terminal login hangs the agent up: they go.
        let out = agent.step(Input::Restart);
        assert!(out.hang_up);
        assert!(agent.s.transcript.options().is_empty() && agent.s.keys.is_empty());
    }

    #[test]
    fn a_continue_takes_its_answers_options_and_says_when_the_mode_differs() {
        for (mode, notices) in [("default", vec![]), ("plan", vec!["Mode: Plan"])] {
            let mut agent = Agent::with_options("default");
            let id = agent.turn("first");
            agent.end(id, "end_turn");
            agent.exit();
            let id = agent.restart();
            agent.feed(&initialized(id, resume_only()));
            let resume = agent.last("session/resume")["id"].as_u64().expect("an id");
            let before = agent.events.len();
            let mut answer = offering("s1", mode);
            answer.as_object_mut().map(|answer| answer.remove("sessionId"));
            agent.feed(&result(resume, answer));
            let kinds = shape(&agent.events[before..]);
            assert_eq!(kinds[..2], ["conversation".to_owned(), "options 3".to_owned()], "{kinds:?}");
            assert_eq!(option_ids(&agent), ["model", "fast", "mode"]);
            let new: Vec<String> = agent
                .notices()
                .into_iter()
                .filter(|text| text.starts_with("Mode"))
                .collect();
            assert_eq!(new, notices, "{mode}");
        }
    }

    #[test]
    fn a_muted_loads_lists_apply_uncounted_with_no_mode_notice_and_its_answer_stands() {
        let mut agent = Agent::with_options("default");
        let id = agent.turn("first");
        agent.feed(&said("Done."));
        agent.end(id, "end_turn");
        agent.exit();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        for line in [
            chunk("user_message_chunk", "first"),
            commands_update(&["compact"]),
            update(json!({"sessionUpdate": "config_option_update", "configOptions": config_options("sonnet", true)})),
            update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"})),
            update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "default"})),
            said("Done."),
        ] {
            agent.feed(&line);
        }
        assert_eq!(agent.s.transcript.commands().len(), 1);
        // The replay's config options, with no mode option yet to change.
        assert_eq!(option_ids(&agent), ["model", "fast"]);
        let mut answer = offering("s1", "plan");
        answer.as_object_mut().map(|answer| answer.remove("sessionId"));
        agent.feed(&result(id + 1, answer));
        assert_eq!(
            agent.notices(),
            [
                "the agent was started again to continue the conversation".to_owned(),
                "Mode: Plan".to_owned(),
                "conversation continued; the agent's replay of 2 updates wasn't shown".to_owned(),
            ]
        );
        assert_eq!(current_value(&agent, "model"), "opus");
        assert_eq!(current_value(&agent, "mode"), "plan");
    }

    #[test]
    fn a_shown_loads_mode_updates_add_no_notice() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Load), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        agent.feed(&result(2, json!(null)));
        assert!(agent.notices().iter().all(|text| !text.starts_with("Mode")));
    }

    #[test]
    fn a_set_out_at_an_exit_is_forgotten_and_its_late_answer_changes_nothing() {
        let mut agent = Agent::with_options("default");
        agent.set("mode", choice("plan"));
        let id = agent.last("session/set_mode")["id"].as_u64().expect("an id");
        agent.exit();
        assert_eq!(agent.feed(&result(id, json!({}))), Output::default());
        assert!(agent.s.transcript.options().is_empty());
    }
}
