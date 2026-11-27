//! An agent zone's slash commands and session options (OpenSpec change
//! `agents-commands`, design decision 6): what a prompt beginning with `/`
//! is ([`command_text`]), the commands of Brindle's own that set an option
//! ([`option_commands`]), the list shown above the prompt input while a
//! command word is typed ([`completion`]) and its keys
//! ([`completion_key`]), what a submit does before anything is sent
//! ([`local_command`]), the options under the input ([`option_items`]),
//! the next mode ([`next_mode`]) and the options palette's rows
//! ([`options_palette`]).
//!
//! Pure: values in, values out. The session's lists come from the server
//! as the transcript's `Commands` and `Options` events; nothing here is
//! shown as changed until the server says it is.

use std::ops::Range;

use brindle_ui::filter::fuzzy_match;

use crate::agent::transcript::{OptionCategory, OptionChoice, OptionControl, SessionOption, SlashCommand};
use crate::picker::{PaletteGroup, PaletteRow};
use crate::remote::protocol::client::OptionValue;

/// What every option command's long name starts with: `/brindle:model` is
/// always Brindle's, whatever the agent offers (the server leaves out an
/// agent's command whose name starts with it).
pub const LONG_PREFIX: &str = "brindle:";

/// The text a prompt's command starts at: from its first character that
/// isn't whitespace, when that is `/`; `None` for any other prompt, which
/// is sent exactly as typed (the user, 2026-11-25: an accidental space
/// mustn't change behaviour).
pub fn command_text(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    trimmed.starts_with('/').then_some(trimmed)
}

/// One option's command of Brindle's own: the option whose id is `option`,
/// typed as `/NAME` or `/brindle:NAME` (`long`). `shadowed`: the agent
/// offers a command named `name`, so only the long name reaches this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionCommand {
    pub option: String,
    pub name: String,
    pub long: String,
    pub shadowed: bool,
}

impl OptionCommand {
    /// Whether `word` (what follows the `/`) names this command: its long
    /// name always, its short name unless the agent's command holds it.
    fn named(&self, word: &str) -> bool {
        word == self.long || (!self.shadowed && word == self.name)
    }
}

/// An option's command name: its name in lower case, each run of
/// characters other than ASCII letters and digits one `-`, trimmed
/// (`Reasoning effort` → `reasoning-effort`).
pub fn command_name(option_name: &str) -> String {
    let mut name = String::new();
    for c in option_name.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    while name.ends_with('-') {
        name.pop();
    }
    name
}

/// One command per option, in the options' order: an option whose name
/// gives an empty command name is skipped, a second of the same name gets
/// `-2` (a third `-3`), and one whose name an agent's command holds is
/// shadowed.
pub fn option_commands(options: &[SessionOption], agent: &[SlashCommand]) -> Vec<OptionCommand> {
    let mut out: Vec<OptionCommand> = Vec::new();
    for option in options {
        let base = command_name(&option.name);
        if base.is_empty() {
            continue;
        }
        let mut name = base.clone();
        let mut n = 2;
        while out.iter().any(|command| command.name == name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        let shadowed = agent.iter().any(|command| command.name == name);
        out.push(OptionCommand { option: option.id.clone(), long: format!("{LONG_PREFIX}{name}"), name, shadowed });
    }
    out
}

/// The shown value of `option`: its current choice's name (the value
/// itself when no choice has it), or `on`/`off`.
pub fn current_label(option: &SessionOption) -> String {
    match &option.kind {
        OptionControl::Select { current, choices } => {
            choices.iter().find(|choice| choice.value == *current).map_or_else(|| current.clone(), |c| c.name.clone())
        }
        OptionControl::Toggle { on } => on_off(*on).to_string(),
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// What the completion list shows: an agent's or an option's command
/// names, or one option's choices after its command and a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Commands,
    /// The choices of option `option`, typed as `/command`.
    Choices { command: String, option: String },
}

/// What a row of the list stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// A command, as its row names it (`brindle:model` included).
    Command { name: String },
    /// One of the option's choices.
    Choice { value: OptionValue },
}

/// A row of the completion list: its label, the command's input hint and
/// its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub hint: Option<String>,
    pub description: Option<String>,
    pub pick: Pick,
}

/// The completion list: its stage, the word or the rest of the line typed
/// (`query`), the text's bytes a choice replaces, its rows (never empty)
/// and the highlighted one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub stage: Stage,
    pub query: String,
    pub range: Range<usize>,
    pub items: Vec<Suggestion>,
    pub highlighted: usize,
}

impl Completion {
    /// The dump's `--- completion` line and one row per item, as palette
    /// rows print (`label  hint  description`).
    pub fn lines(&self) -> String {
        let mut out = String::from("--- completion");
        if let Stage::Choices { command, .. } = &self.stage {
            out.push_str(&format!(" command={command:?}"));
        }
        out.push_str(&format!(" query={:?} highlighted={}\n", self.query, self.highlighted));
        for item in &self.items {
            let row = format!(
                "{}  {}  {}",
                item.label,
                item.hint.as_deref().unwrap_or(""),
                item.description.as_deref().unwrap_or("")
            );
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }
}

/// Where a prompt's command line is in `text`: the `/`'s byte, the end of
/// the command word and the end of the line.
fn command_line(text: &str) -> Option<(usize, usize, usize)> {
    let slash = text.len() - command_text(text)?.len();
    let line_end = text[slash..].find('\n').map_or(text.len(), |at| slash + at);
    let word_end = text[slash..line_end].find(char::is_whitespace).map_or(line_end, |at| slash + at);
    Some((slash, word_end, line_end))
}

/// Whether the cursor at byte `cursor` is in a command word or the rest of
/// its line: where a dismissed list stays dismissed.
pub fn in_command(text: &str, cursor: usize) -> bool {
    command_line(text).is_some_and(|(slash, _, line_end)| cursor > slash && cursor <= line_end)
}

/// How well `query` matches any of `names`: 0 for a prefix (ignoring
/// case), 1 for a fuzzy match, `None` for none.
fn rank<'a>(query: &str, names: impl IntoIterator<Item = &'a str> + Clone) -> Option<u8> {
    let lower = query.trim().to_lowercase();
    if names.clone().into_iter().any(|name| name.to_lowercase().starts_with(&lower)) {
        Some(0)
    } else if names.into_iter().any(|name| fuzzy_match(query, name)) {
        Some(1)
    } else {
        None
    }
}

/// The items in rank order, prefix matches first, each group in the order
/// given.
fn ranked<T>(items: impl IntoIterator<Item = (Option<u8>, T)>) -> Vec<T> {
    let mut kept: Vec<(u8, T)> = items.into_iter().filter_map(|(rank, item)| rank.map(|rank| (rank, item))).collect();
    kept.sort_by_key(|(rank, _)| *rank);
    kept.into_iter().map(|(_, item)| item).collect()
}

/// The option of `options` that the command word `word` names, with its
/// command.
fn named_option<'a>(
    word: &str,
    options: &'a [SessionOption],
    agent: &[SlashCommand],
) -> Option<(OptionCommand, &'a SessionOption)> {
    let command = option_commands(options, agent).into_iter().find(|command| command.named(word))?;
    let option = options.iter().find(|option| option.id == command.option)?;
    Some((command, option))
}

/// The choices of `option` matching `query` (by name or value, ignoring
/// case; prefix matches first), each with its value; a toggle's are `on`
/// and `off`.
fn choices(option: &SessionOption, query: &str) -> Vec<(String, Option<String>, OptionValue)> {
    let all: Vec<(String, Option<String>, OptionValue)> = match &option.kind {
        OptionControl::Select { choices, .. } => choices
            .iter()
            .map(|c| (c.name.clone(), c.description.clone(), OptionValue::Choice { value: c.value.clone() }))
            .collect(),
        OptionControl::Toggle { .. } => [true, false]
            .into_iter()
            .map(|on| (on_off(on).to_string(), None, OptionValue::Toggle { on }))
            .collect(),
    };
    if query.trim().is_empty() {
        return all;
    }
    ranked(all.into_iter().map(|choice| {
        let value = match &choice.2 {
            OptionValue::Choice { value } => value.clone(),
            OptionValue::Toggle { on } => on_off(*on).to_string(),
        };
        (rank(query, [choice.0.as_str(), value.as_str()]), choice)
    }))
}

/// The value `option` holds now, as a set would name it.
fn current_value(option: &SessionOption) -> OptionValue {
    match &option.kind {
        OptionControl::Select { current, .. } => OptionValue::Choice { value: current.clone() },
        OptionControl::Toggle { on } => OptionValue::Toggle { on: *on },
    }
}

/// The completion list for `text` with the cursor at byte `cursor`, the
/// agent's `commands` and the session's `options`: with the cursor in the
/// command word, the commands matching it (the agent's, then the option
/// commands, prefix matches first); after an option command and a space on
/// a one-line text, that option's choices matching the rest of the line,
/// the current one highlighted while nothing is typed. `None` when no row
/// matches, or the text isn't a command.
pub fn completion(text: &str, cursor: usize, commands: &[SlashCommand], options: &[SessionOption]) -> Option<Completion> {
    let (slash, word_end, line_end) = command_line(text)?;
    let word = &text[slash + 1..word_end];
    if cursor > slash && cursor <= word_end {
        let long_typed = word.starts_with(LONG_PREFIX);
        let agent = commands.iter().map(|command| {
            let item = Suggestion {
                label: format!("/{}", command.name),
                hint: command.hint.clone(),
                description: Some(command.description.clone()).filter(|d| !d.is_empty()),
                pick: Pick::Command { name: command.name.clone() },
            };
            (rank(word, [command.name.as_str()]), item)
        });
        let own = option_commands(options, commands).into_iter().filter_map(|command| {
            let option = options.iter().find(|option| option.id == command.option)?;
            let shown = if command.shadowed || long_typed { &command.long } else { &command.name };
            // By the name the word is typed as: the long one after
            // `brindle:`, else the short one (shown long when shadowed), so
            // the prefix's letters match nothing by themselves.
            let names = if long_typed { [command.long.as_str()] } else { [command.name.as_str()] };
            let hint = match &option.kind {
                OptionControl::Select { .. } => "<choice>",
                OptionControl::Toggle { .. } => "on|off",
            };
            let item = Suggestion {
                label: format!("/{shown}"),
                hint: Some(hint.to_string()),
                description: Some(format!("{}: {}", option.name, current_label(option))),
                pick: Pick::Command { name: shown.clone() },
            };
            Some((rank(word, names), item))
        });
        let items = ranked(agent.collect::<Vec<_>>().into_iter().chain(own.collect::<Vec<_>>()));
        return (!items.is_empty()).then(|| Completion {
            stage: Stage::Commands,
            query: word.to_string(),
            range: slash..word_end,
            items,
            highlighted: 0,
        });
    }
    // The second stage: one line, the cursor after the word.
    if line_end != text.len() || cursor <= word_end || word_end == line_end {
        return None;
    }
    let (_, option) = named_option(word, options, commands)?;
    let rest = &text[word_end..line_end];
    let query = rest.trim_start();
    let start = line_end - query.len();
    let found = choices(option, query);
    if found.is_empty() {
        return None;
    }
    let highlighted = if query.trim().is_empty() {
        let current = current_value(option);
        found.iter().position(|(_, _, value)| *value == current).unwrap_or(0)
    } else {
        0
    };
    let items = found
        .into_iter()
        .map(|(name, description, value)| Suggestion { label: name, hint: None, description, pick: Pick::Choice { value } })
        .collect();
    Some(Completion {
        stage: Stage::Choices { command: word.to_string(), option: option.id.clone() },
        query: query.to_string(),
        range: start..line_end,
        items,
        highlighted,
    })
}

/// A key the completion list takes while it shows, before the prompt
/// input's own handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKey {
    Up,
    Down,
    Enter,
    Tab,
    Escape,
}

/// What a key does to the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionAction {
    /// Move the highlight, wrapping.
    Previous,
    Next,
    /// Write the highlighted command's name and a space into the text.
    Accept,
    /// Write the highlighted choice's name into the text.
    Write,
    /// Set the highlighted choice.
    Set,
    /// Close the list until the text stops being a command word.
    Dismiss,
}

/// What `key` does in `stage`: up and down move; enter and tab take a
/// command's name; for a choice, tab writes its name and enter sets it;
/// escape dismisses.
pub fn completion_key(key: CompletionKey, stage: &Stage) -> CompletionAction {
    match (key, stage) {
        (CompletionKey::Up, _) => CompletionAction::Previous,
        (CompletionKey::Down, _) => CompletionAction::Next,
        (CompletionKey::Escape, _) => CompletionAction::Dismiss,
        (CompletionKey::Enter | CompletionKey::Tab, Stage::Commands) => CompletionAction::Accept,
        (CompletionKey::Tab, Stage::Choices { .. }) => CompletionAction::Write,
        (CompletionKey::Enter, Stage::Choices { .. }) => CompletionAction::Set,
    }
}

/// The highlight moved by one, wrapping over `len` rows.
pub fn moved(highlighted: usize, len: usize, forward: bool) -> usize {
    match (forward, len) {
        (_, 0) => 0,
        (true, _) => (highlighted + 1) % len,
        (false, _) => (highlighted + len - 1) % len,
    }
}

/// `text` with item `item` of `completion` written in: a command's `/NAME`
/// and a space in place of the command word, or a choice's name in place
/// of the rest of the line; and the cursor after it.
pub fn accept(text: &str, completion: &Completion, item: usize) -> Option<(String, usize)> {
    let suggestion = completion.items.get(item)?;
    let range = completion.range.clone();
    let written = match (&completion.stage, &suggestion.pick) {
        (Stage::Commands, Pick::Command { name }) => {
            let spaced = text[range.end..].starts_with(char::is_whitespace);
            if spaced { format!("/{name}") } else { format!("/{name} ") }
        }
        (Stage::Choices { .. }, Pick::Choice { .. }) => suggestion.label.clone(),
        (Stage::Commands, Pick::Choice { .. }) | (Stage::Choices { .. }, Pick::Command { .. }) => return None,
    };
    let mut out = String::with_capacity(text.len() + written.len());
    out.push_str(&text[..range.start]);
    out.push_str(&written);
    let mut cursor = out.len();
    if text[range.end..].starts_with(char::is_whitespace) && matches!(completion.stage, Stage::Commands) {
        // Past the space already there.
        cursor += 1;
    }
    out.push_str(&text[range.end..]);
    let cursor = cursor.min(out.len());
    Some((out, cursor))
}

/// What a submit does with a text before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Local {
    /// Ask the server to set option `option` (named `name`) to `value`.
    Set { option: String, name: String, value: OptionValue },
    /// Show option `option`'s choices: its command was sent alone.
    Choose { option: String },
    /// Nothing matched `text` among the choices of the option named
    /// `option`: keep the text, saying so.
    NoSuchChoice { option: String, text: String },
}

impl Local {
    /// The reason a [`Local::NoSuchChoice`] shows beside the input.
    pub fn reason(&self) -> Option<String> {
        match self {
            Self::NoSuchChoice { option, text } => Some(format!("{option} has no choice {text:?}")),
            Self::Set { .. } | Self::Choose { .. } => None,
        }
    }
}

/// What submitting `text` does when it is an option command (`None`: send
/// it as a prompt): a one-line command text naming an option command
/// (`/mode plan`, ` /brindle:model so`), whose rest picks the best
/// matching choice; a toggle takes `on` and `off`, and flips alone; a
/// select alone shows its choices.
pub fn local_command(text: &str, options: &[SessionOption], agent: &[SlashCommand]) -> Option<Local> {
    let command = command_text(text)?.trim_end();
    if command.contains('\n') {
        return None;
    }
    let (word, rest) = command[1..].split_once(char::is_whitespace).unwrap_or((&command[1..], ""));
    let (_, option) = named_option(word, options, agent)?;
    let rest = rest.trim();
    let name = option.name.clone();
    if rest.is_empty() {
        return Some(match &option.kind {
            OptionControl::Select { .. } => Local::Choose { option: option.id.clone() },
            OptionControl::Toggle { on } => {
                Local::Set { option: option.id.clone(), name, value: OptionValue::Toggle { on: !on } }
            }
        });
    }
    Some(match choices(option, rest).into_iter().next() {
        Some((_, _, value)) => Local::Set { option: option.id.clone(), name, value },
        None => Local::NoSuchChoice { option: name, text: rest.to_string() },
    })
}

/// An option under the prompt input: its id and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionItem {
    pub id: String,
    pub label: String,
}

/// Where a category's options come under the input.
fn category_rank(category: &OptionCategory) -> u8 {
    match category {
        OptionCategory::Mode => 0,
        OptionCategory::Model => 1,
        OptionCategory::ThoughtLevel => 2,
        OptionCategory::ModelConfig => 3,
        OptionCategory::Other { .. } => 4,
    }
}

/// The options under the prompt input: mode, model, thinking level, model
/// configuration, then the others in the agent's order; a select by its
/// current choice's name, a toggle by its name and `on` or `off`.
pub fn option_items(options: &[SessionOption]) -> Vec<OptionItem> {
    let mut ordered: Vec<&SessionOption> = options.iter().collect();
    ordered.sort_by_key(|option| category_rank(&option.category));
    ordered
        .into_iter()
        .map(|option| {
            let label = match &option.kind {
                OptionControl::Select { .. } => current_label(option),
                OptionControl::Toggle { on } => format!("{} {}", option.name, on_off(*on)),
            };
            OptionItem { id: option.id.clone(), label }
        })
        .collect()
}

/// The first select option of category mode and the choice after its
/// current one, wrapping (the first when the current one isn't a choice);
/// `None` without one, or with fewer than two choices.
pub fn next_mode(options: &[SessionOption]) -> Option<(String, OptionValue)> {
    options.iter().find_map(|option| match (&option.category, &option.kind) {
        (OptionCategory::Mode, OptionControl::Select { current, choices }) if choices.len() >= 2 => {
            let next = choices.iter().position(|choice| choice.value == *current).map_or(0, |at| (at + 1) % choices.len());
            Some((option.id.clone(), OptionValue::Choice { value: choices[next].value.clone() }))
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
}

/// What choosing a row of the options palette does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionPick {
    /// List option `option`'s choices.
    Open { option: String },
    /// Ask the server to set option `option` (named `name`) to `value`.
    Set { option: String, name: String, value: OptionValue },
}

/// The options palette: its placeholder, its groups, and what each row
/// does, across the groups in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsList {
    pub title: String,
    pub groups: Vec<PaletteGroup>,
    pub picks: Vec<OptionPick>,
}

fn row(label: String, detail: Option<String>) -> PaletteRow {
    PaletteRow { label: label.into(), detail: detail.map(Into::into), shortcut: None, disabled: false }
}

/// The options palette for `option` (`None`: every option, in the order
/// the input shows them, as `Name: value`, a select opening its choices
/// and a toggle flipping), or that option's choices, grouped under the
/// agent's group names (the ungrouped first), the current one's detail
/// saying so. `None` when there is nothing to list.
pub fn options_palette(options: &[SessionOption], option: Option<&str>) -> Option<OptionsList> {
    let Some(id) = option else {
        let items = option_items(options);
        if items.is_empty() {
            return None;
        }
        let (mut rows, mut picks) = (Vec::new(), Vec::new());
        for item in items {
            let option = options.iter().find(|option| option.id == item.id)?;
            rows.push(row(format!("{}: {}", option.name, current_label(option)), option.description.clone()));
            picks.push(match &option.kind {
                OptionControl::Select { .. } => OptionPick::Open { option: option.id.clone() },
                OptionControl::Toggle { on } => OptionPick::Set {
                    option: option.id.clone(),
                    name: option.name.clone(),
                    value: OptionValue::Toggle { on: !on },
                },
            });
        }
        return Some(OptionsList { title: "Agent options".into(), groups: vec![PaletteGroup::unnamed(rows)], picks });
    };
    let option = options.iter().find(|option| option.id == id)?;
    let current = current_value(option);
    let detail = |value: &OptionValue, description: Option<String>| {
        let mark = (*value == current).then(|| "current".to_string());
        match (mark, description) {
            (Some(mark), Some(description)) => Some(format!("{mark} · {description}")),
            (mark, description) => mark.or(description),
        }
    };
    let pick = |value: OptionValue| OptionPick::Set { option: option.id.clone(), name: option.name.clone(), value };
    let (groups, picks) = match &option.kind {
        OptionControl::Select { choices, .. } => grouped(choices, |choice| {
            let value = OptionValue::Choice { value: choice.value.clone() };
            (row(choice.name.clone(), detail(&value, choice.description.clone())), pick(value))
        }),
        OptionControl::Toggle { .. } => {
            let (rows, picks): (Vec<_>, Vec<_>) = [true, false]
                .into_iter()
                .map(|on| {
                    let value = OptionValue::Toggle { on };
                    (row(on_off(on).to_string(), detail(&value, None)), pick(value))
                })
                .unzip();
            (vec![PaletteGroup::unnamed(rows)], picks)
        }
    };
    Some(OptionsList { title: option.name.clone(), groups, picks })
}

/// `choices` as groups in the order their names first come (the
/// ungrouped first), each row with its pick, the picks in the rows'
/// order across the groups.
fn grouped(
    choices: &[OptionChoice],
    each: impl Fn(&OptionChoice) -> (PaletteRow, OptionPick),
) -> (Vec<PaletteGroup>, Vec<OptionPick>) {
    let mut names: Vec<Option<&str>> = vec![None];
    for choice in choices {
        let group = choice.group.as_deref();
        if !names.contains(&group) {
            names.push(group);
        }
    }
    let (mut groups, mut picks) = (Vec::new(), Vec::new());
    for name in names {
        let (rows, these): (Vec<_>, Vec<_>) =
            choices.iter().filter(|choice| choice.group.as_deref() == name).map(&each).unzip();
        if rows.is_empty() {
            continue;
        }
        picks.extend(these);
        groups.push(match name {
            Some(name) => PaletteGroup::named(name.to_string(), rows),
            None => PaletteGroup::unnamed(rows),
        });
    }
    (groups, picks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, description: &str, hint: Option<&str>) -> SlashCommand {
        SlashCommand { name: name.into(), description: description.into(), hint: hint.map(Into::into) }
    }

    fn choice(value: &str, name: &str) -> OptionChoice {
        OptionChoice { value: value.into(), name: name.into(), description: None, group: None }
    }

    fn select(id: &str, name: &str, category: OptionCategory, current: &str, choices: Vec<OptionChoice>) -> SessionOption {
        SessionOption {
            id: id.into(),
            name: name.into(),
            description: None,
            category,
            kind: OptionControl::Select { current: current.into(), choices },
        }
    }

    fn toggle(id: &str, name: &str, on: bool) -> SessionOption {
        SessionOption {
            id: id.into(),
            name: name.into(),
            description: None,
            category: OptionCategory::Other { name: "speed".into() },
            kind: OptionControl::Toggle { on },
        }
    }

    fn mode() -> SessionOption {
        select(
            "mode",
            "Mode",
            OptionCategory::Mode,
            "auto",
            vec![choice("manual", "Manual"), choice("auto", "Auto"), choice("accept", "Accept edits"), choice("plan", "Plan")],
        )
    }

    fn model() -> SessionOption {
        select("model", "Model", OptionCategory::Model, "opus", vec![choice("opus", "Opus"), choice("sonnet", "Sonnet")])
    }

    fn agent() -> Vec<SlashCommand> {
        vec![
            command("compact", "Free up context", Some("<instructions>")),
            command("context", "Show context usage", None),
            command("review", "Review the changes", None),
        ]
    }

    fn labels(completion: &Option<Completion>) -> Vec<&str> {
        completion.as_ref().map(|c| c.items.iter().map(|i| i.label.as_str()).collect()).unwrap_or_default()
    }

    fn at_end(text: &str) -> Option<Completion> {
        completion(text, text.len(), &agent(), &[mode(), model()])
    }

    #[test]
    fn a_command_text_starts_at_its_slash() {
        assert_eq!(command_text("/compact"), Some("/compact"));
        for text in [" /compact", "\t/compact", "\n\n  /compact"] {
            assert_eq!(command_text(text), Some("/compact"), "{text:?}");
        }
        assert_eq!(command_text("  indented code"), None, "left as typed");
        assert_eq!(command_text("see src/main.rs"), None);
        assert_eq!(command_text(""), None);
    }

    #[test]
    fn whitespace_before_the_slash_gives_the_same_list() {
        let plain = at_end("/co");
        for text in [" /co", "\t/co", "\n /co"] {
            let spaced = at_end(text).expect("a list");
            assert_eq!(spaced.items, plain.as_ref().unwrap().items, "{text:?}");
            assert_eq!(spaced.query, "co");
        }
        assert_eq!(labels(&plain), ["/compact", "/context"]);
    }

    #[test]
    fn a_slash_alone_lists_every_command_the_agents_first() {
        assert_eq!(labels(&at_end("/")), ["/compact", "/context", "/review", "/mode", "/model"]);
        let list = at_end("/").unwrap();
        assert_eq!(list.highlighted, 0);
        assert_eq!(list.items[0].hint.as_deref(), Some("<instructions>"));
        assert_eq!(list.items[3].description.as_deref(), Some("Mode: Auto"));
        assert_eq!(list.items[3].hint.as_deref(), Some("<choice>"));
    }

    #[test]
    fn prefix_matches_come_before_fuzzy_ones() {
        // `cox` is a prefix of none; `context` matches it fuzzily.
        assert_eq!(labels(&at_end("/cox")), ["/context"]);
        // `re` starts `review`, and the long names' `brindle:` matches
        // nothing by itself.
        assert_eq!(labels(&at_end("/re")), ["/review"]);
        // `mo` starts both options' names; `compact` matches `mo`? no.
        assert_eq!(labels(&at_end("/mo")), ["/mode", "/model"]);
        // `ct`: no prefix; fuzzy in compact, context.
        assert_eq!(labels(&at_end("/ct")), ["/compact", "/context"]);
        // A prefix match of an option comes before a fuzzy agent one.
        let commands = vec![command("xmodel", "", None)];
        let list = completion("/mod", 4, &commands, &[model()]);
        assert_eq!(labels(&list), ["/model", "/xmodel"]);
    }

    #[test]
    fn no_match_and_other_texts_give_no_list() {
        assert!(at_end("/zzz").is_none());
        assert!(at_end("see src/main.rs").is_none());
        assert!(at_end("/compact keep the plan").is_none(), "past an agent command's word");
        assert!(at_end("first line\n/compact").is_none(), "a second line starting with a slash");
        assert!(completion("/co", 0, &agent(), &[]).is_none(), "the cursor before the slash");
        assert!(completion("/co x", 5, &agent(), &[]).is_none(), "the cursor past the first word");
        assert!(completion("/co x", 2, &agent(), &[]).is_some(), "the cursor in the first word");
    }

    #[test]
    fn naming_options_commands() {
        assert_eq!(command_name("Mode"), "mode");
        assert_eq!(command_name("Reasoning Effort"), "reasoning-effort");
        assert_eq!(command_name("  Fast  mode!! "), "fast-mode");
        assert_eq!(command_name("Thought_Level (beta)"), "thought-level-beta");
        assert_eq!(command_name("???"), "");
        let options = vec![
            select("a", "Model", OptionCategory::Model, "", vec![]),
            select("b", "model", OptionCategory::ModelConfig, "", vec![]),
            select("c", "!!", OptionCategory::Other { name: "x".into() }, "", vec![]),
            select("d", "MODEL", OptionCategory::Other { name: "x".into() }, "", vec![]),
        ];
        let commands = option_commands(&options, &[command("model-2", "", None)]);
        let names: Vec<(&str, &str, bool)> =
            commands.iter().map(|c| (c.option.as_str(), c.name.as_str(), c.shadowed)).collect();
        assert_eq!(names, [("a", "model", false), ("b", "model-2", true), ("d", "model-3", false)]);
        assert_eq!(commands[1].long, "brindle:model-2");
    }

    #[test]
    fn an_agent_command_shadows_the_short_name_and_the_long_one_shows() {
        let commands = vec![command("model", "Pick a model", None), command("compact", "", None)];
        let list = completion("/mo", 3, &commands, &[mode(), model()]);
        assert_eq!(labels(&list), ["/model", "/mode", "/brindle:model"]);
        // Listed once: the unshadowed `mode` under its short name.
        let list = completion("/brindle:", 9, &commands, &[mode(), model()]);
        assert_eq!(labels(&list), ["/brindle:mode", "/brindle:model"]);
        let list = completion("/brindle:mo", 11, &agent(), &[mode(), model()]);
        assert_eq!(labels(&list), ["/brindle:mode", "/brindle:model"], "the long names when typed");
    }

    #[test]
    fn the_second_stage_lists_the_options_choices() {
        let list = at_end("/mode ").expect("the choices");
        assert_eq!(list.stage, Stage::Choices { command: "mode".into(), option: "mode".into() });
        assert_eq!(labels(&Some(list.clone())), ["Manual", "Auto", "Accept edits", "Plan"]);
        assert_eq!(list.highlighted, 1, "the current one");
        let list = at_end("/model so").unwrap();
        assert_eq!(labels(&Some(list.clone())), ["Sonnet"]);
        assert_eq!((list.query.as_str(), list.highlighted, list.range.clone()), ("so", 0, 7..9));
        // By value, ignoring case; a name with a space.
        assert_eq!(labels(&at_end("/model OPUS")), ["Opus"]);
        assert_eq!(labels(&at_end("/mode accept e")), ["Accept edits"]);
        // After the long name too, whatever the agent offers.
        let shadowing = vec![command("model", "", None)];
        let list = completion("/brindle:model s", 16, &shadowing, &[model()]).unwrap();
        assert_eq!(list.stage, Stage::Choices { command: "brindle:model".into(), option: "model".into() });
        assert!(completion("/model s", 8, &shadowing, &[model()]).is_none(), "the agent's command takes no choices");
        assert!(at_end("/model gpt").is_none());
        assert!(at_end("/mode plan\nthen").is_none(), "more lines");
        assert!(at_end("/compact ").is_none(), "an agent's command");
        let fast = [toggle("fast", "Fast mode", false)];
        let list = completion("/fast-mode ", 11, &[], &fast).unwrap();
        assert_eq!(labels(&Some(list.clone())), ["on", "off"]);
        assert_eq!(list.highlighted, 1, "off is current");
    }

    #[test]
    fn each_key_does_its_action() {
        let commands = Stage::Commands;
        let choices = Stage::Choices { command: "mode".into(), option: "mode".into() };
        for stage in [&commands, &choices] {
            assert_eq!(completion_key(CompletionKey::Up, stage), CompletionAction::Previous);
            assert_eq!(completion_key(CompletionKey::Down, stage), CompletionAction::Next);
            assert_eq!(completion_key(CompletionKey::Escape, stage), CompletionAction::Dismiss);
        }
        assert_eq!(completion_key(CompletionKey::Enter, &commands), CompletionAction::Accept);
        assert_eq!(completion_key(CompletionKey::Tab, &commands), CompletionAction::Accept);
        assert_eq!(completion_key(CompletionKey::Tab, &choices), CompletionAction::Write);
        assert_eq!(completion_key(CompletionKey::Enter, &choices), CompletionAction::Set);
        assert_eq!((moved(0, 3, true), moved(2, 3, true), moved(0, 3, false), moved(1, 3, false)), (1, 0, 2, 0));
        assert_eq!(moved(0, 0, true), 0);
    }

    #[test]
    fn accepting_writes_the_name_and_a_space() {
        let list = at_end("/co").unwrap();
        assert_eq!(accept("/co", &list, 1), Some(("/context ".to_string(), 9)));
        let list = at_end("  /co").unwrap();
        assert_eq!(accept("  /co", &list, 0), Some(("  /compact ".to_string(), 11)));
        let text = "/co keep";
        let list = completion(text, 3, &agent(), &[]).unwrap();
        assert_eq!(accept(text, &list, 0), Some(("/compact keep".to_string(), 9)), "the space kept, not doubled");
        let list = at_end("/model s").unwrap();
        assert_eq!(accept("/model s", &list, 0), Some(("/model Sonnet".to_string(), 13)));
        assert_eq!(accept("/model s", &list, 5), None);
    }

    #[test]
    fn local_commands() {
        let options = [mode(), model(), toggle("fast", "Fast mode", false)];
        let local = |text: &str| local_command(text, &options, &agent());
        let set = |option: &str, name: &str, value: OptionValue| {
            Some(Local::Set { option: option.into(), name: name.into(), value })
        };
        let choice = |value: &str| OptionValue::Choice { value: value.into() };
        assert_eq!(local("/mode plan"), set("mode", "Mode", choice("plan")));
        assert_eq!(local(" /mode plan"), set("mode", "Mode", choice("plan")), "after leading whitespace");
        assert_eq!(local("/MODEL sonnet"), None, "a command name is lower case");
        assert_eq!(local("/model SONNET"), set("model", "Model", choice("sonnet")));
        assert_eq!(local("/model so"), set("model", "Model", choice("sonnet")));
        assert_eq!(local("/mode accept edits"), set("mode", "Mode", choice("accept")));
        assert_eq!(local("/model"), Some(Local::Choose { option: "model".into() }));
        assert_eq!(local("/model   \n"), Some(Local::Choose { option: "model".into() }), "a trailing line break");
        assert_eq!(local("/fast-mode on"), set("fast", "Fast mode", OptionValue::Toggle { on: true }));
        assert_eq!(local("/fast-mode off"), set("fast", "Fast mode", OptionValue::Toggle { on: false }));
        assert_eq!(local("/fast-mode"), set("fast", "Fast mode", OptionValue::Toggle { on: true }), "flipped");
        let none = local("/model gpt").unwrap();
        assert_eq!(none, Local::NoSuchChoice { option: "Model".into(), text: "gpt".into() });
        assert_eq!(none.reason().as_deref(), Some("Model has no choice \"gpt\""));
        assert_eq!(local("/mode plan\nthen refactor it"), None, "more lines");
        assert_eq!(local("/compact"), None, "the agent's");
        assert_eq!(local("mode plan"), None);
        assert_eq!(local("/brindle:mode plan"), set("mode", "Mode", choice("plan")));
        let shadowing = [command("model", "", None)];
        assert_eq!(local_command("/model sonnet", &options, &shadowing), None, "the agent's command runs");
        assert_eq!(local_command("/brindle:model so", &options, &shadowing), set("model", "Model", choice("sonnet")));
        assert_eq!(local_command("/mode plan", &[], &[]), None, "no options, no option commands");
    }

    #[test]
    fn option_items_come_in_their_categories_order() {
        let effort = select("effort", "Effort", OptionCategory::ThoughtLevel, "medium", vec![choice("medium", "Medium")]);
        let config = select("ctx", "Context", OptionCategory::ModelConfig, "1m", vec![]);
        let other = toggle("fast", "Fast mode", true);
        let options = [other, config, effort, model(), mode()];
        let items: Vec<String> = option_items(&options).into_iter().map(|item| item.label).collect();
        assert_eq!(items, ["Auto", "Opus", "Medium", "1m", "Fast mode on"], "a current value not among the choices");
        assert!(option_items(&[]).is_empty());
    }

    #[test]
    fn next_mode_wraps_and_needs_two_choices() {
        let value = |value: &str| OptionValue::Choice { value: value.into() };
        assert_eq!(next_mode(&[model(), mode()]), Some(("mode".into(), value("accept"))));
        let two = vec![choice("manual", "Manual"), choice("plan", "Plan")];
        let last = select("mode", "Mode", OptionCategory::Mode, "plan", two);
        assert_eq!(next_mode(&[last]), Some(("mode".into(), value("manual"))), "wraps");
        let unknown = select("mode", "Mode", OptionCategory::Mode, "gone", vec![choice("a", "A"), choice("b", "B")]);
        assert_eq!(next_mode(&[unknown]), Some(("mode".into(), value("a"))));
        assert_eq!(next_mode(&[model()]), None, "no mode option");
        let one = select("mode", "Mode", OptionCategory::Mode, "a", vec![choice("a", "A")]);
        assert_eq!(next_mode(&[one]), None, "one choice");
    }

    #[test]
    fn the_options_palette_lists_options_then_choices() {
        let mut grouped = model();
        grouped.kind = OptionControl::Select {
            current: "sonnet".into(),
            choices: vec![
                OptionChoice { value: "auto".into(), name: "Auto".into(), description: None, group: None },
                OptionChoice { value: "opus".into(), name: "Opus".into(), description: Some("Best".into()), group: Some("Anthropic".into()) },
                OptionChoice { value: "gpt".into(), name: "GPT".into(), description: None, group: Some("OpenAI".into()) },
                OptionChoice { value: "sonnet".into(), name: "Sonnet".into(), description: None, group: Some("Anthropic".into()) },
            ],
        };
        let options = [toggle("fast", "Fast mode", false), grouped, mode()];
        let all = options_palette(&options, None).unwrap();
        let rows: Vec<&str> = all.groups[0].rows.iter().map(|row| row.label.as_ref()).collect();
        assert_eq!(rows, ["Mode: Auto", "Model: Sonnet", "Fast mode: off"]);
        assert_eq!(all.picks[1], OptionPick::Open { option: "model".into() });
        assert_eq!(
            all.picks[2],
            OptionPick::Set { option: "fast".into(), name: "Fast mode".into(), value: OptionValue::Toggle { on: true } }
        );
        let model = options_palette(&options, Some("model")).unwrap();
        let headings: Vec<Option<&str>> = model.groups.iter().map(|g| g.heading.as_ref().map(|h| h.as_ref())).collect();
        assert_eq!(headings, [None, Some("Anthropic"), Some("OpenAI")]);
        let rows: Vec<(&str, Option<&str>)> = model
            .groups
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| (r.label.as_ref(), r.detail.as_ref().map(|d| d.as_ref()))))
            .collect();
        assert_eq!(rows, [("Auto", None), ("Opus", Some("Best")), ("Sonnet", Some("current")), ("GPT", None)]);
        let value = |v: &str| OptionValue::Choice { value: v.into() };
        let picks: Vec<&OptionValue> = model
            .picks
            .iter()
            .map(|pick| match pick {
                OptionPick::Set { value, .. } => value,
                OptionPick::Open { .. } => unreachable!(),
            })
            .collect();
        assert_eq!(picks, [&value("auto"), &value("opus"), &value("sonnet"), &value("gpt")], "the rows' order");
        assert!(options_palette(&[], None).is_none());
        assert!(options_palette(&options, Some("gone")).is_none());
    }

    #[test]
    fn the_dump_lines() {
        let list = at_end("/co").unwrap();
        assert_eq!(
            list.lines(),
            "--- completion query=\"co\" highlighted=0\n/compact  <instructions>  Free up context\n/context    Show context usage\n"
        );
        let list = at_end("/model \"s").map(|c| c.lines());
        assert_eq!(list, None);
        let list = at_end("/model so").unwrap();
        assert_eq!(list.lines(), "--- completion command=\"model\" query=\"so\" highlighted=0\nSonnet\n");
        let escaped = Completion { query: "a\"b".into(), ..list };
        assert!(escaped.lines().starts_with("--- completion command=\"model\" query=\"a\\\"b\""));
    }

    #[test]
    fn the_cursor_in_a_command_word() {
        assert!(in_command("/co", 3) && in_command(" /co x", 6));
        assert!(!in_command("/co", 0) && !in_command("text", 2) && !in_command("/co\nx", 5));
    }
}
