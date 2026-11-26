//! A local tab's binary split tree of zones, normalized geometry, and
//! native GPUI zone chrome.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use brindle_ui::{
    ActiveTheme as _, Button, ButtonSize, ButtonVariant, Card, CardColors, CardHeader, ContextMenuTarget as _, EdgeBar,
    Icon, Loader, Modality, ThemeScope, Tokens, Tooltip, TooltipFor as _, modality, shows_ring,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render,
    SharedString, Size, Styled, Subscription, Window, div, px, size,
};
use serde::{Deserialize, Serialize};

use crate::actions::*;
use crate::agent::close::{BusyAgent, ClosePlan, Closing, close_plan};
use crate::agent::login::{self, TerminalLogin};
use crate::agent::model::LoginZoneEnd;
use crate::agent::{Agent, LoginZone};
use crate::agent::spawn::OpenedBy;
use crate::agent::view::{AgentView, ConfirmLink};
use brindle_ui::FocusStops as _;
use gpui::Focusable as _;
use crate::config::{Config, Profile, ProfileKey, ProfileKind};
use crate::remote::Host;
use crate::settings::Settings;
use crate::terminal::MIN_COLS;
use crate::terminal::context::{Badge, BadgeKind};
use crate::terminal::process::LOCAL_PATH_STYLE;
use crate::terminal::progress::{self, Progress};
use crate::terminal::status::State;
use crate::terminal::{Terminal, TerminalEvent};
use crate::terminal_element::cell_metrics;
use crate::terminal_view::{GridLayout, LinkFailed, TerminalView};
use crate::theme::Theme;
use crate::workspace::{program_title, shortcut, terminal_automatic_title};
use crate::workspace::mark::{Mark, mark};
use crate::motion::{self, Motion};

/// Height of a zone card's header bar, shown when a tab has 2+ zones, its
/// bottom line included (`brindle_ui::CARD_HEADER_HEIGHT`).
pub const ZONE_HEADER_HEIGHT: f32 = 28.0;
/// The gap between a tab's zone cards, and the margin round them; it
/// replaces the dividers, and the focus ring is drawn in it.
pub const ZONE_GAP: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneId(pub u64);

/// Which way `FocusZone*` moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitAxis {
    /// Left and right halves (`Split Zone Right`).
    Horizontal,
    /// Top and bottom halves (`Split Zone Down`).
    Vertical,
}

/// Normalized rectangle in `[0, 1] x [0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl UnitRect {
    pub const FULL: Self = Self { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    pub fn split(self, axis: SplitAxis) -> (Self, Self) {
        match axis {
            SplitAxis::Horizontal => {
                let half = self.w * 0.5;
                (
                    Self { x: self.x, y: self.y, w: half, h: self.h },
                    Self { x: self.x + half, y: self.y, w: half, h: self.h },
                )
            }
            SplitAxis::Vertical => {
                let half = self.h * 0.5;
                (
                    Self { x: self.x, y: self.y, w: self.w, h: half },
                    Self { x: self.x, y: self.y + half, w: self.w, h: half },
                )
            }
        }
    }

    fn center_x(self) -> f64 {
        self.x + self.w * 0.5
    }

    fn center_y(self) -> f64 {
        self.y + self.h * 0.5
    }
}

/// Binary split tree of zones in a local tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneTree {
    Leaf(ZoneId),
    Split {
        axis: SplitAxis,
        first: Box<ZoneTree>,
        second: Box<ZoneTree>,
    },
}

impl ZoneTree {
    pub fn new(id: ZoneId) -> Self {
        Self::Leaf(id)
    }

    /// Replaces `Leaf(target)` with a `Split` holding `target` in `first`
    /// and `new_id` in `second`. Returns whether `target` was found.
    pub fn split(&mut self, target: ZoneId, axis: SplitAxis, new_id: ZoneId) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new_id)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, axis, new_id) || second.split(target, axis, new_id)
            }
        }
    }

    /// Removes `Leaf(target)`, promoting its sibling subtree in place of the
    /// parent `Split`. Returns `Some(promoted_leaves)` when removed from a
    /// split, or `None` if `target` was not found (or was the root leaf).
    pub fn remove(&mut self, target: ZoneId) -> Option<Vec<ZoneId>> {
        match self {
            Self::Leaf(_) => None,
            Self::Split { first, second, .. } => {
                if matches!(**first, Self::Leaf(id) if id == target) {
                    let promoted = second.leaves();
                    *self = (**second).clone();
                    return Some(promoted);
                }
                if matches!(**second, Self::Leaf(id) if id == target) {
                    let promoted = first.leaves();
                    *self = (**first).clone();
                    return Some(promoted);
                }
                first.remove(target).or_else(|| second.remove(target))
            }
        }
    }

    /// All leaf IDs in tree order (first before second).
    pub fn leaves(&self) -> Vec<ZoneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<ZoneId>) {
        match self {
            Self::Leaf(id) => out.push(*id),
            Self::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Normalized `[0, 1] x [0, 1]` rectangles for every leaf in tree order.
    pub fn rects(&self) -> Vec<(ZoneId, UnitRect)> {
        let mut out = Vec::new();
        self.collect_rects(UnitRect::FULL, &mut out);
        out
    }

    fn collect_rects(&self, rect: UnitRect, out: &mut Vec<(ZoneId, UnitRect)>) {
        match self {
            Self::Leaf(id) => out.push((*id, rect)),
            Self::Split { axis, first, second } => {
                let (r1, r2) = rect.split(*axis);
                first.collect_rects(r1, out);
                second.collect_rects(r2, out);
            }
        }
    }

    /// Finds the leaf to focus when moving from `active` in `direction`:
    /// candidates must overlap on the perpendicular axis and lie in
    /// `direction`, ranked by nearest primary edge, then closest
    /// perpendicular center, then tree order.
    pub fn focus_target(&self, active: ZoneId, direction: Direction) -> Option<ZoneId> {
        const EPS: f64 = 1e-9;
        let rects = self.rects();
        let (_, cur) = rects.iter().copied().find(|(id, _)| *id == active)?;

        let mut best: Option<(ZoneId, f64, f64)> = None;
        for (id, cand) in rects {
            if id == active {
                continue;
            }
            let (primary_dist, perp_overlap, perp_dist) = match direction {
                Direction::Left => {
                    let dist = cur.x - (cand.x + cand.w);
                    let overlap = (cur.y + cur.h).min(cand.y + cand.h) - cur.y.max(cand.y);
                    (dist, overlap, (cur.center_y() - cand.center_y()).abs())
                }
                Direction::Right => {
                    let dist = cand.x - (cur.x + cur.w);
                    let overlap = (cur.y + cur.h).min(cand.y + cand.h) - cur.y.max(cand.y);
                    (dist, overlap, (cur.center_y() - cand.center_y()).abs())
                }
                Direction::Up => {
                    let dist = cur.y - (cand.y + cand.h);
                    let overlap = (cur.x + cur.w).min(cand.x + cand.w) - cur.x.max(cand.x);
                    (dist, overlap, (cur.center_x() - cand.center_x()).abs())
                }
                Direction::Down => {
                    let dist = cand.y - (cur.y + cur.h);
                    let overlap = (cur.x + cur.w).min(cand.x + cand.w) - cur.x.max(cand.x);
                    (dist, overlap, (cur.center_x() - cand.center_x()).abs())
                }
            };
            if primary_dist < -EPS || perp_overlap <= EPS {
                continue;
            }
            let replace = match best {
                None => true,
                Some((_, best_primary, best_perp)) => {
                    if (primary_dist - best_primary).abs() > EPS {
                        primary_dist < best_primary
                    } else if (perp_dist - best_perp).abs() > EPS {
                        perp_dist < best_perp
                    } else {
                        // Earlier in tree order wins ties.
                        false
                    }
                }
            };
            if replace {
                best = Some((id, primary_dist, perp_dist));
            }
        }
        best.map(|(id, _, _)| id)
    }
}

/// Whether a zone card of outer pixel size `zone_size` (its border, and its
/// header bar when `has_header` is true, included) can be split in half
/// along `axis` while fitting, in each resulting card, a minimum
/// [`MIN_COLS`] x 2 terminal grid with its padding, the card's border and a header bar, with
/// the [`ZONE_GAP`] between the two cards.
pub fn can_split_zone(
    zone_size: Size<Pixels>,
    has_header: bool,
    axis: SplitAxis,
    padding: Pixels,
    cell_width: Pixels,
    line_height: Pixels,
) -> bool {
    let border = f32::from(brindle_ui::CARD_BORDER) * 2.0;
    let min_term_w = f32::from(cell_width) * f32::from(MIN_COLS) + f32::from(padding) * 2.0;
    let min_term_h = f32::from(line_height) * 2.0 + f32::from(padding) * 2.0;
    let min_zone_w = min_term_w + border;
    // Every card of a split tab has a header, whether or not this one has
    // yet: its height is the card's either way.
    let min_zone_h = min_term_h + border + ZONE_HEADER_HEIGHT;

    let w = f32::from(zone_size.width);
    let h = f32::from(zone_size.height);
    let _ = has_header;

    match axis {
        SplitAxis::Horizontal => {
            let child_w = (w - ZONE_GAP) * 0.5;
            child_w >= min_zone_w && h >= min_zone_h
        }
        SplitAxis::Vertical => {
            let child_h = (h - ZONE_GAP) * 0.5;
            w >= min_zone_w && child_h >= min_zone_h
        }
    }
}

/// Which profile a split that asks for a kind runs (OpenSpec change
/// `zones-mixed-tabs`, decision 2), as the config has it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitChoice {
    /// This profile.
    Run(ProfileKey),
    /// One of these, which a palette asks for, in the config's order.
    Ask(Vec<ProfileKey>),
    /// None: nothing is split.
    Refused(SplitRefusal),
}

/// Why a split runs no profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRefusal {
    /// The config has no profile of the kind asked for.
    NoProfileOfKind(ProfileKind),
}

impl SplitRefusal {
    /// What the window's notice line says.
    pub fn text(self) -> &'static str {
        match self {
            Self::NoProfileOfKind(ProfileKind::Terminal) => "no terminal profile to split with",
            Self::NoProfileOfKind(ProfileKind::Agent) => "no agent profile to split with",
        }
    }
}

/// What a split starts beside the zone it splits: a zone of a kind, its
/// profile chosen by [`split_choice`], or a zone of the profile a palette
/// chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitWith {
    Kind(ProfileKind),
    Profile(ProfileKey),
}

/// Which profile a split asking for `want` runs, `zone` being the index in
/// `config` of the split zone's own profile (`None`: gone from the config):
/// the zone's own profile when it is of the kind; else the default profile
/// when it is; else the only profile of the kind; several: ask which, in
/// the config's order; none: refused.
pub fn split_choice(want: ProfileKind, zone: Option<usize>, config: &Config) -> SplitChoice {
    let of_kind = |ix: &usize| config.profiles.get(*ix).is_some_and(|p| p.kind() == want);
    let key = |ix: usize| config.profile_key(ix);
    if let Some(key) = zone.filter(of_kind).and_then(key) {
        return SplitChoice::Run(key);
    }
    if let Some(key) = Some(config.default_profile_index()).filter(of_kind).and_then(key) {
        return SplitChoice::Run(key);
    }
    let mut kind: Vec<ProfileKey> = (0..config.profiles.len()).filter(of_kind).filter_map(key).collect();
    match kind.len() {
        0 => SplitChoice::Refused(SplitRefusal::NoProfileOfKind(want)),
        1 => SplitChoice::Run(kind.remove(0)),
        _ => SplitChoice::Ask(kind),
    }
}

/// The profiles `Split … With Profile…` lists: every one, of either kind
/// and any host, the default first, then the others in the config's order.
pub fn split_profile_choices(config: &Config) -> Vec<ProfileKey> {
    let default = config.default_profile_index();
    std::iter::once(default)
        .chain((0..config.profiles.len()).filter(|&ix| ix != default))
        .filter_map(|ix| config.profile_key(ix))
        .collect()
}

/// Where a zone of `profile` started by a split of a zone of `split`
/// running on `split_host` runs: on the split zone's endpoint when the
/// profile is its own (names match ignoring case, as profile lookups do),
/// even when the profile's target has moved since; any other profile
/// where it says.
pub fn split_host(split: &Profile, split_host: &Host, profile: &Profile) -> Host {
    if profile.name.eq_ignore_ascii_case(&split.name) { split_host.clone() } else { Host::of(profile) }
}

/// The directory a split's new zone starts in: the split zone's, when both
/// run on one endpoint (`same_host`), whatever the new profile's own
/// directory; else none, and the new zone's start takes its profile's
/// directory, else the home directory there. A directory never crosses
/// endpoints.
pub fn split_cwd(split_dir: Option<PathBuf>, same_host: bool) -> Option<PathBuf> {
    split_dir.filter(|_| same_host)
}

/// What a zone header's menu enables its split items by: whether the zone
/// can be split along each axis (it isn't a terminal login's zone, and
/// [`can_split_zone`] for its current size), and whether the config has a
/// profile of each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuSplits {
    pub right: bool,
    pub down: bool,
    pub terminal: bool,
    pub agent: bool,
}

/// A zone header's menu: the terminal, agent and choose-a-profile splits,
/// each right then down, a separator and Close Zone. A split item is
/// enabled only when the zone can be split along its axis and, for a
/// kind's items, when the config has a profile of that kind (the two that
/// ask take any); Close Zone always is.
pub fn zone_menu_rows(splits: MenuSplits) -> Vec<ActionRow> {
    let MenuSplits { right, down, terminal, agent } = splits;
    let any = terminal || agent;
    vec![
        ActionRow::item(SplitTerminalRight, right && terminal),
        ActionRow::item(SplitTerminalDown, down && terminal),
        ActionRow::item(SplitAgentRight, right && agent),
        ActionRow::item(SplitAgentDown, down && agent),
        ActionRow::item(SplitRightWithProfile, right && any),
        ActionRow::item(SplitDownWithProfile, down && any),
        ActionRow::Separator,
        ActionRow::item(CloseZone, true),
    ]
}

/// The split-right and split-down actions of a zone of `kind`, which its
/// header's buttons run and name in their tooltips: a zone splits as its
/// own kind.
pub fn split_actions(kind: ProfileKind) -> (Box<dyn Action>, Box<dyn Action>) {
    match kind {
        ProfileKind::Terminal => (Box::new(SplitTerminalRight), Box::new(SplitTerminalDown)),
        ProfileKind::Agent => (Box::new(SplitAgentRight), Box::new(SplitAgentDown)),
    }
}

/// A zone asks its tab's window for its menu: on a secondary press on its
/// header, or `OpenZoneMenu`. The zone has focus by then, so the items,
/// which run the zone actions, act on it.
#[derive(Clone)]
pub struct ZoneMenuRequest {
    pub rows: Vec<ActionRow>,
    /// Where the menu's corner goes, in window coordinates: the pointer, or
    /// below the header's start.
    pub at: Point<Pixels>,
    /// The secondary press that asked, whose release ends the gesture.
    pub press: Option<MouseDownEvent>,
    /// The token set the menu is drawn in: the zone's for an agent zone,
    /// which is drawn in a theme scope, `None` (the window's) for a
    /// terminal zone ([`ZoneTreeView::scope_of`]).
    pub tokens: Option<Tokens>,
}

/// A zone's close asks first (design.md decision 12): its agent is busy.
/// The tab's window opens the confirmation, and closes the zone
/// ([`ZoneTreeView::close_zone_by_id`]) once the user confirms.
#[derive(Clone, Debug)]
pub struct CloseZoneAsk {
    pub zone: ZoneId,
    pub busy: Vec<BusyAgent>,
}

/// Formats `--dump-screen-after` zone lines when a tab has 2 or more zones,
/// or returns an empty string for a single-zone tab.
pub fn describe_zone_tree(tree: &ZoneTree, active: ZoneId) -> String {
    let rects = tree.rects();
    if rects.len() < 2 {
        return String::new();
    }
    let mut out = format!("--- zones count={} active={}\n", rects.len(), active.0);
    for (id, r) in rects {
        let active_suffix = if id == active { " active" } else { "" };
        out.push_str(&format!(
            "--- zone id={} rect={},{},{},{}{}\n",
            id.0,
            fmt_unit(r.x),
            fmt_unit(r.y),
            fmt_unit(r.w),
            fmt_unit(r.h),
            active_suffix,
        ));
    }
    out
}

/// How a context badge is drawn on a tab face or a zone header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeDraw {
    /// A warning icon, its label in the tooltip.
    Icon,
    /// The label as text.
    Text,
}

impl BadgeDraw {
    /// The user's call: "a warning icon, follow the theme's color, no name
    /// needed" for an elevation or privilege change; the other contexts
    /// (a host, a container, a VM) keep their name.
    pub fn of(kind: BadgeKind) -> Self {
        match kind {
            BadgeKind::Privileged => BadgeDraw::Icon,
            BadgeKind::Elsewhere => BadgeDraw::Text,
        }
    }

    fn name(self) -> &'static str {
        match self {
            BadgeDraw::Icon => "icon",
            BadgeDraw::Text => "text",
        }
    }
}

/// The `brindle-ui` badge a tab face or zone header shows for `badge`, in
/// `theme` (the face's or header's own, `Theme::badge_color`): a warning
/// icon whose tooltip names the target user ("Running as root"), or the
/// label ([`BadgeDraw::of`]).
pub fn ui_badge(badge: &Badge, theme: &Theme) -> brindle_ui::Badge {
    let color = theme.badge_color(badge.kind());
    match BadgeDraw::of(badge.kind()) {
        BadgeDraw::Icon => {
            brindle_ui::Badge::icon(Icon::TriangleAlert, color).tooltip(format!("Running as {}", badge.label()))
        }
        BadgeDraw::Text => brindle_ui::Badge::new(badge.label().to_owned(), color),
    }
}

/// A badge for the dump: `kind=… label="…" draw=icon|text`, the label kept
/// as data even when only an icon is drawn.
pub fn describe_badge(badge: &Badge) -> String {
    format!("{} draw={}", badge.describe(), BadgeDraw::of(badge.kind()).name())
}

/// Formats `--dump-screen-after` zone badge lines when a tab has 2 or more
/// zones (`--- badge zone=ID kind=… label="…" draw=…`, the active zone
/// first, then by id), one per zone whose header shows a badge, or returns
/// an empty string for a single-zone tab, which has no headers.
pub fn describe_zone_badges<'a>(zones: impl IntoIterator<Item = (ZoneId, Option<&'a Badge>)>, active: ZoneId) -> String {
    let mut zones: Vec<(ZoneId, Option<&Badge>)> = zones.into_iter().collect();
    if zones.len() < 2 {
        return String::new();
    }
    zones.sort_by_key(|(id, _)| (*id != active, id.0));
    zones
        .into_iter()
        .filter_map(|(id, badge)| Some(format!("--- badge zone={} {}\n", id.0, describe_badge(badge?))))
        .collect()
}

/// Formats `--dump-screen-after` zone progress lines when a tab has 2 or
/// more zones (`--- progress zone=ID state=…[ percent=N]`, the active zone
/// first, then by id), one per zone with progress, or returns an empty
/// string for a single-zone tab, whose tab line says the same.
pub fn describe_zone_progress(zones: impl IntoIterator<Item = (ZoneId, Option<Progress>)>, active: ZoneId) -> String {
    let mut zones: Vec<(ZoneId, Option<Progress>)> = zones.into_iter().collect();
    if zones.len() < 2 {
        return String::new();
    }
    zones.sort_by_key(|(id, _)| (*id != active, id.0));
    zones
        .into_iter()
        .filter_map(|(id, progress)| Some(format!("--- progress zone={} {}\n", id.0, progress?.describe())))
        .collect()
}

fn fmt_unit(v: f64) -> String {
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

/// What a new zone shows, as its tab or a split starts it: a terminal, or
/// an agent session's entity, for which the tree makes the view.
pub enum ZoneStart {
    Terminal(Entity<Terminal>),
    Agent(Entity<Agent>),
}

/// The profile a zone runs, where, and how it is drawn: what a zone keeps
/// of its own (OpenSpec change `zones-mixed-tabs`, decision 1).
#[derive(Clone)]
pub struct ZoneSpec {
    /// As the config has it now; kept as last seen once it is deleted.
    pub profile: Profile,
    /// Where the zone's session runs. It never changes.
    pub host: Host,
    /// The profile's theme, or the preview of it.
    pub theme: Theme,
    pub font_size: Option<f32>,
}

impl ZoneSpec {
    /// `profile` on `host`, in the profile's theme and font size as
    /// `config` gives them.
    pub fn of(profile: Profile, host: Host, config: &Config) -> Self {
        let theme = config.profile_theme(&profile);
        let font_size = profile.font_size;
        Self { profile, host, theme, font_size }
    }
}

/// What a zone holds: a terminal, or an agent session's conversation
/// (`agent-zones`). Every question the tree and the window ask of a zone is
/// a method here, each an exhaustive `match` (`types R2`, design.md
/// decision 5).
pub enum ZoneContent {
    Terminal(Entity<TerminalView>),
    Agent(Entity<AgentView>),
}

impl ZoneContent {
    /// The zone's kind, which is what it shows: no other field says it.
    pub fn kind(&self) -> ProfileKind {
        match self {
            Self::Terminal(_) => ProfileKind::Terminal,
            Self::Agent(_) => ProfileKind::Agent,
        }
    }

    /// The terminal's view; `None` for an agent zone.
    pub fn terminal_view(&self) -> Option<&Entity<TerminalView>> {
        match self {
            Self::Terminal(view) => Some(view),
            Self::Agent(_) => None,
        }
    }

    /// The agent zone's view; `None` for a terminal zone.
    pub fn agent_view(&self) -> Option<&Entity<AgentView>> {
        match self {
            Self::Terminal(_) => None,
            Self::Agent(view) => Some(view),
        }
    }

    /// The terminal; `None` for an agent zone.
    pub fn terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        Some(self.terminal_view()?.read(cx).terminal.clone())
    }

    /// The handle focusing the zone focuses: the terminal's, or the agent
    /// zone's prompt input.
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Terminal(view) => view.read(cx).focus_handle.clone(),
            Self::Agent(view) => view.read(cx).focus_handle(cx),
        }
    }

    /// Whether keyboard focus is in the zone: in the terminal, or on one of
    /// the agent zone's stops (its prompt, its buttons).
    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        match self {
            Self::Terminal(view) => view.read(cx).focus_handle.contains_focused(window, cx),
            Self::Agent(view) => view.read(cx).stops(cx).iter().any(|stop| stop.contains_focused(window, cx)),
        }
    }

    /// Where a split of the zone, or a new tab following it, starts: the
    /// terminal's directory, or the one the agent session was started in.
    pub fn working_directory(&self, cx: &App) -> Option<PathBuf> {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.read(cx).working_directory(),
            Self::Agent(view) => view.read(cx).agent().read(cx).directory().cloned(),
        }
    }

    /// Whether session restore leaves the zone out: a terminal that only
    /// shows a message (a failed attach), or an agent zone whose session
    /// isn't open any more (ended, never started, lost or broken) or whose
    /// agent exited or was lost with its holder, though the server keeps
    /// that session for a continue ([`crate::agent::model::ChatModel::keeps_session`]).
    pub fn unrecorded(&self, cx: &App) -> bool {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.read(cx).shows_only_message(),
            Self::Agent(view) => !view.read(cx).agent().read(cx).model().keeps_session(),
        }
    }

    /// The terminal server session (id and key) the zone shows.
    #[cfg(any(target_os = "linux", windows))]
    pub fn server_session(
        &self,
        cx: &App,
    ) -> Option<(crate::remote::protocol::SessionId, Option<crate::remote::SessionKey>)> {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.read(cx).server_session(),
            Self::Agent(view) => view.read(cx).agent().read(cx).server_session(),
        }
    }

    /// The zone's title: the terminal's automatic title in a tab of
    /// `profile_name`, or the agent's name, else the profile's.
    pub fn title(&self, profile_name: &str, cx: &App) -> SharedString {
        match self {
            Self::Terminal(view) => {
                let terminal = view.read(cx).terminal.read(cx);
                // The process name can cost a read of `/proc`: only when the
                // program set no title of its own. On Windows it is what the
                // title tick found off the UI thread.
                let process_name = match program_title(terminal.title(), LOCAL_PATH_STYLE) {
                    Some(_) => None,
                    None => terminal.foreground_process_name(),
                };
                terminal_automatic_title(terminal.title(), profile_name, LOCAL_PATH_STYLE, process_name).into()
            }
            Self::Agent(view) => view.read(cx).agent().read(cx).title().into(),
        }
    }

    /// The status the zone's dot shows, its progress and its context badge:
    /// a terminal's program status, or the agent zone's shown status with
    /// no progress or badge. `idle` shows no dot, as a program's `idle`
    /// record shows none.
    pub fn status(&self, cx: &App) -> (Option<State>, Option<Progress>, Option<Badge>) {
        match self {
            Self::Terminal(view) => {
                let terminal = view.read(cx).terminal.read(cx);
                (terminal.status().summary(), terminal.progress(), terminal.badge().cloned())
            }
            Self::Agent(view) => {
                let status = view.read(cx).agent().read(cx).status().filter(|&state| state != State::Idle);
                (status, None, None)
            }
        }
    }

    /// Takes the tab's profile's theme and font size, and an agent zone its
    /// name (a title that is the profile's follows a rename).
    pub fn set_profile(&self, name: &str, theme: Theme, font_size: Option<f32>, cx: &mut App) {
        match self {
            Self::Terminal(view) => view.update(cx, |view, cx| {
                view.font_size_override = font_size;
                view.terminal.update(cx, |t, _| t.theme = theme);
            }),
            Self::Agent(view) => {
                view.update(cx, |view, cx| view.set_profile(theme, font_size, cx));
                let agent = view.read(cx).agent().clone();
                agent.update(cx, |agent, cx| agent.set_profile_name(name, cx));
            }
        }
    }

    /// The user closed the zone: its program or agent session ends.
    pub fn close(&self, cx: &mut App) {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.clone().update(cx, |t, _| t.close()),
            Self::Agent(view) => view.read(cx).agent().clone().update(cx, |agent, _| agent.close()),
        }
    }

    /// Brindle is quitting: the session is left as the zone's kind says,
    /// a kept one detached with its tab's `layout`.
    pub fn quit_with(&self, layout: Option<serde_json::Value>, cx: &mut App) {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.clone().update(cx, |t, _| t.quit_with(layout)),
            Self::Agent(view) => view.read(cx).agent().clone().update(cx, |agent, _| agent.quit_with(layout)),
        }
    }

    /// Keeps the session's copy of its tab's layout current.
    pub fn set_layout(&self, layout: Option<serde_json::Value>, cx: &App) {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.read(cx).set_layout(layout),
            Self::Agent(view) => view.read(cx).agent().read(cx).set_layout(layout),
        }
    }

    /// A restore's note: a dim line in the terminal, or a notice row above
    /// the agent zone's conversation.
    pub fn note(&self, text: &str, cx: &mut App) {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.clone().update(cx, |t, cx| t.note(text, cx)),
            Self::Agent(view) => view.read(cx).agent().clone().update(cx, |agent, cx| agent.note(text, cx)),
        }
    }

    /// The agent zone's dump block as zone `zone` (design.md decision 15),
    /// with where its conversation is scrolled; `None` for a terminal.
    pub fn describe_agent(&self, zone: ZoneId, cx: &App) -> Option<String> {
        match self {
            Self::Terminal(_) => None,
            Self::Agent(view) => {
                let view = view.read(cx);
                let at_end = view.scroller().read(cx).follow() == brindle_ui::Follow::AtEnd;
                let agent = view.agent().read(cx);
                Some(agent.model().describe(zone.0, at_end, &agent.offer(cx), &crate::terminal::links::plan_here))
            }
        }
    }

    /// What `--wait-for` matches: the terminal's screen, or the agent
    /// zone's dump lines without its id and scroll position.
    pub fn wait_text(&self, cx: &App) -> String {
        match self {
            Self::Terminal(view) => view.read(cx).terminal.read(cx).screen_text(),
            Self::Agent(view) => {
                let agent = view.read(cx).agent().read(cx);
                agent.model().wait_text(&agent.offer(cx), &crate::terminal::links::plan_here)
            }
        }
    }

    /// Where the terminal's grid was last painted; `None` for an agent
    /// zone, whose size the tree estimates from its rectangle.
    fn grid(&self, cx: &App) -> Option<GridLayout> {
        match self {
            Self::Terminal(view) => view.read(cx).layout.get(),
            Self::Agent(_) => None,
        }
    }

    /// The element the zone's card holds.
    fn element(&self) -> AnyElement {
        match self {
            Self::Terminal(view) => view.clone().into_any_element(),
            Self::Agent(view) => view.clone().into_any_element(),
        }
    }
}

pub struct Zone {
    pub content: ZoneContent,
    /// The zone's profile, as the config has it now (by name; a rename in
    /// the settings dialog follows), else as last seen. Its kind is the
    /// content's whatever the profile's is now.
    pub profile: Profile,
    /// Where its session runs, whatever the profile targets now. It never
    /// changes.
    pub host: Host,
    /// Its profile's theme (or the preview of it), for its terminal or
    /// agent view and the card round it.
    pub theme: Theme,
    /// `theme`'s zone token set (`Theme::zone_tokens`), which an agent zone
    /// is drawn in, kept beside it so `render` derives nothing.
    zone_tokens: Tokens,
    pub font_size: Option<f32>,
    pub title: SharedString,
    pub status: Option<State>,
    /// Its terminal's progress, cached for its header's bar.
    pub progress: Option<Progress>,
    /// Its terminal's context badge, cached for its header and the tab's
    /// face; both refreshed on its terminal's `Status`.
    pub badge: Option<Badge>,
    pub last_focused: u64,
    /// The agent zone whose terminal login this zone runs (OpenSpec change
    /// `agents-login`, decision 9); `None` for every other zone. Such a
    /// zone is never recorded or left detached.
    pub login_for: Option<ZoneId>,
    _subscriptions: Vec<Subscription>,
}

/// The leaves session restore and a detached layout record: every one but
/// a terminal login's zone (`logins`), which a quit ends.
pub fn recorded_leaves(leaves: Vec<ZoneId>, logins: &[ZoneId]) -> Vec<ZoneId> {
    leaves.into_iter().filter(|id| !logins.contains(id)).collect()
}

/// Where a terminal login's zone goes beside its agent zone: to the right
/// when the agent zone is wide enough to split that way, else below.
pub fn login_axis(fits_right: bool) -> SplitAxis {
    if fits_right { SplitAxis::Horizontal } else { SplitAxis::Vertical }
}

/// A split of the active zone wants a profile chosen (OpenSpec change
/// `zones-mixed-tabs`, decision 2): the window asks which of `profiles` in
/// a palette, and splits `zone` along `axis` with the one chosen
/// ([`ZoneTreeView::split_zone`] with [`SplitWith::Profile`]).
#[derive(Clone, Debug)]
pub struct SplitAsk {
    pub zone: ZoneId,
    pub axis: SplitAxis,
    pub profiles: Vec<ProfileKey>,
}

/// A split ran no profile, which the window's notice line says.
#[derive(Clone, Copy, Debug)]
pub struct SplitRefused(pub SplitRefusal);

pub struct ZoneTreeView {
    tree: ZoneTree,
    zones: HashMap<ZoneId, Zone>,
    active: ZoneId,
    next_id: u64,
    focus_serial: u64,
    shutting_down: bool,
    focus_handle: FocusHandle,
    /// The zone whose bell or notification the tree passed on last, which
    /// the window's notification names ([`ZoneTreeView::notifying_profile`]).
    notifying: Option<ZoneId>,
    /// Where the tree and each zone's header were laid out in the last
    /// frame, in window coordinates, for placing a menu opened from a key.
    /// Recorded while the frame is drawn, so without a notify.
    laid_out: Rc<RefCell<LaidOut>>,
}

/// See [`ZoneTreeView::laid_out`].
#[derive(Default)]
struct LaidOut {
    tree: Option<Bounds<Pixels>>,
    headers: HashMap<ZoneId, Bounds<Pixels>>,
}

impl EventEmitter<TerminalEvent> for ZoneTreeView {}
impl EventEmitter<ZoneMenuRequest> for ZoneTreeView {}
impl EventEmitter<CloseZoneAsk> for ZoneTreeView {}
impl EventEmitter<SplitAsk> for ZoneTreeView {}
impl EventEmitter<SplitRefused> for ZoneTreeView {}
/// A zone's link wasn't opened, passed on to the window (whichever tab the
/// zone is in: an opener can fail after its tab went to the background).
impl EventEmitter<LinkFailed> for ZoneTreeView {}
/// An agent zone's link asks before it opens, passed on to the window.
impl EventEmitter<ConfirmLink> for ZoneTreeView {}

impl ZoneTreeView {
    /// Creates a `ZoneTreeView` with a single initial zone, of `spec`.
    pub fn new(spec: ZoneSpec, start: ZoneStart, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = ZoneId(1);
        let mut this = Self::empty(ZoneTree::new(id), id, 2, cx);
        Self::unwant_on_release(cx);
        this.insert_zone(id, spec, start, window, cx);
        this
    }

    /// A tree of no zones yet, `tree` its shape.
    fn empty(tree: ZoneTree, active: ZoneId, next_id: u64, cx: &mut Context<Self>) -> Self {
        Self {
            tree,
            zones: HashMap::new(),
            active,
            next_id,
            focus_serial: 0,
            shutting_down: false,
            focus_handle: cx.focus_handle(),
            notifying: None,
            laid_out: Rc::default(),
        }
    }

    /// A closed tab can't keep the motion clock running.
    fn unwant_on_release(cx: &mut Context<Self>) {
        let id = cx.entity_id();
        cx.on_release(move |_, cx| Motion::want(id, false, cx)).detach();
    }

    /// Creates a `ZoneTreeView` from a restored split tree and the zones
    /// started for each leaf, each of its own profile on its own host.
    pub fn from_restored(
        tree: ZoneTree,
        zones: Vec<(ZoneId, ZoneSpec, ZoneStart)>,
        active: ZoneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let max_id = zones.iter().map(|(id, ..)| id.0).max().unwrap_or(1);
        let mut this = Self::empty(tree, active, max_id + 1, cx);
        Self::unwant_on_release(cx);
        for (id, spec, start) in zones {
            this.insert_zone(id, spec, start, window, cx);
        }
        // Give the recorded active zone the highest focus serial.
        this.focus_serial += 1;
        if let Some(zone) = this.zones.get_mut(&active) {
            zone.last_focused = this.focus_serial;
        }
        this
    }

    /// Puts zone `id` in the tree's map, showing what `start` started, of
    /// `spec`'s profile, host, theme and font size, with the subscriptions
    /// its kind has.
    fn insert_zone(&mut self, id: ZoneId, spec: ZoneSpec, start: ZoneStart, window: &mut Window, cx: &mut Context<Self>) {
        let ZoneSpec { profile, host, theme, font_size } = spec;
        let zone_tokens = theme.zone_tokens();
        let zone_theme = theme.clone();
        let (content, initial_title, mut subscriptions) = match start {
            ZoneStart::Terminal(terminal) => {
                terminal.update(cx, |t, _| t.theme = theme);
                let view = cx.new(|cx| TerminalView::new(terminal.clone(), font_size, window, cx));
                let events = cx.subscribe_in(&terminal, window, move |this, _, event, window, cx| {
                    this.on_zone_terminal_event(id, event, window, cx);
                });
                let link_failed = cx.subscribe(&view, |_, _, failed: &LinkFailed, cx| cx.emit(failed.clone()));
                // Without the process name, which the next refresh reads.
                let title: SharedString = {
                    let t = terminal.read(cx);
                    terminal_automatic_title(t.title(), &profile.name, LOCAL_PATH_STYLE, None).into()
                };
                (ZoneContent::Terminal(view), title, vec![events, link_failed])
            }
            ZoneStart::Agent(agent) => {
                let view = cx.new(|cx| AgentView::new(agent.clone(), theme, font_size, window, cx));
                // The agent emits the terminals' status and title events,
                // never `Exited`: its zone stays until the user closes it.
                let events = cx.subscribe_in(&agent, window, move |this, _, event, window, cx| {
                    this.on_zone_terminal_event(id, event, window, cx);
                });
                // Its links, as a terminal's: a failure for the config line,
                // and a link that asks first for the window's confirmation.
                let link_failed = cx.subscribe(&view, |_, _, failed: &LinkFailed, cx| cx.emit(failed.clone()));
                let confirm_link = cx.subscribe(&view, |_, _, confirm: &ConfirmLink, cx| cx.emit(confirm.clone()));
                // A terminal login's zone, beside this one.
                let login = cx.subscribe_in(&agent, window, move |this, _, event: &LoginZone, window, cx| {
                    this.on_login_zone(id, event, window, cx);
                });
                let title: SharedString = agent.read(cx).title().into();
                (ZoneContent::Agent(view), title, vec![events, link_failed, confirm_link, login])
            }
        };
        let focus = content.focus_handle(cx);
        subscriptions.push(cx.on_focus_in(&focus, window, move |this, _, cx| {
            this.on_zone_focused(id, cx);
        }));
        self.focus_serial += 1;
        let (status, progress, badge) = content.status(cx);
        let zone = Zone {
            content,
            profile,
            host,
            theme: zone_theme,
            zone_tokens,
            font_size,
            title: initial_title,
            status,
            progress,
            badge,
            last_focused: self.focus_serial,
            login_for: None,
            _subscriptions: subscriptions,
        };
        self.zones.insert(id, zone);
        self.refresh_motion(cx);
    }

    /// Zone `id`'s status mark (OpenSpec change `agents-zone-attention`,
    /// decision 3): an agent zone's `working` is a spinner.
    fn zone_mark(zone: &Zone) -> Mark {
        let agent = match zone.content {
            ZoneContent::Agent(_) => true,
            ZoneContent::Terminal(_) => false,
        };
        mark(zone.status, agent)
    }

    /// Whether one of the tab's working zones is an agent zone: what the
    /// tab's face needs to draw its `working` as a spinner.
    pub fn agent_working(&self) -> bool {
        self.zones.values().any(|zone| Self::zone_mark(zone).is_spinner())
    }

    /// Whether a zone header shows a spinner: the tab has 2+ zones and one
    /// is an agent working.
    fn header_spins(&self) -> bool {
        self.zones.len() >= 2 && self.agent_working()
    }

    /// Wants the motion clock while a header shows a spinner (decision 1).
    fn refresh_motion(&self, cx: &mut Context<Self>) {
        Motion::want(cx.entity_id(), self.header_spins(), cx);
    }

    /// The headers' spinner lines for `--dump-screen-after` (`--- spinner
    /// zone=ID`, the active zone first, then by id), none for a tab of one
    /// zone, which has no headers.
    pub fn describe_spinners(&self) -> String {
        if self.zones.len() < 2 {
            return String::new();
        }
        let mut ids: Vec<ZoneId> =
            self.zones.iter().filter(|(_, zone)| Self::zone_mark(zone).is_spinner()).map(|(id, _)| *id).collect();
        ids.sort_by_key(|id| (*id != self.active, id.0));
        ids.into_iter().map(|id| format!("--- spinner zone={}\n", id.0)).collect()
    }

    /// Agent zone `agent_zone` asks for its terminal login's zone, or for
    /// it to close (Cancel login).
    fn on_login_zone(&mut self, agent_zone: ZoneId, event: &LoginZone, window: &mut Window, cx: &mut Context<Self>) {
        if self.shutting_down {
            return;
        }
        match event {
            LoginZone::Open(plan) => self.open_login_zone(agent_zone, plan.clone(), window, cx),
            LoginZone::Close => {
                if let Some(id) = self.login_zone_of(agent_zone) {
                    self.close_zone_by_id(id, window, cx);
                }
            }
        }
    }

    /// The terminal login's zone of agent zone `agent_zone`, while open.
    fn login_zone_of(&self, agent_zone: ZoneId) -> Option<ZoneId> {
        self.zones.iter().find(|(_, zone)| zone.login_for == Some(agent_zone)).map(|(id, _)| *id)
    }

    /// The zones running terminal logins.
    fn login_zones(&self) -> Vec<ZoneId> {
        self.zones.iter().filter(|(_, zone)| zone.login_for.is_some()).map(|(id, _)| *id).collect()
    }

    /// Opens a terminal zone beside agent zone `agent_zone` running `plan`
    /// on that zone's host, in the agent's directory, with its profile's
    /// environment and the login's variables over it (decision 9), drawn as
    /// the agent zone is. It isn't a profile's zone: no split starts from
    /// it, its program ends with the tab or Brindle (a command's terminal,
    /// `Closing::Ends`), and it takes focus, since the login asks for
    /// typing. One per agent zone.
    fn open_login_zone(&mut self, agent_zone: ZoneId, plan: TerminalLogin, window: &mut Window, cx: &mut Context<Self>) {
        if self.login_zone_of(agent_zone).is_some() {
            return;
        }
        let Some(zone) = self.zones.get(&agent_zone) else { return };
        let spec = ZoneSpec {
            profile: zone.profile.clone(),
            host: zone.host.clone(),
            theme: zone.theme.clone(),
            font_size: zone.font_size,
        };
        let axis = login_axis(self.fits(agent_zone, SplitAxis::Horizontal, window, cx));
        let cwd = self.zone_working_directory(agent_zone, cx);
        let config: Config = Settings::get(cx).config.clone();
        let ignore_case = spec.host.is_local() && crate::terminal::process::LOCAL_ENV_IGNORES_CASE;
        let mut profile = spec.profile.clone();
        profile.env = login::overlay(&profile.env, &plan.env, ignore_case);
        let new_id = ZoneId(self.next_id);
        self.next_id += 1;
        if !self.tree.split(agent_zone, axis, new_id) {
            return;
        }
        let host = spec.host.clone();
        let command = Some((plan.command, plan.args));
        let terminal = cx.new(|cx| Terminal::spawn(&profile, &host, command, cwd, &config, cx));
        self.refresh_zone_title(agent_zone, cx);
        self.insert_zone(new_id, spec, ZoneStart::Terminal(terminal), window, cx);
        if let Some(zone) = self.zones.get_mut(&new_id) {
            zone.login_for = Some(agent_zone);
        }
        self.refresh_zone_title(new_id, cx);
        self.active = new_id;
        let focus = self.focus_handle(cx);
        window.focus(&focus);
        cx.emit(TerminalEvent::TitleChanged);
        cx.notify();
    }

    /// Zone `id`'s terminal login ended as `end`: its agent zone hears it,
    /// once.
    fn end_login(&mut self, id: ZoneId, end: LoginZoneEnd, cx: &mut Context<Self>) {
        let Some(agent_zone) = self.zones.get_mut(&id).and_then(|zone| zone.login_for.take()) else { return };
        let Some(view) = self.zones.get(&agent_zone).and_then(|zone| zone.content.agent_view()) else { return };
        let agent = view.read(cx).agent().clone();
        agent.update(cx, |agent, cx| agent.login_zone_ended(end, cx));
    }

    fn on_zone_focused(&mut self, id: ZoneId, cx: &mut Context<Self>) {
        let Some(zone) = self.zones.get_mut(&id) else { return };
        self.focus_serial += 1;
        zone.last_focused = self.focus_serial;
        if self.active != id {
            self.active = id;
            cx.emit(TerminalEvent::TitleChanged);
            cx.notify();
        }
    }

    fn on_zone_terminal_event(
        &mut self,
        id: ZoneId,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shutting_down {
            return;
        }
        match event {
            TerminalEvent::Exited => {
                // A terminal login's program: its status first reaches
                // its agent zone.
                let code = self.zones.get(&id).and_then(|zone| zone.content.terminal(cx)).and_then(|t| t.read(cx).exit_code());
                self.end_login(id, LoginZoneEnd::Exited { code }, cx);
                self.close_zone_by_id(id, window, cx);
            }
            TerminalEvent::Bell | TerminalEvent::Notify(_) | TerminalEvent::Notify99(_) => {
                self.notifying = Some(id);
                cx.emit(event.clone());
            }
            TerminalEvent::Status(notice) => {
                if notice.is_some() {
                    self.notifying = Some(id);
                }
                if let Some(zone) = self.zones.get_mut(&id) {
                    (zone.status, zone.progress, zone.badge) = zone.content.status(cx);
                }
                self.refresh_motion(cx);
                cx.emit(event.clone());
                cx.notify();
            }
            TerminalEvent::TitleChanged => {
                let changed = self.refresh_zone_title(id, cx);
                if id == self.active {
                    cx.emit(TerminalEvent::TitleChanged);
                } else if changed {
                    cx.notify();
                }
            }
        }
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.zones
            .get(&self.active)
            .map(|z| z.content.focus_handle(cx))
            .unwrap_or_else(|| self.focus_handle.clone())
    }

    /// The active zone's terminal view; `None` for an agent zone.
    pub fn active_terminal_view(&self) -> Option<Entity<TerminalView>> {
        self.zones.get(&self.active)?.content.terminal_view().cloned()
    }

    /// The active zone's terminal; `None` for an agent zone.
    pub fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        self.zones.get(&self.active)?.content.terminal(cx)
    }

    /// The active zone's content.
    pub fn active_content(&self) -> Option<&ZoneContent> {
        Some(&self.zones.get(&self.active)?.content)
    }

    /// The active zone's agent dump block; `None` for a terminal zone.
    pub fn describe_active_agent(&self, cx: &App) -> Option<String> {
        self.active_content()?.describe_agent(self.active, cx)
    }

    pub fn active_id(&self) -> ZoneId {
        self.active
    }

    pub fn tree(&self) -> &ZoneTree {
        &self.tree
    }


    pub fn zone_working_directory(&self, id: ZoneId, cx: &App) -> Option<PathBuf> {
        self.zones.get(&id)?.content.working_directory(cx)
    }

    /// What session restore records for each zone (its directory and its
    /// session), for `session::record_zones`. A zone that only shows a
    /// message (an attach that failed), and an agent zone whose session
    /// isn't open, has no entry ([`ZoneContent::unrecorded`]).
    pub fn zone_records(&self, cx: &App) -> HashMap<ZoneId, crate::session::ZoneRecord> {
        let unrecorded = |id: &ZoneId| self.zones.get(id).is_some_and(|zone| zone.content.unrecorded(cx));
        recorded_leaves(self.tree.leaves(), &self.login_zones())
            .into_iter()
            .filter(|id| !unrecorded(id))
            .filter_map(|id| {
                let zone = self.zones.get(&id)?;
                let record = crate::session::ZoneRecord {
                    cwd: self.zone_working_directory(id, cx),
                    session: self.zone_server_session(id, cx),
                    profile: zone.profile.name.clone(),
                    // Where it runs, which its profile may no longer name.
                    remote: zone.host.target().cloned(),
                };
                Some((id, record))
            })
            .collect()
    }

    /// The terminal server session zone `id` shows, for session restore.
    pub fn zone_server_session(&self, id: ZoneId, cx: &App) -> Option<crate::session::ServerSession> {
        #[cfg(any(target_os = "linux", windows))]
        {
            let zone = self.zones.get(&id)?;
            let (id, key) = zone.content.server_session(cx)?;
            // The state file's form: an empty key for none.
            let key = key.map(crate::remote::SessionKey::into_string).unwrap_or_default();
            Some(crate::session::ServerSession { id, key })
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = (id, cx);
            None
        }
    }

    /// The active zone's title as [`ZoneTreeView::refresh_titles`] last
    /// cached it (its program's, else its profile's name): the tab's
    /// automatic title. Empty with no zone, which only a closing tab has.
    pub fn active_title(&self) -> SharedString {
        self.zones.get(&self.active).map(|zone| zone.title.clone()).unwrap_or_default()
    }

    /// The active zone, which the tab takes its profile's look from.
    pub fn active_zone(&self) -> Option<&Zone> {
        self.zones.get(&self.active)
    }

    /// The active zone's theme: the tab face's (`tabs`' "A tab follows its
    /// active zone"). The default theme with no zone.
    pub fn active_theme(&self) -> Theme {
        self.active_zone().map(|zone| zone.theme.clone()).unwrap_or_default()
    }

    /// The name of the profile of the zone whose bell or notification was
    /// passed on last, else the active zone's: what the window's
    /// notification names.
    pub fn notifying_profile(&self) -> String {
        self.notifying
            .and_then(|id| self.zones.get(&id))
            .or_else(|| self.active_zone())
            .map(|zone| zone.profile.name.clone())
            .unwrap_or_default()
    }

    /// The tree's zones in its order.
    pub fn zones_in_order(&self) -> Vec<&Zone> {
        self.tree.leaves().iter().filter_map(|id| self.zones.get(id)).collect()
    }

    /// The endpoints the tab's zones run on, in the tree's order, each
    /// once.
    pub fn hosts(&self) -> Vec<Host> {
        let mut hosts: Vec<Host> = Vec::new();
        for zone in self.zones_in_order() {
            if !hosts.contains(&zone.host) {
                hosts.push(zone.host.clone());
            }
        }
        hosts
    }

    /// The name of the profile of the first zone on `host`, in the tree's
    /// order.
    pub fn profile_on(&self, host: &Host) -> Option<String> {
        self.zones_in_order().into_iter().find(|zone| zone.host == *host).map(|zone| zone.profile.name.clone())
    }

    /// The terminal server sessions the zones on `host` show.
    pub fn sessions_on(&self, host: &Host, cx: &App) -> Vec<crate::session::ServerSession> {
        self.tree
            .leaves()
            .into_iter()
            .filter(|id| self.zones.get(id).is_some_and(|zone| zone.host == *host))
            .filter_map(|id| self.zone_server_session(id, cx))
            .collect()
    }

    /// Refreshes cached zone header titles for all zones in this tab.
    pub fn refresh_titles(&mut self, cx: &App) -> bool {
        let ids: Vec<ZoneId> = self.zones.keys().copied().collect();
        let mut any = false;
        for id in ids {
            any |= self.refresh_zone_title(id, cx);
        }
        any
    }

    fn refresh_zone_title(&mut self, id: ZoneId, cx: &App) -> bool {
        let Some(zone) = self.zones.get_mut(&id) else { return false };
        let title = zone.content.title(&zone.profile.name, cx);
        let changed = zone.title != title;
        zone.title = title;
        changed
    }

    /// The most pressing status across all zones in this tab: the
    /// terminals' program status and the agent zones' shown status.
    pub fn status_summary(&self, cx: &App) -> Option<State> {
        self.zones.values().filter_map(|z| z.content.status(cx).0).max()
    }

    /// The most pressing progress across all zones in this tab
    /// ([`progress::most_pressing`]), from the zones' cached values.
    pub fn progress_summary(&self) -> Option<Progress> {
        progress::most_pressing(self.zones.values().filter_map(|z| z.progress))
    }

    /// The zones' progress lines for `--dump-screen-after`
    /// ([`describe_zone_progress`]).
    pub fn describe_progress(&self) -> String {
        describe_zone_progress(self.zones.iter().map(|(id, z)| (*id, z.progress)), self.active)
    }

    /// The active zone's context badge, from its cached value: what the
    /// tab's face shows.
    pub fn active_badge(&self) -> Option<&Badge> {
        self.zones.get(&self.active).and_then(|z| z.badge.as_ref())
    }

    /// The zones' badge lines for `--dump-screen-after`
    /// ([`describe_zone_badges`]).
    pub fn describe_badges(&self) -> String {
        describe_zone_badges(self.zones.iter().map(|(id, z)| (*id, z.badge.as_ref())), self.active)
    }

    /// Gives each zone the profile `follow` gives its own (the config's
    /// after a reload or a rename), keeping it where `follow` gives none.
    /// Hosts stay; the look is [`ZoneTreeView::restyle`]'s.
    pub fn follow_profiles(&mut self, follow: &dyn Fn(&Profile) -> Option<Profile>) {
        for zone in self.zones.values_mut() {
            if let Some(profile) = follow(&zone.profile) {
                zone.profile = profile;
            }
        }
    }

    /// Draws each zone in the theme and font size `look` gives its own
    /// profile (the config's, or the settings dialog's preview), an agent
    /// zone taking its profile's name too (a title that is the profile's
    /// follows a rename).
    pub fn restyle(&mut self, look: &dyn Fn(&Profile) -> (Theme, Option<f32>), cx: &mut Context<Self>) {
        for zone in self.zones.values_mut() {
            let (theme, font_size) = look(&zone.profile);
            zone.zone_tokens = theme.zone_tokens();
            zone.theme = theme.clone();
            zone.font_size = font_size;
            zone.content.set_profile(&zone.profile.name, theme, font_size, cx);
        }
        cx.notify();
    }

    /// Closes every zone's terminal or agent session, ending it, when the
    /// user closes the tab.
    pub fn shutdown_all(&mut self, cx: &mut Context<Self>) {
        self.shutting_down = true;
        for zone in self.zones.values() {
            zone.content.close(cx);
        }
    }

    /// Brindle is quitting: every terminal leaves its program as its
    /// `Closing` says ([`Terminal::quit_with`]), a kept session detached
    /// with this tab's layout (its split tree with the zones' session keys,
    /// the `index`th tab of the Brindle `origin`, titled `title`), so a
    /// launch that sweeps it rebuilds the tab.
    pub fn quit_all(&mut self, origin: &str, index: usize, title: Option<&str>, cx: &mut Context<Self>) {
        self.shutting_down = true;
        let layout = self.layout(origin, index, title, cx);
        for zone in self.zones.values() {
            zone.content.quit_with(layout.clone(), cx);
        }
    }

    /// This tab's layout as the `index`th tab of the Brindle `origin`
    /// (`layout::origin_id`), titled `title` ([`crate::layout::record`]);
    /// `None` when no zone has a session.
    pub fn layout(&self, origin: &str, index: usize, title: Option<&str>, cx: &App) -> Option<serde_json::Value> {
        crate::layout::record(&self.tree, self.active, &self.session_keys(cx), origin, index, title)
    }

    /// The session key of each zone that shows a session with one, for
    /// [`crate::layout::record`]. A zone without one (a message, a session
    /// not started yet, a core that predates keys) has no entry.
    pub fn session_keys(&self, cx: &App) -> HashMap<ZoneId, String> {
        recorded_leaves(self.zones.keys().copied().collect(), &self.login_zones())
            .into_iter()
            .filter_map(|id| {
                let key = self.zone_server_session(id, cx)?.key;
                (!key.is_empty()).then_some((id, key))
            })
            .collect()
    }

    /// Keeps every zone's session's copy of this tab's layout current
    /// (`Terminal::set_layout`), the session recorder's once-a-second tick.
    pub fn send_layout(&self, origin: &str, index: usize, title: Option<&str>, cx: &App) {
        if self.shutting_down {
            return;
        }
        let layout = self.layout(origin, index, title, cx);
        for zone in self.zones.values() {
            zone.content.set_layout(layout.clone(), cx);
        }
    }

    /// Every terminal zone's terminal (agent zones have none).
    pub fn terminals(&self, cx: &App) -> Vec<Entity<Terminal>> {
        self.zones.values().filter_map(|z| z.content.terminal(cx)).collect()
    }

    /// A restore's note for the active zone ([`ZoneContent::note`]).
    pub fn note_active(&self, text: &str, cx: &mut App) {
        self.note_zone(self.active, text, cx);
    }

    /// A restore's note for zone `id` ([`ZoneContent::note`]).
    pub fn note_zone(&self, id: ZoneId, text: &str, cx: &mut App) {
        if let Some(zone) = self.zones.get(&id) {
            zone.content.note(text, cx);
        }
    }

    pub fn describe_zones(&self) -> String {
        describe_zone_tree(&self.tree, self.active)
    }

    /// Outer pixel size of `id`'s card (its border and header included) for
    /// the minimum-split-size check.
    fn zone_outer_size(&self, id: ZoneId, window: &Window, cx: &App) -> Option<Size<Pixels>> {
        let padding = px(Settings::get(cx).config.padding);
        let border = brindle_ui::CARD_BORDER * 2.0;
        let has_header = self.zones.len() >= 2;
        if let Some(grid) = self.zones.get(&id).and_then(|z| z.content.grid(cx)) {
            let w = grid.cell_width * grid.cols as f32 + padding * 2.0 + border;
            let h = grid.line_height * grid.rows as f32
                + padding * 2.0
                + border
                + if has_header { px(ZONE_HEADER_HEIGHT) } else { px(0.0) };
            return Some(size(w, h));
        }
        // Before a terminal's first paint, and for an agent zone (which has
        // no grid): estimate from the window's inner bounds, less the
        // cards' margin, and the zone's normalized rectangle.
        let (_, rect) = self.tree.rects().into_iter().find(|(zid, _)| *zid == id)?;
        let win_size = window.inner_window_bounds().get_bounds().size;
        let content_w = (f32::from(win_size.width) - ZONE_GAP * 2.0).max(0.0);
        let content_h = (f32::from(win_size.height) - crate::workspace::tab_bar_height() - ZONE_GAP * 2.0).max(0.0);
        Some(size(px(content_w * rect.w as f32), px(content_h * rect.h as f32)))
    }

    /// Which profile a split of zone `id` asking for `want` runs, from the
    /// config as it is now ([`split_choice`]).
    fn split_choice(&self, id: ZoneId, want: ProfileKind, cx: &App) -> SplitChoice {
        let config = &Settings::get(cx).config;
        let own = self.zones.get(&id).and_then(|zone| config.profile_index(&zone.profile.name));
        split_choice(want, own, config)
    }

    /// Whether zone `id` can be split at all: it is there and isn't a
    /// terminal login's zone, which no split starts from.
    fn splittable(&self, id: ZoneId) -> bool {
        self.zones.get(&id).is_some_and(|zone| zone.login_for.is_none())
    }

    /// Whether a split of zone `id` asking for `want` runs a profile, so its
    /// item and button aren't disabled: the zone can be split and the config
    /// has a profile of the kind.
    fn can_split_with(&self, id: ZoneId, want: ProfileKind, cx: &App) -> bool {
        match self.split_choice(id, want, cx) {
            SplitChoice::Run(_) | SplitChoice::Ask(_) => self.splittable(id),
            SplitChoice::Refused(_) => false,
        }
    }

    /// Whether zone `id` is big enough to split along `axis`, at its
    /// current size ([`can_split_zone`]); one whose size isn't known yet is.
    fn fits(&self, id: ZoneId, axis: SplitAxis, window: &Window, cx: &App) -> bool {
        let font_size = self.zones.get(&id).and_then(|zone| zone.font_size);
        let metrics = cell_metrics(font_size, window, cx);
        let padding = px(Settings::get(cx).config.padding);
        let has_header = self.zones.len() >= 2;
        self.zone_outer_size(id, window, cx).is_none_or(|zone_size| {
            can_split_zone(zone_size, has_header, axis, padding, metrics.cell_width, metrics.line_height)
        })
    }

    /// Focuses zone `id`, then asks the window for its header menu at `at`
    /// (from the secondary `press`, if one asked), its split items enabled
    /// by [`MenuSplits`]. Focus comes first, so the items act on this zone.
    pub fn open_menu(
        &mut self,
        id: ZoneId,
        at: Point<Pixels>,
        press: Option<MouseDownEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.zones.contains_key(&id) {
            return;
        }
        self.focus_zone(id, window, cx);
        let splittable = self.splittable(id);
        let rows = zone_menu_rows(MenuSplits {
            right: splittable && self.fits(id, SplitAxis::Horizontal, window, cx),
            down: splittable && self.fits(id, SplitAxis::Vertical, window, cx),
            terminal: self.can_split_with(id, ProfileKind::Terminal, cx),
            agent: self.can_split_with(id, ProfileKind::Agent, cx),
        });
        cx.emit(ZoneMenuRequest { rows, at, press, tokens: self.scope_of(id) });
    }

    /// The theme scope zone `id` is drawn in, which its tooltips and header
    /// menu take too: an agent zone's is its own theme's zone token set
    /// (`agents-zone-theme-scope` decision 3), a terminal zone has none
    /// and keeps the window's set, as does a zone that isn't there.
    fn scope_of(&self, id: ZoneId) -> Option<Tokens> {
        let zone = self.zones.get(&id)?;
        match &zone.content {
            ZoneContent::Agent(_) => Some(zone.zone_tokens),
            ZoneContent::Terminal(_) => None,
        }
    }

    /// Zone `id`'s card colours, and the scope it is drawn in: its own
    /// theme's card colours on the window's background (`window`, the set
    /// round the cards), and for a zone drawn in a scope, the window's
    /// focus ring, which would otherwise take the scope's.
    fn leaf_look(&self, id: ZoneId, window: &Tokens) -> (CardColors, Option<Tokens>) {
        let scope = self.scope_of(id);
        let mut colors = self.zones.get(&id).map_or_else(|| Theme::default().card_colors(), |z| z.theme.card_colors());
        colors.surround = Some(window.background);
        if scope.is_some() {
            colors.ring = Some(brindle_ui::focus_ring(window));
        }
        (colors, scope)
    }

    /// Where `OpenZoneMenu` puts the active zone's menu: below the start of
    /// its header, or, in a tab of one zone (which has no header), at the
    /// top-left corner of the tab's content. `None` before the first frame.
    fn menu_anchor(&self) -> Option<Point<Pixels>> {
        let laid_out = self.laid_out.borrow();
        match laid_out.headers.get(&self.active) {
            Some(header) if self.zones.len() >= 2 => Some(header.bottom_left()),
            _ => laid_out.tree.map(|tree| tree.origin),
        }
    }

    /// Splits zone `target` along `axis` with a new zone of what `with`
    /// asks for (OpenSpec change `zones-mixed-tabs`, decisions 2–4). The
    /// size rule comes first, and a terminal login's zone splits with
    /// nothing. A kind whose profile needs choosing asks the window
    /// ([`SplitAsk`]), and one with no profile says so ([`SplitRefused`]).
    /// The new zone runs where [`split_host`] says, in the directory
    /// [`split_cwd`] gives, and takes focus.
    pub fn split_zone(&mut self, target: ZoneId, axis: SplitAxis, with: SplitWith, window: &mut Window, cx: &mut Context<Self>) {
        if !self.splittable(target) || !self.fits(target, axis, window, cx) {
            return;
        }
        let config: Config = Settings::get(cx).config.clone();
        let key = match with {
            SplitWith::Kind(want) => match self.split_choice(target, want, cx) {
                SplitChoice::Run(key) => key,
                SplitChoice::Ask(profiles) => {
                    cx.emit(SplitAsk { zone: target, axis, profiles });
                    return;
                }
                SplitChoice::Refused(refusal) => {
                    cx.emit(SplitRefused(refusal));
                    return;
                }
            },
            SplitWith::Profile(key) => key,
        };
        // A profile chosen in a palette may have gone since it opened.
        let Some(profile) = config.find_profile(&key).map(|ix| config.profiles[ix].clone()) else { return };
        let Some(split) = self.zones.get(&target) else { return };
        let host = split_host(&split.profile, &split.host, &profile);
        // The target zone's working directory (an agent zone's: the one its
        // session started in) when the new zone runs on its endpoint; else
        // the spawn takes the profile's directory, or home there.
        let same_host = split.host == host;
        let cwd = split_cwd(self.zone_working_directory(target, cx), same_host);

        let new_id = ZoneId(self.next_id);
        self.next_id += 1;
        if !self.tree.split(target, axis, new_id) {
            return;
        }
        let start = match profile.kind() {
            ProfileKind::Terminal => {
                ZoneStart::Terminal(cx.new(|cx| Terminal::spawn(&profile, &host, None, cwd, &config, cx)))
            }
            ProfileKind::Agent => {
                ZoneStart::Agent(cx.new(|cx| Agent::spawn(&profile, &host, cwd, OpenedBy::Window, None, cx)))
            }
        };
        let spec = ZoneSpec::of(profile, host, &config);

        // Refresh the existing zone's title so its header bar has an up-to-date label.
        self.refresh_zone_title(target, cx);
        self.insert_zone(new_id, spec, start, window, cx);
        self.refresh_zone_title(new_id, cx);
        self.active = new_id;
        let focus = self.focus_handle(cx);
        window.focus(&focus);
        cx.emit(TerminalEvent::TitleChanged);
        cx.notify();
    }

    /// Closes zone `target`, ending its terminal's program or its agent
    /// session.
    pub fn close_zone_by_id(&mut self, target: ZoneId, window: &mut Window, cx: &mut Context<Self>) {
        // A terminal login's zone closed before its program exited is a
        // cancel; an agent zone's login zone closes with it.
        self.end_login(target, LoginZoneEnd::Closed, cx);
        if let Some(login) = self.login_zone_of(target) {
            if let Some(zone) = self.zones.get_mut(&login) {
                zone.login_for = None;
            }
            self.close_zone_by_id(login, window, cx);
        }
        let Some(removed) = self.zones.remove(&target) else { return };
        removed.content.close(cx);
        self.refresh_motion(cx);

        if self.zones.is_empty() {
            cx.emit(TerminalEvent::Exited);
            return;
        }

        let Some(promoted) = self.tree.remove(target) else {
            cx.emit(TerminalEvent::Exited);
            return;
        };

        if self.active == target {
            // Focus the most recently focused leaf in the promoted sibling subtree.
            let next = promoted
                .into_iter()
                .filter_map(|id| self.zones.get(&id).map(|z| (id, z.last_focused)))
                .max_by_key(|&(_, serial)| serial)
                .map(|(id, _)| id)
                .or_else(|| self.tree.leaves().into_iter().next());
            if let Some(next_id) = next {
                self.active = next_id;
                self.focus_serial += 1;
                if let Some(zone) = self.zones.get_mut(&next_id) {
                    zone.last_focused = self.focus_serial;
                }
            }
        }
        let focus = self.focus_handle(cx);
        window.focus(&focus);
        cx.emit(TerminalEvent::TitleChanged);
        cx.emit(TerminalEvent::Status(None));
        cx.notify();
    }

    pub fn focus_direction(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.tree.focus_target(self.active, direction) else { return };
        self.focus_zone(target, window, cx);
    }

    pub fn focus_zone(&mut self, id: ZoneId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.zones.contains_key(&id) {
            return;
        }
        self.focus_serial += 1;
        if let Some(zone) = self.zones.get_mut(&id) {
            zone.last_focused = self.focus_serial;
        }
        let changed = self.active != id;
        self.active = id;
        let focus = self.focus_handle(cx);
        window.focus(&focus);
        if changed {
            cx.emit(TerminalEvent::TitleChanged);
        }
        cx.notify();
    }

    // ---- action listeners -------------------------------------------------------

    fn split_terminal_right(&mut self, _: &SplitTerminalRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_zone(self.active, SplitAxis::Horizontal, SplitWith::Kind(ProfileKind::Terminal), window, cx);
    }

    fn split_terminal_down(&mut self, _: &SplitTerminalDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_zone(self.active, SplitAxis::Vertical, SplitWith::Kind(ProfileKind::Terminal), window, cx);
    }

    fn split_agent_right(&mut self, _: &SplitAgentRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_zone(self.active, SplitAxis::Horizontal, SplitWith::Kind(ProfileKind::Agent), window, cx);
    }

    fn split_agent_down(&mut self, _: &SplitAgentDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_zone(self.active, SplitAxis::Vertical, SplitWith::Kind(ProfileKind::Agent), window, cx);
    }

    fn split_right_with_profile(&mut self, _: &SplitRightWithProfile, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_split_profile(self.active, SplitAxis::Horizontal, window, cx);
    }

    fn split_down_with_profile(&mut self, _: &SplitDownWithProfile, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_split_profile(self.active, SplitAxis::Vertical, window, cx);
    }

    /// Asks the window which profile to split zone `target` with, offering
    /// every profile ([`split_profile_choices`]), when the zone can be split
    /// along `axis`.
    fn ask_split_profile(&mut self, target: ZoneId, axis: SplitAxis, window: &mut Window, cx: &mut Context<Self>) {
        if !self.splittable(target) || !self.fits(target, axis, window, cx) {
            return;
        }
        let profiles = split_profile_choices(&Settings::get(cx).config);
        cx.emit(SplitAsk { zone: target, axis, profiles });
    }

    /// Close Zone (its key, the header's ×, the header menu's item): at
    /// once, or after the window's confirmation when its agent is busy
    /// ([`close_plan`]).
    fn close_active_zone(&mut self, _: &CloseZone, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close_zone(self.active, window, cx);
    }

    /// The user asked to close zone `target`: [`close_plan`] decides
    /// whether it closes now or the window asks first ([`CloseZoneAsk`]).
    pub fn request_close_zone(&mut self, target: ZoneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closing) = self.closing_zone(target, cx) else { return };
        match close_plan(&[closing]) {
            ClosePlan::Now => self.close_zone_by_id(target, window, cx),
            ClosePlan::Ask(busy) => cx.emit(CloseZoneAsk { zone: target, busy }),
        }
    }

    /// What closing zone `id` would end, for [`close_plan`]: a terminal, or
    /// an agent with its title and whether the status it shows is busy.
    fn closing_zone(&self, id: ZoneId, cx: &App) -> Option<Closing> {
        let zone = self.zones.get(&id)?;
        Some(match &zone.content {
            ZoneContent::Terminal(_) => Closing::Terminal(id),
            ZoneContent::Agent(view) => Closing::Agent {
                zone: id,
                name: zone.title.to_string(),
                busy: view.read(cx).agent().read(cx).model().close_busy(),
            },
        })
    }

    /// What closing the whole tab would end: every zone, in the tree's
    /// order.
    pub fn closing(&self, cx: &App) -> Vec<Closing> {
        self.tree.leaves().into_iter().filter_map(|id| self.closing_zone(id, cx)).collect()
    }

    /// Whether the tab still has zone `id` (a confirmation open on it
    /// closes itself once it is gone).
    pub fn has_zone(&self, id: ZoneId) -> bool {
        self.zones.contains_key(&id)
    }

    /// `OpenZoneMenu`: the active zone's header menu, below its header (see
    /// [`ZoneTreeView::menu_anchor`]; before the first frame, at the
    /// window's top-left).
    fn open_zone_menu(&mut self, _: &OpenZoneMenu, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.menu_anchor().unwrap_or_default();
        self.open_menu(self.active, at, None, window, cx);
    }

    fn focus_left(&mut self, _: &FocusZoneLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Left, window, cx);
    }

    fn focus_right(&mut self, _: &FocusZoneRight, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Right, window, cx);
    }

    fn focus_up(&mut self, _: &FocusZoneUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Up, window, cx);
    }

    fn focus_down(&mut self, _: &FocusZoneDown, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Down, window, cx);
    }

    // ---- rendering --------------------------------------------------------------

    /// The split tree under `node`: a card per zone, the [`ZONE_GAP`]
    /// between them (no dividers), on the chrome's background token, which the
    /// root paints.
    fn render_tree(&self, node: &ZoneTree, tokens: &Tokens, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match node {
            ZoneTree::Leaf(id) => self.render_leaf(*id, tokens, window, cx),
            ZoneTree::Split { axis, first, second } => {
                let first_el = self.render_tree(first, tokens, window, cx);
                let second_el = self.render_tree(second, tokens, window, cx);
                // Nothing here clips: the focus ring is drawn in the gap.
                div()
                    .flex()
                    .when(*axis == SplitAxis::Horizontal, |d| d.flex_row())
                    .when(*axis == SplitAxis::Vertical, |d| d.flex_col())
                    .gap(px(ZONE_GAP))
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().min_h_0().size_full().child(first_el))
                    .child(div().flex_1().min_w_0().min_h_0().size_full().child(second_el))
                    .into_any_element()
            }
        }
    }

    /// Zone `id`: a card in its terminal's colours round the terminal, with
    /// a header when the tab has 2+ zones (recorded for `menu_anchor`), and
    /// the focus ring when it is the active zone of such a tab and focus
    /// came there from the keyboard.
    fn render_leaf(&self, id: ZoneId, tokens: &Tokens, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(zone) = self.zones.get(&id) else {
            return div().size_full().into_any_element();
        };
        let view = match &zone.content {
            ZoneContent::Terminal(_) => zone.content.element(),
            // A terminal focuses itself on a press; the agent zone's
            // conversation has nothing focusable outside its buttons, so a
            // press anywhere in an inactive agent zone makes it the active
            // one first (focusing its prompt), before a button it reached
            // takes focus for itself.
            ZoneContent::Agent(_) => div()
                .id(("agent-zone", id.0 as usize))
                .size_full()
                .min_w_0()
                .min_h_0()
                .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    if this.active != id {
                        this.focus_zone(id, window, cx);
                    }
                }))
                .child(zone.content.element())
                .into_any_element(),
        };
        // The gap round the cards is the strip's colour (the chrome
        // background), so the corner cutouts match it. An agent zone is
        // drawn, header and all, in its theme's zone tokens, keeping the
        // window's ring; what else keeps the window's colours (the status
        // dot, the progress bar) is computed from `tokens` below.
        let (colors, scope) = self.leaf_look(id, tokens);
        let scoped = move |card: AnyElement| match scope {
            Some(set) => ThemeScope::new(set, card).into_any_element(),
            None => card,
        };
        let card = Card::new(("zone", id.0 as usize)).colors(colors).size_full().min_w_0().min_h_0();
        if self.zones.len() <= 1 {
            return scoped(card.child(view).into_any_element());
        }

        let active = id == self.active;
        let focus = zone.content.focus_handle(cx);
        let ring = active && shows_ring(zone.content.contains_focused(window, cx), modality(window, cx), false);
        // Status dots in the chrome theme's colours, like the strip's; an
        // agent's `working` is a spinner in the working dot's colour, at
        // the motion clock's step.
        let status_mark = match Self::zone_mark(zone) {
            Mark::None => None,
            Mark::Dot(state) => {
                let color = Settings::chrome_theme(cx).status_color(state, tokens.muted_foreground);
                Some(div().flex_none().size(px(6.0)).rounded_full().bg(color).into_any_element())
            }
            Mark::Spinner => {
                let color = Settings::chrome_theme(cx).status_color(State::Working, tokens.muted_foreground);
                Some(Loader::new().step(motion::loader_step(Motion::step(cx))).color(color).into_any_element())
            }
        };
        // Its progress bar, drawn as the tab's is, from the same colours.
        let progress_bar = zone.progress.map(|progress| {
            let (length, color) = Settings::chrome_theme(cx).progress_bar(progress, tokens.muted_foreground);
            EdgeBar::new(length, color)
        });
        let title = zone.title.clone();
        let muted = colors.muted.unwrap_or(tokens.muted_foreground);

        // A tooltip naming a button and the key bound to its action, looked
        // up when it is shown. The zone actions are bound in `ZoneTree`, so
        // the lookup starts at this zone's terminal, which is inside it.
        let tip = |label: &'static str, action: Box<dyn Action>| {
            let (focus, action): (FocusHandle, Rc<dyn Action>) = (focus.clone(), Rc::from(action));
            move |window: &mut Window, _: &mut App| Tooltip::new(label).shortcut_opt(shortcut(&*action, &focus, window))
        };
        // In the terminal's muted colour: a `Button` sets its own text
        // colour, so the header's action slot can't give it one.
        let button = |name: &'static str, icon: Icon| {
            Button::icon_only((name, id.0 as usize), icon)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconXs)
                .text_color(muted)
        };
        // A press on a header button stops at its wrapper: the header's own
        // press focuses the zone, and a `Button` without a focus handle
        // would let the press bubble to the zone tree, which tracks focus
        // and would take it from the terminal.
        let inert = |name: &'static str, control: Button| {
            div()
                .id((name, id.0 as usize))
                .flex_none()
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .child(control)
        };
        // The split buttons start another zone of this zone's own kind,
        // disabled where that can't run a profile or the zone is a terminal
        // login's (a zone too small to split finds out on a press, as
        // before: that needs the font's metrics).
        let kind = zone.content.kind();
        let refused = !self.can_split_with(id, kind, cx);
        let (right_action, down_action) = split_actions(kind);
        let with = SplitWith::Kind(kind);
        let with_down = with.clone();
        let split_right = button("zone-split-right", Icon::Columns2)
            .disabled(refused)
            .tooltip_with(tip("Split right", right_action))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.split_zone(id, SplitAxis::Horizontal, with.clone(), window, cx)
            }));
        let split_down = button("zone-split-down", Icon::Rows2)
            .disabled(refused)
            .tooltip_with(tip("Split down", down_action))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.split_zone(id, SplitAxis::Vertical, with_down.clone(), window, cx)
            }));
        let close = button("zone-close", Icon::X)
            .tooltip_with(tip("Close zone", Box::new(CloseZone)))
            .on_click(cx.listener(move |this, _, window, cx| this.request_close_zone(id, window, cx)));

        // The title in the terminal's foreground on the active zone, muted
        // on the others (`trigger_label` cuts it, as `truncate` can't).
        let mut header = CardHeader::new(("zone-header", id.0 as usize));
        if let Some(status_mark) = status_mark {
            header = header.leading(status_mark);
        }
        if let Some(bar) = progress_bar {
            header = header.edge_bar(bar);
        }
        // Its context badge, drawn as the tab face's is.
        if let Some(badge) = &zone.badge {
            header = header.badge(ui_badge(badge, &zone.theme));
        }
        let header = header
            .title(title)
            .title_muted(!active)
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                this.focus_zone(id, window, cx);
                cx.stop_propagation();
            }))
            // A secondary press focuses the zone and opens its menu at the
            // pointer; it goes no further and moves focus nowhere else.
            .on_context_menu(cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.open_menu(id, event.position, Some(event.clone()), window, cx);
            }))
            .action(inert("zone-split-right-press", split_right))
            .action(inert("zone-split-down-press", split_down))
            .action(inert("zone-close-press", close));

        let laid_out = self.laid_out.clone();
        scoped(
            card.header(header)
                .ring(ring)
                .on_header_bounds(move |bounds, _, _| {
                    laid_out.borrow_mut().headers.insert(id, bounds);
                })
                .child(view)
                .into_any_element(),
        )
    }
}

impl Render for ZoneTreeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.tree.clone();
        // Headers are recorded again as this frame lays them out.
        self.laid_out.borrow_mut().headers.clear();
        // The window's set, for what round and on the cards keeps the
        // window's colours whatever scope a zone is drawn in.
        let tokens = cx.window_theme();
        let content = self.render_tree(&tree, &tokens, window, cx);
        let laid_out = self.laid_out.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| laid_out.borrow_mut().tree = bounds.first().copied())
            .id("zone-tree")
            .key_context("ZoneTree")
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, window, cx| {
                let position = window.mouse_position();
                for view in this.zones.values().filter_map(|zone| zone.content.terminal_view()) {
                    view.update(cx, |view, cx| view.modifiers_changed(position, e.modifiers, cx));
                }
            }))
            .on_action(cx.listener(Self::split_terminal_right))
            .on_action(cx.listener(Self::split_terminal_down))
            .on_action(cx.listener(Self::split_agent_right))
            .on_action(cx.listener(Self::split_agent_down))
            .on_action(cx.listener(Self::split_right_with_profile))
            .on_action(cx.listener(Self::split_down_with_profile))
            .on_action(cx.listener(Self::close_active_zone))
            .on_action(cx.listener(Self::open_zone_menu))
            .on_action(cx.listener(Self::focus_left))
            .on_action(cx.listener(Self::focus_right))
            .on_action(cx.listener(Self::focus_up))
            .on_action(cx.listener(Self::focus_down))
            // A press anywhere in the tab is pointer input: the zone it
            // focuses shows no ring. It goes on to whatever was pressed.
            .capture_any_mouse_down(|_, window, cx| brindle_ui::set_modality(window, Modality::Pointer, cx))
            .relative()
            .size_full()
            .p(px(ZONE_GAP))
            .bg(tokens.background)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    /// A config of `profiles` (name, agent?), `default` the default's name.
    fn kinds_config(profiles: &[(&str, bool)], default: Option<&str>) -> Config {
        let profiles = profiles
            .iter()
            .map(|(name, agent)| {
                let kind = if *agent { "type = \"agent\"\ncommand = \"agent\"\n" } else { "" };
                format!("[[profiles]]\nname = \"{name}\"\n{kind}\n")
            })
            .collect::<String>();
        let default = default.map(|name| format!("default_profile = \"{name}\"\n")).unwrap_or_default();
        Config::parse(&format!("{default}{profiles}")).expect("the test config").config
    }

    fn run(name: &str, nth: usize) -> SplitChoice {
        SplitChoice::Run(ProfileKey { name: name.into(), nth })
    }

    /// Each rule of `split_choice`, for each kind: the zone's own profile,
    /// its profile gone or of the other kind, the default of the kind, the
    /// only one of the kind, several, none.
    #[test]
    fn a_split_runs_the_first_profile_its_rules_give() {
        use ProfileKind::{Agent, Terminal};
        let config = kinds_config(&[("Shell", false), ("Work", false), ("Claude", true)], Some("Shell"));
        assert_eq!(split_choice(Terminal, Some(1), &config), run("Work", 0), "the zone's own profile");
        assert_eq!(split_choice(Agent, Some(2), &config), run("Claude", 0), "an agent zone's own profile");
        assert_eq!(split_choice(Terminal, None, &config), run("Shell", 0), "its profile gone: the default");
        assert_eq!(split_choice(Terminal, Some(2), &config), run("Shell", 0), "of the other kind: the default");
        assert_eq!(split_choice(Agent, Some(0), &config), run("Claude", 0), "the only agent profile");
        let config = kinds_config(&[("Shell", false), ("Claude", true), ("Codex", true)], Some("Shell"));
        let keys = |names: &[&str]| names.iter().map(|n| ProfileKey { name: (*n).into(), nth: 0 }).collect();
        assert_eq!(split_choice(Agent, Some(0), &config), SplitChoice::Ask(keys(&["Claude", "Codex"])), "several");
        assert_eq!(split_choice(Agent, Some(2), &config), run("Codex", 0), "several, the zone's own first");
        let config = kinds_config(&[("Claude", true), ("Shell", false), ("Work", false)], Some("Claude"));
        assert_eq!(split_choice(Terminal, Some(0), &config), SplitChoice::Ask(keys(&["Shell", "Work"])));
        let config = kinds_config(&[("Shell", false), ("Work", false), ("Claude", true)], Some("Work"));
        assert_eq!(split_choice(Terminal, Some(2), &config), run("Work", 0), "the default of the kind");
        let config = kinds_config(&[("Shell", false)], None);
        assert_eq!(split_choice(Agent, Some(0), &config), SplitChoice::Refused(SplitRefusal::NoProfileOfKind(Agent)));
        assert_eq!(SplitRefusal::NoProfileOfKind(Agent).text(), "no agent profile to split with");
        let config = kinds_config(&[("Claude", true)], None);
        assert_eq!(split_choice(Terminal, Some(0), &config), SplitChoice::Refused(SplitRefusal::NoProfileOfKind(Terminal)));
    }

    #[test]
    fn split_with_profile_lists_every_profile_the_default_first() {
        let config = kinds_config(&[("Shell", false), ("Claude", true), ("Work", false)], Some("Work"));
        let names: Vec<String> = split_profile_choices(&config).into_iter().map(|key| key.name).collect();
        assert_eq!(names, ["Work", "Shell", "Claude"]);
    }

    /// A split of the zone's own profile stays on its endpoint after the
    /// profile's target moved; another profile runs where it says; a
    /// directory never crosses endpoints.
    #[test]
    fn a_split_runs_where_its_profile_says_and_keeps_a_directory_on_its_host() {
        let ssh = |host: &str| Config::parse(&format!(
            "[[profiles]]\nname = \"Remote\"\n[profiles.remote]\ntype = \"ssh\"\nhost = \"{host}\"\n"
        ))
        .expect("the test config")
        .config
        .profiles
        .remove(0);
        let (old, moved) = (ssh("old-host"), ssh("new-host"));
        let local = Profile { name: "Shell".into(), ..Profile::default() };
        let on_old = Host::of(&old);
        assert_eq!(split_host(&old, &on_old, &moved), on_old, "its own profile stays where the zone runs");
        assert_eq!(split_host(&old, &on_old, &local), Host::Local, "another profile runs where it says");
        assert_eq!(split_host(&local, &Host::Local, &moved), Host::of(&moved));
        let here = Some(PathBuf::from("/srv/app"));
        assert_eq!(split_cwd(here.clone(), true), here, "the same host: the split zone's directory");
        assert_eq!(split_cwd(here.clone(), Host::Local == on_old), None, "remote to local");
        assert_eq!(split_cwd(here.clone(), on_old == Host::of(&moved)), None, "between two remotes");
        assert_eq!(split_cwd(None, true), None);
    }

    #[test]
    fn tree_split_remove_and_rects() {
        let z1 = ZoneId(1);
        let z2 = ZoneId(2);
        let z3 = ZoneId(3);
        let mut tree = ZoneTree::new(z1);
        assert_eq!(tree.leaves(), vec![z1]);
        assert_eq!(tree.rects(), vec![(z1, UnitRect::FULL)]);

        // Split z1 right -> z1 (left) and z2 (right).
        assert!(tree.split(z1, SplitAxis::Horizontal, z2));
        assert_eq!(
            tree.rects(),
            vec![
                (z1, UnitRect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }),
                (z2, UnitRect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }),
            ]
        );

        // Split z2 down -> z2 (top-right) and z3 (bottom-right).
        assert!(tree.split(z2, SplitAxis::Vertical, z3));
        assert_eq!(
            tree.rects(),
            vec![
                (z1, UnitRect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }),
                (z2, UnitRect { x: 0.5, y: 0.0, w: 0.5, h: 0.5 }),
                (z3, UnitRect { x: 0.5, y: 0.5, w: 0.5, h: 0.5 }),
            ]
        );

        // Removing z1 promotes the right subtree (z2 and z3) to fill the tab.
        let promoted = tree.remove(z1).unwrap();
        assert_eq!(promoted, vec![z2, z3]);
        assert_eq!(
            tree.rects(),
            vec![
                (z2, UnitRect { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }),
                (z3, UnitRect { x: 0.0, y: 0.5, w: 1.0, h: 0.5 }),
            ]
        );

        // Removing z2 leaves z3 as the single root leaf.
        let promoted = tree.remove(z2).unwrap();
        assert_eq!(promoted, vec![z3]);
        assert_eq!(tree, ZoneTree::Leaf(z3));
        assert_eq!(tree.remove(z3), None);
    }

    #[test]
    fn directional_focus_navigation() {
        let z1 = ZoneId(1);
        let z2 = ZoneId(2);
        let z3 = ZoneId(3);
        let mut tree = ZoneTree::new(z1);
        tree.split(z1, SplitAxis::Horizontal, z2);

        // Side-by-side moves and outer-edge no-ops.
        assert_eq!(tree.focus_target(z1, Direction::Right), Some(z2));
        assert_eq!(tree.focus_target(z2, Direction::Left), Some(z1));
        assert_eq!(tree.focus_target(z1, Direction::Left), None);
        assert_eq!(tree.focus_target(z2, Direction::Right), None);
        assert_eq!(tree.focus_target(z1, Direction::Up), None);
        assert_eq!(tree.focus_target(z1, Direction::Down), None);

        // Split right half into z2 (top-right) and z3 (bottom-right).
        // Moving right from tall left z1 breaks center-distance tie by tree order -> z2.
        tree.split(z2, SplitAxis::Vertical, z3);
        assert_eq!(tree.focus_target(z1, Direction::Right), Some(z2));
        assert_eq!(tree.focus_target(z2, Direction::Down), Some(z3));
        assert_eq!(tree.focus_target(z3, Direction::Up), Some(z2));
        assert_eq!(tree.focus_target(z2, Direction::Left), Some(z1));
        assert_eq!(tree.focus_target(z3, Direction::Left), Some(z1));

        // When left half is split into top-quarter and bottom-three-quarters
        // (via two vertical splits on left), perpendicular center picks the closer one.
        let z4 = ZoneId(4);
        let z5 = ZoneId(5);
        tree.split(z1, SplitAxis::Vertical, z4); // z1 top-left, z4 bottom-left
        tree.split(z4, SplitAxis::Vertical, z5); // z4 mid-left (0.5..0.75), z5 bottom-left (0.75..1.0)
        // From z5 (center_y = 0.875), moving right goes to z3 (center_y = 0.75) rather than z2 (0.25).
        assert_eq!(tree.focus_target(z5, Direction::Right), Some(z3));
        // From z1 (center_y = 0.25), moving right goes to z2 (center_y = 0.25).
        assert_eq!(tree.focus_target(z1, Direction::Right), Some(z2));
    }

    /// The header menu's items, labelled from the actions table, with the
    /// split items disabled where the zone is too small for them (the
    /// spec's "Too small to split": all disabled, Close Zone enabled) and
    /// a kind's where the config has no profile of it ("No agent profile").
    #[test]
    fn zone_menu_items() {
        let all = MenuSplits { right: true, down: true, terminal: true, agent: true };
        let menu_of = |rows: Vec<ActionRow>| {
            let shortcuts: Vec<Option<gpui::SharedString>> = rows
                .iter()
                .map(|row| row.action().filter(|a| a.partial_eq(&CloseZone)).map(|_| "ctrl-shift-x".into()))
                .collect();
            ActionMenu::new(rows, &shortcuts)
        };
        let describe = |splits| {
            let menu = menu_of(zone_menu_rows(splits));
            menu.entries
                .iter()
                .map(|e| match e.item() {
                    None => "-".to_string(),
                    Some(item) if item.disabled => format!("[{}]", item.label),
                    Some(item) => item.label.to_string(),
                })
                .collect::<Vec<_>>()
                .join("|")
        };
        assert_eq!(
            describe(all),
            "Split Terminal Right|Split Terminal Down|Split Agent Right|Split Agent Down\
             |Split Right With Profile…|Split Down With Profile…|-|Close Zone"
        );
        assert_eq!(
            describe(MenuSplits { right: false, ..all }),
            "[Split Terminal Right]|Split Terminal Down|[Split Agent Right]|Split Agent Down\
             |[Split Right With Profile…]|Split Down With Profile…|-|Close Zone"
        );
        assert_eq!(
            describe(MenuSplits { down: false, ..all }),
            "Split Terminal Right|[Split Terminal Down]|Split Agent Right|[Split Agent Down]\
             |Split Right With Profile…|[Split Down With Profile…]|-|Close Zone"
        );
        assert_eq!(
            describe(MenuSplits { right: false, down: false, ..all }),
            "[Split Terminal Right]|[Split Terminal Down]|[Split Agent Right]|[Split Agent Down]\
             |[Split Right With Profile…]|[Split Down With Profile…]|-|Close Zone"
        );
        assert_eq!(
            describe(MenuSplits { agent: false, ..all }),
            "Split Terminal Right|Split Terminal Down|[Split Agent Right]|[Split Agent Down]\
             |Split Right With Profile…|Split Down With Profile…|-|Close Zone"
        );
        assert_eq!(
            describe(MenuSplits { terminal: false, ..all }),
            "[Split Terminal Right]|[Split Terminal Down]|Split Agent Right|Split Agent Down\
             |Split Right With Profile…|Split Down With Profile…|-|Close Zone"
        );
        let menu = menu_of(zone_menu_rows(MenuSplits { down: false, ..all }));
        assert!(menu.action(0).is_some_and(|a| a.partial_eq(&SplitTerminalRight)));
        assert!(menu.action(1).is_none(), "a disabled split runs nothing");
        assert!(menu.action(2).is_some_and(|a| a.partial_eq(&SplitAgentRight)));
        assert!(menu.action(4).is_some_and(|a| a.partial_eq(&SplitRightWithProfile)));
        assert!(menu.action(7).is_some_and(|a| a.partial_eq(&CloseZone)));
        assert_eq!(menu.entries[7].item().and_then(|i| i.shortcut.clone()), Some("ctrl-shift-x".into()));
    }

    /// A zone's header buttons split it as its own kind.
    #[test]
    fn a_zones_buttons_split_as_its_kind() {
        let (right, down) = split_actions(ProfileKind::Terminal);
        assert!(right.partial_eq(&SplitTerminalRight) && down.partial_eq(&SplitTerminalDown));
        let (right, down) = split_actions(ProfileKind::Agent);
        assert!(right.partial_eq(&SplitAgentRight) && down.partial_eq(&SplitAgentDown));
    }

    #[test]
    fn minimum_split_size_and_describe_zones() {
        let cw = px(8.0);
        let lh = px(16.0);
        let pad = px(4.0);
        // The split math counts the card's real border and header.
        assert_eq!(px(ZONE_HEADER_HEIGHT), brindle_ui::CARD_HEADER_HEIGHT);
        assert_eq!(brindle_ui::CARD_BORDER, px(1.0));
        // Min terminal is 10*8 + 2*4 = 88px wide (MIN_COLS columns), 2*16 + 2*4 = 40px high.
        // With the card's 1px border and its header (28px), each zone card
        // needs >= 90px wide and >= 70px high.
        // Horizontal split needs width >= 2 * 90 + 6 (gap) = 186px, height >= 70px.
        // Vertical split needs width >= 90px, height >= 2 * 70 + 6 = 146px.
        assert_eq!(MIN_COLS, 10);
        assert!(can_split_zone(size(px(186.0), px(70.0)), false, SplitAxis::Horizontal, pad, cw, lh));
        assert!(!can_split_zone(size(px(185.0), px(70.0)), false, SplitAxis::Horizontal, pad, cw, lh));
        assert!(!can_split_zone(size(px(300.0), px(69.0)), false, SplitAxis::Horizontal, pad, cw, lh));

        assert!(can_split_zone(size(px(90.0), px(146.0)), true, SplitAxis::Vertical, pad, cw, lh));
        assert!(!can_split_zone(size(px(90.0), px(145.0)), true, SplitAxis::Vertical, pad, cw, lh));
        assert!(!can_split_zone(size(px(89.0), px(146.0)), true, SplitAxis::Vertical, pad, cw, lh));

        // The spec's "Card chrome counts": a zone exactly as wide as two
        // minimum grids plus their padding (2 * 88), but not the gap and the
        // borders, can't split right; the same tall enough can't split down.
        assert!(!can_split_zone(size(px(176.0), px(500.0)), true, SplitAxis::Horizontal, pad, cw, lh));
        assert!(!can_split_zone(size(px(500.0), px(2.0 * (40.0 + 28.0))), true, SplitAxis::Vertical, pad, cw, lh));

        // describe_zone_tree is empty for 1 zone and prints normalized rects for 2+ zones.
        let z1 = ZoneId(1);
        let z2 = ZoneId(2);
        let mut tree = ZoneTree::new(z1);
        assert_eq!(describe_zone_tree(&tree, z1), "");

        tree.split(z1, SplitAxis::Horizontal, z2);
        assert_eq!(
            describe_zone_tree(&tree, z2),
            "--- zones count=2 active=2\n--- zone id=1 rect=0,0,0.5,1\n--- zone id=2 rect=0.5,0,0.5,1 active\n"
        );
    }

    /// Zone badge lines: none for one zone (it has no header), else one per
    /// zone with a badge, the active one first.
    #[test]
    fn zone_badge_lines_put_the_active_zone_first() {
        use crate::terminal::context::{Contexts, parse};
        let badge = |payload: &[u8]| {
            let mut contexts = Contexts::default();
            contexts.apply(parse(payload).unwrap());
            contexts.notable()
        };
        let root = badge(b"start=a;type=elevate;targetuser=root");
        let web = badge(b"start=c;type=container;container=web");
        assert_eq!(describe_zone_badges([(ZoneId(1), root.as_ref())], ZoneId(1)), "");
        let zones = [(ZoneId(3), web.as_ref()), (ZoneId(1), root.as_ref()), (ZoneId(2), None)];
        assert_eq!(
            describe_zone_badges(zones, ZoneId(3)),
            "--- badge zone=3 kind=elsewhere label=\"web\" draw=text\n\
             --- badge zone=1 kind=privileged label=\"root\" draw=icon\n"
        );
        assert_eq!(describe_zone_badges(zones, ZoneId(2)).lines().count(), 2);
    }

    /// The user's decision: a privileged context is a warning icon in the
    /// theme's yellow, its target user only in the tooltip; the others keep
    /// their muted label and have no tooltip. The dump says which.
    #[test]
    fn a_privileged_badge_draws_as_a_warning_icon() {
        use crate::terminal::context::{Contexts, parse};
        use brindle_ui::BadgeContent;
        let badge = |payload: &[u8]| {
            let mut contexts = Contexts::default();
            contexts.apply(parse(payload).unwrap());
            contexts.notable().unwrap()
        };
        let theme = crate::theme::builtin_theme("tokyo-night").unwrap();
        let root = badge(b"start=a;type=elevate;targetuser=root");
        let drawn = ui_badge(&root, &theme);
        assert_eq!(drawn.content(), &BadgeContent::Icon(Icon::TriangleAlert));
        assert_eq!(drawn.color(), theme.badge_color(BadgeKind::Privileged));
        assert_eq!(drawn.tooltip_text().map(|text| text.as_ref()), Some("Running as root"));
        assert_eq!(describe_badge(&root), "kind=privileged label=\"root\" draw=icon");
        let chpriv = badge(b"start=b;type=chpriv;targetuser=postgres");
        assert_eq!(BadgeDraw::of(chpriv.kind()), BadgeDraw::Icon);
        assert_eq!(ui_badge(&chpriv, &theme).tooltip_text().map(|text| text.as_ref()), Some("Running as postgres"));

        let web = badge(b"start=c;type=container;container=web");
        let drawn = ui_badge(&web, &theme);
        assert_eq!(drawn.content(), &BadgeContent::Label("web".into()));
        assert_eq!(drawn.color(), theme.badge_color(BadgeKind::Elsewhere));
        assert_eq!(drawn.tooltip_text(), None);
        assert_eq!(describe_badge(&web), "kind=elsewhere label=\"web\" draw=text");
    }

    /// A tab of one agent zone (task 6.1): no terminal paths, the agent's
    /// title and status, its directory, a record while its session is
    /// open, and a close ending the session.
    #[test]
    fn an_agent_zone_answers_every_question_without_a_terminal() {
        use crate::agent::test_support::{Call, Zone, app, ready, settle};
        let mut app = app();
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let profile = Profile::default_shell();
        let spec = ZoneSpec { profile, host: Host::Local, theme: Theme::default(), font_size: None };
        let (view, cx) = app.add_window_view(|window, cx| ZoneTreeView::new(spec, ZoneStart::Agent(agent), window, cx));
        let mut zone = zone;
        zone.feed(&[ready()]);
        settle(cx);
        view.update(cx, |view, cx| {
            assert!(view.active_terminal_view().is_none());
            assert!(view.active_terminal(cx).is_none());
            assert!(view.terminals(cx).is_empty());
            let content = view.active_content().expect("the agent zone");
            assert!(content.terminal_view().is_none());
            // The agent's profile's name, not the agent's own.
            assert_eq!(content.title("Shell", cx).as_ref(), "Fake");
            assert_eq!(content.working_directory(cx), None);
            assert!(!content.unrecorded(cx), "an open session is recorded");
            assert_eq!(view.zone_records(cx).len(), 1);
            let (_, progress, badge) = content.status(cx);
            assert!(progress.is_none() && badge.is_none());
        });
        view.update(cx, |view, cx| view.shutdown_all(cx));
        assert_eq!(zone.take_calls(), vec![Call::Close]);
    }

    use std::cell::Cell;

    use crate::agent::close::describe_busy;

    /// Puts `toml`'s config in the test app's settings.
    fn use_config(cx: &mut gpui::TestAppContext, toml: &str) {
        let config = Config::parse(toml).expect("the test config").config;
        cx.update(|cx| cx.set_global(Settings::new(config, crate::config::ConfigReport::default(), Vec::new())));
    }

    /// Whether each split item of a zone menu is enabled.
    fn split_items(rows: &[ActionRow]) -> Vec<bool> {
        rows.iter()
            .filter_map(|row| match row {
                ActionRow::Item { action, enabled } if action.name() != CloseZone.name() => Some(*enabled),
                ActionRow::Item { .. } | ActionRow::Separator => None,
            })
            .collect()
    }

    /// A tab of one zone in a test window, and the split items of every
    /// menu it asks for.
    struct Tab {
        view: Entity<ZoneTreeView>,
        cx: &'static mut gpui::VisualTestContext,
        menus: Rc<RefCell<Vec<Vec<bool>>>>,
        /// The token set each menu request carried.
        menu_sets: Rc<RefCell<Vec<Option<Tokens>>>>,
        exited: Rc<Cell<bool>>,
        _subscriptions: Vec<Subscription>,
    }

    impl Tab {
        fn new(app: &mut gpui::TestAppContext, profile: Profile, start: impl FnOnce(&mut App) -> ZoneStart) -> Self {
            Self::themed(app, profile, Theme::default(), start)
        }

        /// A tab whose theme (its profile's) is `theme`.
        fn themed(
            app: &mut gpui::TestAppContext,
            profile: Profile,
            theme: Theme,
            start: impl FnOnce(&mut App) -> ZoneStart,
        ) -> Self {
            let start = app.update(start);
            let spec = ZoneSpec { profile, host: Host::Local, theme, font_size: None };
            let (view, cx) = app.add_window_view(|window, cx| ZoneTreeView::new(spec, start, window, cx));
            let cx: &'static mut gpui::VisualTestContext = Box::leak(Box::new(cx.clone()));
            let (menus, exited) = (Rc::<RefCell<Vec<Vec<bool>>>>::default(), Rc::new(Cell::new(false)));
            let menu_sets = Rc::<RefCell<Vec<Option<Tokens>>>>::default();
            let subscriptions = cx.update(|_, cx| {
                let menus = menus.clone();
                let menu_sets = menu_sets.clone();
                let exited = exited.clone();
                vec![
                    cx.subscribe(&view, move |_, request: &ZoneMenuRequest, _| {
                        menus.borrow_mut().push(split_items(&request.rows));
                        menu_sets.borrow_mut().push(request.tokens);
                    }),
                    cx.subscribe(&view, move |_, event: &TerminalEvent, _| {
                        if matches!(event, TerminalEvent::Exited) {
                            exited.set(true);
                        }
                    }),
                ]
            });
            Self { view, cx, menus, menu_sets, exited, _subscriptions: subscriptions }
        }

        /// The only zone's scope and card look, and the set the menu it
        /// asks for carries, with `window` as the window's set.
        fn look(&mut self, window: &Tokens) -> (Option<Tokens>, CardColors, Option<Tokens>) {
            let window = *window;
            let (scope, colors) = self.view.read_with(self.cx, |view, _| {
                let (colors, scope) = view.leaf_look(view.active, &window);
                (scope, colors)
            });
            self.view.update_in(self.cx, |view, window, cx| view.open_menu(view.active, Point::default(), None, window, cx));
            let menu = self.menu_sets.borrow_mut().pop().expect("the zone asked for its menu");
            (scope, colors, menu)
        }

        /// Which of the active zone's menu's six split items are enabled.
        fn splits(&mut self) -> Vec<bool> {
            let view = self.view.clone();
            view.update_in(self.cx, |view, window, cx| view.open_menu(view.active, Point::default(), None, window, cx));
            self.menus.borrow_mut().pop().expect("the zone asked for its menu")
        }

        /// Whether the active zone's header button splitting it as `kind`
        /// is enabled.
        fn button(&mut self, kind: ProfileKind) -> bool {
            self.view.read_with(self.cx, |view, cx| view.can_split_with(view.active, kind, cx))
        }

        fn zones(&mut self) -> usize {
            self.view.read_with(self.cx, |view, _| view.zones.len())
        }
    }

    /// A zone splits by the config's profiles, not by its own: an agent
    /// zone whose profile is gone, in a config with the agent profile
    /// `Other`, has every split enabled (the spec's "Agent tab without its
    /// agent profile"); with no agent profile in the config its agent
    /// items and its own button are disabled, and Split Agent Right says
    /// why on the window's notice line and starts nothing ("No agent
    /// profile"). Its size isn't known yet, so it fits either way.
    #[test]
    fn a_zone_splits_by_the_configs_profiles_not_its_own() {
        use crate::agent::test_support::{Zone, app};
        let mut app = app();
        use_config(
            &mut app,
            "[[profiles]]\nname = \"Shell\"\n\n[[profiles]]\nname = \"Other\"\ntype = \"agent\"\ncommand = \"fake\"\n",
        );
        let gone = Profile { name: "Gone".into(), ..Profile::default_shell() };
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let mut tab = Tab::new(&mut app, gone.clone(), |_| ZoneStart::Agent(agent));
        assert_eq!(tab.splits(), vec![true; 6]);
        assert!(tab.button(ProfileKind::Agent) && tab.button(ProfileKind::Terminal));

        use_config(&mut app, "[[profiles]]\nname = \"Shell\"\n");
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let mut tab = Tab::new(&mut app, gone, |_| ZoneStart::Agent(agent));
        assert_eq!(tab.splits(), vec![true, true, false, false, true, true]);
        assert!(!tab.button(ProfileKind::Agent), "an agent zone's own button");
        let refused = Rc::<RefCell<Vec<SplitRefusal>>>::default();
        let _refused = tab.cx.update(|_, cx| {
            let refused = refused.clone();
            cx.subscribe(&tab.view, move |_, SplitRefused(why): &SplitRefused, _| refused.borrow_mut().push(*why))
        });
        tab.view.update_in(tab.cx, |view, window, cx| view.split_agent_right(&SplitAgentRight, window, cx));
        assert_eq!(*refused.borrow(), [SplitRefusal::NoProfileOfKind(ProfileKind::Agent)]);
        assert_eq!(refused.borrow()[0].text(), "no agent profile to split with");
        assert_eq!(tab.zones(), 1, "no zone was started");
    }

    /// The spec's "A profile theme unlike the window's" and "The ring and
    /// the status dot", at the view: an agent zone in a tab whose profile
    /// sets brindle-light is drawn in that theme's zone tokens, keeps the
    /// window's ring and asks for its menu in its set; a terminal zone gets
    /// no scope, no ring of its own and a menu in the window's set. A
    /// profile change brings the new theme's set.
    #[test]
    fn an_agent_zone_is_drawn_in_its_themes_zone_tokens_and_a_terminal_zone_in_none() {
        use crate::agent::test_support::{Zone, app};
        let mut app = app();
        let light = crate::theme::builtin_theme("brindle-light").unwrap();
        let window = app.update(|cx| cx.window_theme());
        assert_ne!(window, light.zone_tokens(), "the case needs a profile theme unlike the window's");

        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let mut tab = Tab::themed(&mut app, Profile::default_shell(), light.clone(), |_| ZoneStart::Agent(agent));
        let (scope, colors, menu) = tab.look(&window);
        assert_eq!(scope, Some(light.zone_tokens()));
        assert_eq!(colors.ring, Some(brindle_ui::focus_ring(&window)), "the ring is the window's");
        assert_eq!(colors.surround, Some(window.background));
        assert_eq!(colors.surface, light.card_colors().surface);
        assert_eq!(menu, Some(light.zone_tokens()));

        let dark = crate::theme::builtin_theme("tokyo-night").unwrap();
        tab.view.update(tab.cx, |view, cx| view.restyle(&|_| (dark.clone(), None), cx));
        assert_eq!(tab.look(&window).0, Some(dark.zone_tokens()), "a restyle refreshes the set");

        let mut terminal = Tab::themed(&mut app, Profile::default_shell(), light.clone(), |cx| {
            let config = Settings::get(cx).config.clone();
            ZoneStart::Terminal(cx.new(|cx| Terminal::message("shell", Theme::default(), &config, cx)))
        });
        let (scope, colors, menu) = terminal.look(&window);
        assert_eq!((scope, colors.ring, menu), (None, None, None));
    }

    /// An agent tab a launch opened (`brindle agent -p hi`, task 6.3) sends
    /// its first prompt once the agent is ready, and only then, once.
    #[test]
    fn a_launched_agent_tab_sends_its_first_prompt_once_ready() {
        use crate::agent::test_support::{Call, Zone, app, ready, settle};
        let mut app = app();
        let mut zone = Zone::new(Some("hi"), &mut app);
        let agent = zone.agent.clone();
        let tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent));
        settle(tab.cx);
        assert_eq!(zone.take_calls(), Vec::new(), "nothing before the agent is ready");
        zone.feed(&[ready()]);
        settle(tab.cx);
        assert_eq!(zone.take_calls(), vec![Call::Prompt("hi".into())]);
        zone.feed(&[ready()]);
        settle(tab.cx);
        assert_eq!(zone.take_calls(), Vec::new(), "sent once");
    }

    /// Close Zone (its key and the header menu's item, both the
    /// `CloseZone` action) and the header's × (`request_close_zone`) on a
    /// working agent zone ask the window first and close nothing; on an
    /// idle one they close it at once, ending its session.
    #[test]
    fn closing_a_busy_agent_zone_asks_and_an_idle_one_closes_at_once() {
        use crate::agent::test_support::{Call, Zone, app, ready, settle};
        use crate::agent::transcript::{Event, TurnState};
        let mut app = app();
        let mut zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent));
        let asks = Rc::<RefCell<Vec<CloseZoneAsk>>>::default();
        let _asked = tab.cx.update(|_, cx| {
            let asks = asks.clone();
            cx.subscribe(&tab.view, move |_, ask: &CloseZoneAsk, _| asks.borrow_mut().push(ask.clone()))
        });
        zone.feed(&[ready(), Event::Turn { state: TurnState::Working }]);
        settle(tab.cx);

        // The action, as ctrl-shift-x and the header menu's item run it.
        tab.view.update_in(tab.cx, |view, window, cx| view.close_active_zone(&CloseZone, window, cx));
        // The header's ×.
        tab.view.update_in(tab.cx, |view, window, cx| view.request_close_zone(view.active, window, cx));
        let asked: Vec<_> = asks.borrow().iter().map(|ask| (ask.zone, describe_busy(&ask.busy))).collect();
        assert_eq!(asked, [(ZoneId(1), "1:working".to_string()), (ZoneId(1), "1:working".to_string())]);
        assert!(!tab.exited.get(), "nothing closed");
        assert!(!zone.take_calls().contains(&Call::Close), "the session wasn't ended");

        // Closing the whole tab would name it too.
        let closing = tab.view.read_with(tab.cx, |view, cx| view.closing(cx));
        assert!(matches!(close_plan(&closing), ClosePlan::Ask(busy) if describe_busy(&busy) == "1:working"));

        // Once the turn has ended, the same close is at once.
        zone.feed(&[Event::Turn { state: TurnState::Ended { stop: crate::agent::transcript::StopReason::EndTurn } }]);
        settle(tab.cx);
        tab.view.update_in(tab.cx, |view, window, cx| view.close_active_zone(&CloseZone, window, cx));
        assert_eq!(asks.borrow().len(), 2, "nothing more asked");
        assert!(tab.exited.get(), "the only zone closed, and the tab with it");
        assert_eq!(zone.take_calls(), vec![Call::Close]);
    }

    /// A zone waiting on a permission request is busy too, and a terminal
    /// zone never is: what a tab's close names.
    #[test]
    fn a_tab_names_only_its_busy_agents() {
        use crate::agent::test_support::{Zone, app, ready, settle};
        use crate::agent::transcript::{Event, TurnState};
        let mut app = app();
        let mut zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent));
        zone.feed(&[ready(), Event::Turn { state: TurnState::Blocked }]);
        settle(tab.cx);
        let closing = tab.view.read_with(tab.cx, |view, cx| view.closing(cx));
        assert!(matches!(&closing[..], [Closing::Agent { name, .. }] if name == "Fake"));
        assert!(matches!(close_plan(&closing), ClosePlan::Ask(busy) if describe_busy(&busy) == "1:blocked"));

        let shell = Tab::new(&mut app, Profile::default_shell(), |cx| {
            let config = Settings::get(cx).config.clone();
            ZoneStart::Terminal(cx.new(|cx| Terminal::message("shell", Theme::default(), &config, cx)))
        });
        let closing = shell.view.read_with(shell.cx, |view, cx| view.closing(cx));
        assert_eq!(close_plan(&closing), ClosePlan::Now);
    }

    /// An agent zone's dot follows its activity, drops `done` once the
    /// user types into the prompt input, and shows nothing while idle: what
    /// the zone header draws and the tab's dot summarises.
    #[test]
    fn an_agent_zones_dot_follows_its_status_and_the_seen_rule() {
        use crate::agent::test_support::{Zone, app, ready, settle};
        use crate::agent::transcript::{Event, StopReason, TurnState};
        let mut app = app();
        let mut zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent.clone()));
        let dot = |tab: &Tab| {
            tab.view.read_with(tab.cx, |view, cx| (view.zones[&view.active].status, view.status_summary(cx)))
        };
        zone.feed(&[ready()]);
        settle(tab.cx);
        assert_eq!(dot(&tab), (None, None), "idle shows no dot");
        zone.feed(&[Event::Turn { state: TurnState::Blocked }]);
        settle(tab.cx);
        assert_eq!(dot(&tab), (Some(State::Blocked), Some(State::Blocked)));
        zone.feed(&[Event::Turn { state: TurnState::Ended { stop: StopReason::EndTurn } }]);
        settle(tab.cx);
        assert_eq!(dot(&tab), (Some(State::Done), Some(State::Done)));
        tab.cx.update(|_, cx| agent.update(cx, |agent, cx| agent.seen(cx)));
        settle(tab.cx);
        assert_eq!(dot(&tab), (None, None), "seen");
    }

    /// `agents-zone-attention` decision 3: a working agent zone makes its
    /// tab's face a spinner (`agent_working`), a terminal's zones never do;
    /// a tab of one zone has no header, so it lists no header spinner and
    /// doesn't want the motion clock for one.
    #[test]
    fn a_working_agent_spins_on_its_tabs_face() {
        use crate::agent::test_support::{Zone, app, ready, settle};
        use crate::agent::transcript::{Event, StopReason, TurnState};
        let mut app = app();
        let mut zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent.clone()));
        let face = |tab: &Tab| {
            tab.view.read_with(tab.cx, |view, cx| {
                (mark(view.status_summary(cx), view.agent_working()), view.describe_spinners())
            })
        };
        zone.feed(&[ready(), Event::Turn { state: TurnState::Working }]);
        settle(tab.cx);
        assert_eq!(face(&tab), (Mark::Spinner, String::new()));
        let id = tab.view.entity_id();
        assert!(!tab.cx.update(|_, cx| Motion::wants(id, cx)), "no header to spin");
        zone.feed(&[Event::Turn { state: TurnState::Ended { stop: StopReason::EndTurn } }]);
        settle(tab.cx);
        assert_eq!(face(&tab), (Mark::Dot(State::Done), String::new()), "the green dot once the turn ends");

        let shell = Tab::new(&mut app, Profile::default_shell(), |cx| {
            let config = Settings::get(cx).config.clone();
            ZoneStart::Terminal(cx.new(|cx| Terminal::message("shell", Theme::default(), &config, cx)))
        });
        assert!(!shell.view.read_with(shell.cx, |view, _| view.agent_working()), "a terminal never spins");
    }

    /// A renamed profile reaches an agent zone whose agent hasn't named
    /// itself: its title follows the new name.
    #[test]
    fn an_agent_zones_title_follows_its_profiles_rename() {
        use crate::agent::test_support::{Zone, app, settle};
        let mut app = app();
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent));
        let renamed = Profile { name: "Renamed".into(), ..Profile::default_shell() };
        tab.view.update(tab.cx, |view, cx| {
            view.follow_profiles(&|_| Some(renamed.clone()));
            view.restyle(&|_| (Theme::default(), None), cx);
        });
        settle(tab.cx);
        let title = tab.view.read_with(tab.cx, |view, _| view.zones[&view.active].title.clone());
        assert_eq!(title.as_ref(), "Renamed");
        assert_eq!(tab.view.read_with(tab.cx, |view, _| view.profile_on(&Host::Local)), Some("Renamed".into()));
    }

    /// A mixed tab (`zones-mixed-tabs` decisions 1, 5, 6): a terminal zone
    /// of `Shell` on this machine beside an agent zone of `Fake` on a
    /// remote. Each zone is drawn in its own profile's theme (only the
    /// agent zone in a scope), titled by its own profile while its program
    /// names nothing, and counted on its own endpoint; the tab's theme (its
    /// face's) is the active zone's, and a reload restyles each zone from
    /// its own profile.
    #[test]
    fn a_mixed_tabs_zones_each_keep_their_own_profile() {
        use crate::agent::test_support::{Zone, app, settle};
        use crate::config::RemoteTarget;
        let mut app = app();
        let light = crate::theme::builtin_theme("brindle-light").unwrap();
        let dark = crate::theme::builtin_theme("tokyo-night").unwrap();
        let window_tokens = app.update(|cx| cx.window_theme());
        let remote = Host::Remote(RemoteTarget::Ssh {
            host: "box".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        let shell = Profile { name: "Shell".into(), ..Profile::default_shell() };
        let fake = Profile { name: "Fake".into(), ..Profile::default_shell() };
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let terminal = app.update(|cx| {
            let config = Settings::get(cx).config.clone();
            cx.new(|cx| Terminal::message("", Theme::default(), &config, cx))
        });
        let (z1, z2) = (ZoneId(1), ZoneId(2));
        let mut tree = ZoneTree::new(z1);
        tree.split(z1, SplitAxis::Horizontal, z2);
        let zones = vec![
            (z1, ZoneSpec { profile: shell, host: Host::Local, theme: dark.clone(), font_size: None }, ZoneStart::Terminal(terminal)),
            (z2, ZoneSpec { profile: fake, host: remote.clone(), theme: light.clone(), font_size: None }, ZoneStart::Agent(agent)),
        ];
        let (view, cx) = app.add_window_view(|window, cx| ZoneTreeView::from_restored(tree, zones, z2, window, cx));
        settle(cx);
        view.read_with(cx, |view, _| {
            let (first, first_scope) = view.leaf_look(z1, &window_tokens);
            let (second, second_scope) = view.leaf_look(z2, &window_tokens);
            assert_eq!(first.surface, dark.card_colors().surface, "the terminal zone in its profile's theme");
            assert_eq!(second.surface, light.card_colors().surface, "the agent zone in its profile's theme");
            assert_eq!((first_scope, second_scope), (None, Some(light.zone_tokens())));
            assert_eq!(view.active_theme(), light, "the tab's face is the active zone's");
            let titles: Vec<&str> = view.zones_in_order().iter().map(|zone| zone.title.as_ref()).collect();
            assert_eq!(titles, ["Shell", "Fake"]);
            assert_eq!(view.hosts(), [Host::Local, remote.clone()]);
            assert_eq!(view.profile_on(&Host::Local).as_deref(), Some("Shell"));
            assert_eq!(view.profile_on(&remote).as_deref(), Some("Fake"));
            assert_eq!(view.notifying_profile(), "Fake", "nothing notified: the active zone's");
        });
        view.update_in(cx, |view, window, cx| view.focus_zone(z1, window, cx));
        assert_eq!(view.read_with(cx, |view, _| view.active_theme()), dark, "the face follows the active zone");

        // A reload: each zone's profile gives its own look.
        view.update(cx, |view, cx| {
            view.restyle(&|profile| (if profile.name == "Shell" { light.clone() } else { dark.clone() }, None), cx);
        });
        view.read_with(cx, |view, _| {
            assert_eq!(view.leaf_look(z1, &window_tokens).0.surface, light.card_colors().surface);
            assert_eq!(view.leaf_look(z2, &window_tokens).1, Some(dark.zone_tokens()));
            assert_eq!(view.active_theme(), light);
        });
    }

    /// An agent zone whose agent exits stays open, its tab with it: the
    /// agent emits no `Exited`.
    #[test]
    fn an_agent_zone_stays_open_when_its_agent_exits() {
        use crate::agent::test_support::{Zone, app, ready, settle};
        let mut app = app();
        let mut zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let mut tab = Tab::new(&mut app, Profile::default_shell(), |_| ZoneStart::Agent(agent));
        zone.feed(&[ready()]);
        zone.sink.exited(Some(0));
        settle(tab.cx);
        assert_eq!(tab.zones(), 1);
        assert!(!tab.exited.get(), "the tab didn't close");
        let unrecorded = tab.view.read_with(tab.cx, |view, cx| view.zones[&view.active].content.unrecorded(cx));
        assert!(unrecorded, "an ended session isn't recorded");
    }

    /// Zone progress lines: none for one zone (the tab's line says it),
    /// else one per zone with progress, the active one first.
    #[test]
    fn zone_progress_lines_put_the_active_zone_first() {
        use crate::terminal::progress::Percent;
        let normal = Some(Progress::Normal(Percent::clamped(30)));
        let paused = Some(Progress::Paused(Some(Percent::clamped(70))));
        assert_eq!(describe_zone_progress([(ZoneId(1), normal)], ZoneId(1)), "");
        let zones = [(ZoneId(3), paused), (ZoneId(1), normal), (ZoneId(2), None)];
        assert_eq!(
            describe_zone_progress(zones, ZoneId(3)),
            "--- progress zone=3 state=paused percent=70\n--- progress zone=1 state=normal percent=30\n"
        );
        assert_eq!(
            describe_zone_progress(zones, ZoneId(1)),
            "--- progress zone=1 state=normal percent=30\n--- progress zone=3 state=paused percent=70\n"
        );
    }

    /// `agents-login` decision 9: a terminal login's zone goes right of its
    /// agent zone when that fits, else below, and is never recorded.
    #[test]
    fn a_login_zone_goes_beside_and_is_never_recorded() {
        assert_eq!(login_axis(true), SplitAxis::Horizontal);
        assert_eq!(login_axis(false), SplitAxis::Vertical);
        let leaves = vec![ZoneId(1), ZoneId(2), ZoneId(3)];
        assert_eq!(recorded_leaves(leaves.clone(), &[]), leaves);
        assert_eq!(recorded_leaves(leaves, &[ZoneId(2)]), [ZoneId(1), ZoneId(3)]);
        // The tree without the login zone is the layout a restore rebuilds.
        let mut tree = ZoneTree::Leaf(ZoneId(1));
        assert!(tree.split(ZoneId(1), login_axis(true), ZoneId(2)));
        assert_eq!(recorded_leaves(tree.leaves(), &[ZoneId(2)]), [ZoneId(1)]);
    }
}
