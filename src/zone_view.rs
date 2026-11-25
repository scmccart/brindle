//! A local tab's binary split tree of zones, normalized geometry, and
//! native GPUI zone chrome.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use brindle_ui::{
    ActiveTheme as _, Button, ButtonSize, ButtonVariant, Card, CardHeader, ContextMenuTarget as _, EdgeBar, Icon,
    Modality, Tokens, Tooltip, TooltipFor as _, modality, shows_ring,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render,
    SharedString, Size, Styled, Subscription, Window, div, px, size,
};
use serde::{Deserialize, Serialize};

use crate::actions::*;
use crate::TabKind;
use crate::agent::Agent;
use crate::agent::spawn::OpenedBy;
use crate::agent::view::AgentView;
use brindle_ui::FocusStops as _;
use gpui::Focusable as _;
use crate::config::{Config, Profile, ProfileKind};
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

/// What a split of a tab starts (design.md decision 4): a zone of the
/// tab's kind, or nothing, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitPlan {
    Terminal,
    Agent,
    Refused(SplitRefusal),
}

/// Why a tab can't start a zone of its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRefusal {
    /// The tab's profile is gone from the config.
    ProfileGone,
    /// The tab's profile, as the config has it now, is of the other kind.
    ProfileKindChanged,
}

/// What splitting a tab of kind `tab` starts, `profile` being the kind of
/// the tab's profile as the config has it now (`None`: gone). A tab starts
/// a zone of its own kind only, from a profile of that kind.
pub fn split_plan(tab: TabKind, profile: Option<ProfileKind>) -> SplitPlan {
    match (tab, profile) {
        (TabKind::Terminal, Some(ProfileKind::Terminal)) => SplitPlan::Terminal,
        (TabKind::Agent, Some(ProfileKind::Agent)) => SplitPlan::Agent,
        (TabKind::Terminal, Some(ProfileKind::Agent)) | (TabKind::Agent, Some(ProfileKind::Terminal)) => {
            SplitPlan::Refused(SplitRefusal::ProfileKindChanged)
        }
        (TabKind::Terminal | TabKind::Agent, None) => SplitPlan::Refused(SplitRefusal::ProfileGone),
    }
}

/// A zone header's menu: Split Zone Right, Split Zone Down, a separator and
/// Close Zone. Each split item is enabled only when the zone can be split
/// along its axis (`split_right`, `split_down`: [`can_split_zone`] for the
/// zone's current size); Close Zone always is.
pub fn zone_menu_rows(split_right: bool, split_down: bool) -> Vec<ActionRow> {
    vec![
        ActionRow::item(SplitZoneRight, split_right),
        ActionRow::item(SplitZoneDown, split_down),
        ActionRow::Separator,
        ActionRow::item(CloseZone, true),
    ]
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

impl ZoneStart {
    /// The kind of tab a zone like this one belongs in.
    pub fn kind(&self) -> TabKind {
        match self {
            Self::Terminal(_) => TabKind::Terminal,
            Self::Agent(_) => TabKind::Agent,
        }
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
    /// The terminal's view; `None` for an agent zone.
    pub fn terminal_view(&self) -> Option<&Entity<TerminalView>> {
        match self {
            Self::Terminal(view) => Some(view),
            Self::Agent(_) => None,
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
    /// isn't open any more (ended, never started, lost or broken).
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

    /// The status the zone shows, its progress and its context badge: a
    /// terminal's program status, or the agent zone's shown status with no
    /// progress or badge.
    pub fn status(&self, cx: &App) -> (Option<State>, Option<Progress>, Option<Badge>) {
        match self {
            Self::Terminal(view) => {
                let terminal = view.read(cx).terminal.read(cx);
                (terminal.status().summary(), terminal.progress(), terminal.badge().cloned())
            }
            Self::Agent(view) => (view.read(cx).agent().read(cx).status(), None, None),
        }
    }

    /// Takes the tab's profile's theme and font size.
    pub fn set_profile(&self, theme: Theme, font_size: Option<f32>, cx: &mut App) {
        match self {
            Self::Terminal(view) => view.update(cx, |view, cx| {
                view.font_size_override = font_size;
                view.terminal.update(cx, |t, _| t.theme = theme);
            }),
            Self::Agent(view) => view.update(cx, |view, cx| view.set_profile(theme, font_size, cx)),
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
    pub title: SharedString,
    pub status: Option<State>,
    /// Its terminal's progress, cached for its header's bar.
    pub progress: Option<Progress>,
    /// Its terminal's context badge, cached for its header and the tab's
    /// face; both refreshed on its terminal's `Status`.
    pub badge: Option<Badge>,
    pub last_focused: u64,
    _subscriptions: Vec<Subscription>,
}

pub struct ZoneTreeView {
    /// The tab's profile, as the config has it now: what a split runs.
    pub profile: Profile,
    /// The endpoint the tab opened on, where its splits start too, whatever
    /// the profile targets now. It never changes.
    host: Host,
    /// What the tab's zones are, fixed when it opens as its host is: a
    /// split starts a zone of this kind only ([`split_plan`]).
    kind: TabKind,
    /// The tab's theme, for its terminals and the cards round them (their
    /// headers, borders and the gaps between them). The focus ring and the
    /// status dots stay on the window's tokens and global theme.
    pub theme: Theme,
    pub font_size_override: Option<f32>,
    tree: ZoneTree,
    zones: HashMap<ZoneId, Zone>,
    active: ZoneId,
    next_id: u64,
    focus_serial: u64,
    shutting_down: bool,
    focus_handle: FocusHandle,
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
/// A zone's link wasn't opened, passed on to the window (whichever tab the
/// zone is in: an opener can fail after its tab went to the background).
impl EventEmitter<LinkFailed> for ZoneTreeView {}

impl ZoneTreeView {
    /// Creates a `ZoneTreeView` with a single initial zone, which runs on
    /// `host`.
    pub fn new(
        profile: Profile,
        host: Host,
        theme: Theme,
        font_size_override: Option<f32>,
        start: ZoneStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = ZoneId(1);
        let mut this = Self {
            profile,
            host,
            kind: start.kind(),
            theme,
            font_size_override,
            tree: ZoneTree::new(id),
            zones: HashMap::new(),
            active: id,
            next_id: 2,
            focus_serial: 0,
            shutting_down: false,
            focus_handle: cx.focus_handle(),
            laid_out: Rc::default(),
        };
        this.insert_zone(id, start, window, cx);
        this
    }

    /// Creates a `ZoneTreeView` from a restored split tree and the zones
    /// started for each leaf, which run on `host`.
    #[allow(
        clippy::too_many_arguments,
        reason = "both call sites (`Workspace`'s restore and layout tab) already hold each of these as separate values (profile, host, theme, tree, terminals, active zone); a parameter struct would be built only to be taken apart here"
    )]
    pub fn from_restored(
        profile: Profile,
        host: Host,
        theme: Theme,
        font_size_override: Option<f32>,
        tree: ZoneTree,
        zones: Vec<(ZoneId, ZoneStart)>,
        active: ZoneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let max_id = zones.iter().map(|(id, _)| id.0).max().unwrap_or(1);
        // The callers start every zone of a tree as one kind
        // (`layout::group` splits a recorded layout by kind).
        let kind = zones.first().map_or(TabKind::Terminal, |(_, start)| start.kind());
        let mut this = Self {
            profile,
            host,
            kind,
            theme,
            font_size_override,
            tree,
            zones: HashMap::new(),
            active,
            next_id: max_id + 1,
            focus_serial: 0,
            shutting_down: false,
            focus_handle: cx.focus_handle(),
            laid_out: Rc::default(),
        };
        for (id, start) in zones {
            this.insert_zone(id, start, window, cx);
        }
        // Give the recorded active zone the highest focus serial.
        this.focus_serial += 1;
        if let Some(zone) = this.zones.get_mut(&active) {
            zone.last_focused = this.focus_serial;
        }
        this
    }

    /// Puts zone `id` in the tree's map, showing what `start` started, in
    /// the tab's theme and font size, with the subscriptions its kind has.
    fn insert_zone(&mut self, id: ZoneId, start: ZoneStart, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme.clone();
        let font_size = self.font_size_override;
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
                    terminal_automatic_title(t.title(), &self.profile.name, LOCAL_PATH_STYLE, None).into()
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
                let title: SharedString = agent.read(cx).title().into();
                (ZoneContent::Agent(view), title, vec![events])
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
            title: initial_title,
            status,
            progress,
            badge,
            last_focused: self.focus_serial,
            _subscriptions: subscriptions,
        };
        self.zones.insert(id, zone);
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
                self.close_zone_by_id(id, window, cx);
            }
            TerminalEvent::Bell | TerminalEvent::Notify(_) | TerminalEvent::Notify99(_) => {
                cx.emit(event.clone());
            }
            TerminalEvent::Status(_) => {
                if let Some(zone) = self.zones.get_mut(&id) {
                    (zone.status, zone.progress, zone.badge) = zone.content.status(cx);
                }
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
        self.tree
            .leaves()
            .into_iter()
            .filter(|id| !unrecorded(id))
            .map(|id| {
                let record = crate::session::ZoneRecord {
                    cwd: self.zone_working_directory(id, cx),
                    session: self.zone_server_session(id, cx),
                };
                (id, record)
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
    /// cached it, or the profile name when there is no active zone: the
    /// tab's automatic title.
    pub fn active_title(&self) -> SharedString {
        match self.zones.get(&self.active) {
            Some(zone) => zone.title.clone(),
            None => self.profile.name.clone().into(),
        }
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
        let profile_name = self.profile.name.clone();
        let Some(zone) = self.zones.get_mut(&id) else { return false };
        let title = zone.content.title(&profile_name, cx);
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

    /// Takes the tab's profile as the config now has it (or the settings
    /// dialog previews it), with its theme and font size. The host stays.
    pub fn set_profile(&mut self, profile: Profile, theme: Theme, font_size: Option<f32>, cx: &mut Context<Self>) {
        self.profile = profile;
        self.theme = theme.clone();
        self.font_size_override = font_size;
        for zone in self.zones.values() {
            zone.content.set_profile(theme.clone(), font_size, cx);
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
        self.zones
            .keys()
            .filter_map(|&id| {
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
        if let Some(zone) = self.zones.get(&self.active) {
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

    /// What a split of this tab starts, from its profile as the config has
    /// it now ([`split_plan`]).
    fn split_plan(&self, cx: &App) -> SplitPlan {
        let config = &Settings::get(cx).config;
        split_plan(self.kind, config.profile(&self.profile.name).map(Profile::kind))
    }

    /// Whether zone `id` can be split along `axis`: the tab can start a
    /// zone of its kind ([`split_plan`]) and the zone is big enough, at its
    /// current size ([`can_split_zone`]). A zone whose size isn't known yet
    /// is.
    pub fn can_split(&self, id: ZoneId, axis: SplitAxis, window: &Window, cx: &App) -> bool {
        match self.split_plan(cx) {
            SplitPlan::Terminal | SplitPlan::Agent => {}
            SplitPlan::Refused(_) => return false,
        }
        let metrics = cell_metrics(self.font_size_override, window, cx);
        let padding = px(Settings::get(cx).config.padding);
        let has_header = self.zones.len() >= 2;
        self.zone_outer_size(id, window, cx).is_none_or(|zone_size| {
            can_split_zone(zone_size, has_header, axis, padding, metrics.cell_width, metrics.line_height)
        })
    }

    /// Focuses zone `id`, then asks the window for its header menu at `at`
    /// (from the secondary `press`, if one asked), its split items enabled
    /// as [`ZoneTreeView::can_split`] says. Focus comes first, so the items
    /// act on this zone.
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
        let rows = zone_menu_rows(
            self.can_split(id, SplitAxis::Horizontal, window, cx),
            self.can_split(id, SplitAxis::Vertical, window, cx),
        );
        cx.emit(ZoneMenuRequest { rows, at, press });
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

    pub fn split_zone(&mut self, target: ZoneId, axis: SplitAxis, window: &mut Window, cx: &mut Context<Self>) {
        if !self.zones.contains_key(&target) || !self.can_split(target, axis, window, cx) {
            return;
        }

        let config: Config = Settings::get(cx).config.clone();
        // A split runs on the tab's host and inherits the target zone's
        // current working directory there (an agent zone's: the one its
        // session started in); if unavailable, the spawn falls back to the
        // profile's cwd or home.
        let cwd = self.zone_working_directory(target, cx);
        let plan = self.split_plan(cx);

        let new_id = ZoneId(self.next_id);
        self.next_id += 1;
        if !self.tree.split(target, axis, new_id) {
            return;
        }

        let host = self.host.clone();
        let start = match plan {
            SplitPlan::Terminal => {
                ZoneStart::Terminal(cx.new(|cx| Terminal::spawn(&self.profile, &host, None, cwd, &config, cx)))
            }
            SplitPlan::Agent => {
                ZoneStart::Agent(cx.new(|cx| Agent::spawn(&self.profile, &host, cwd, OpenedBy::Window, None, cx)))
            }
            // `can_split` refused this above; should it get here, the tree
            // loses the leaf it just gained.
            SplitPlan::Refused(_) => {
                self.tree.remove(new_id);
                return;
            }
        };

        // Refresh the existing zone's title so its header bar has an up-to-date label.
        self.refresh_zone_title(target, cx);
        self.insert_zone(new_id, start, window, cx);
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
        let Some(removed) = self.zones.remove(&target) else { return };
        removed.content.close(cx);

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

    fn split_right(&mut self, _: &SplitZoneRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_zone(self.active, SplitAxis::Horizontal, window, cx);
    }

    fn split_down(&mut self, _: &SplitZoneDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_zone(self.active, SplitAxis::Vertical, window, cx);
    }

    fn close_active_zone(&mut self, _: &CloseZone, window: &mut Window, cx: &mut Context<Self>) {
        self.close_zone_by_id(self.active, window, cx);
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
        // background), so the corner cutouts match it.
        let mut colors = self.theme.card_colors();
        colors.surround = Some(tokens.background);
        let card =Card::new(("zone", id.0 as usize)).colors(colors).size_full().min_w_0().min_h_0();
        if self.zones.len() <= 1 {
            return card.child(view).into_any_element();
        }

        let active = id == self.active;
        let focus = zone.content.focus_handle(cx);
        let ring = active && shows_ring(zone.content.contains_focused(window, cx), modality(window, cx), false);
        // Status dots in the chrome theme's colours, like the strip's.
        let status_color =
            zone.status.map(|state| Settings::chrome_theme(cx).status_color(state, tokens.muted_foreground));
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
        // A tab that can't start a zone of its kind shows its split
        // buttons disabled (a zone too small to split finds out on a press,
        // as before: that needs the font's metrics).
        let refused = match self.split_plan(cx) {
            SplitPlan::Terminal | SplitPlan::Agent => false,
            SplitPlan::Refused(_) => true,
        };
        let split_right = button("zone-split-right", Icon::Columns2)
            .disabled(refused)
            .tooltip_with(tip("Split right", Box::new(SplitZoneRight)))
            .on_click(cx.listener(move |this, _, window, cx| this.split_zone(id, SplitAxis::Horizontal, window, cx)));
        let split_down = button("zone-split-down", Icon::Rows2)
            .disabled(refused)
            .tooltip_with(tip("Split down", Box::new(SplitZoneDown)))
            .on_click(cx.listener(move |this, _, window, cx| this.split_zone(id, SplitAxis::Vertical, window, cx)));
        let close = button("zone-close", Icon::X)
            .tooltip_with(tip("Close zone", Box::new(CloseZone)))
            .on_click(cx.listener(move |this, _, window, cx| this.close_zone_by_id(id, window, cx)));

        // The title in the terminal's foreground on the active zone, muted
        // on the others (`trigger_label` cuts it, as `truncate` can't).
        let mut header = CardHeader::new(("zone-header", id.0 as usize));
        if let Some(color) = status_color {
            header = header.leading(div().flex_none().size(px(6.0)).rounded_full().bg(color));
        }
        if let Some(bar) = progress_bar {
            header = header.edge_bar(bar);
        }
        // Its context badge, drawn as the tab face's is.
        if let Some(badge) = &zone.badge {
            header = header.badge(ui_badge(badge, &self.theme));
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
        card.header(header)
            .ring(ring)
            .on_header_bounds(move |bounds, _, _| {
                laid_out.borrow_mut().headers.insert(id, bounds);
            })
            .child(view)
            .into_any_element()
    }
}

impl Render for ZoneTreeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.tree.clone();
        // Headers are recorded again as this frame lays them out.
        self.laid_out.borrow_mut().headers.clear();
        let tokens = *cx.ui_theme();
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
            .on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::split_down))
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

    /// Every (tab kind, profile kind or gone) pair: a tab starts a zone of
    /// its own kind from a profile of that kind, and nothing otherwise.
    #[test]
    fn a_split_starts_a_zone_of_the_tabs_kind_only() {
        let table = [
            (TabKind::Terminal, Some(ProfileKind::Terminal), SplitPlan::Terminal),
            (TabKind::Terminal, Some(ProfileKind::Agent), SplitPlan::Refused(SplitRefusal::ProfileKindChanged)),
            (TabKind::Terminal, None, SplitPlan::Refused(SplitRefusal::ProfileGone)),
            (TabKind::Agent, Some(ProfileKind::Agent), SplitPlan::Agent),
            (TabKind::Agent, Some(ProfileKind::Terminal), SplitPlan::Refused(SplitRefusal::ProfileKindChanged)),
            (TabKind::Agent, None, SplitPlan::Refused(SplitRefusal::ProfileGone)),
        ];
        for (tab, profile, plan) in table {
            assert_eq!(split_plan(tab, profile), plan, "{tab:?} tab, {profile:?} profile");
        }
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
    /// spec's "Too small to split": both disabled, Close Zone enabled).
    #[test]
    fn zone_menu_items() {
        let menu_of = |rows: Vec<ActionRow>| {
            let shortcuts: Vec<Option<gpui::SharedString>> = rows
                .iter()
                .map(|row| row.action().filter(|a| a.partial_eq(&CloseZone)).map(|_| "ctrl-shift-x".into()))
                .collect();
            ActionMenu::new(rows, &shortcuts)
        };
        let describe = |right, down| {
            let menu = menu_of(zone_menu_rows(right, down));
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
        assert_eq!(describe(true, true), "Split Zone Right|Split Zone Down|-|Close Zone");
        assert_eq!(describe(false, true), "[Split Zone Right]|Split Zone Down|-|Close Zone");
        assert_eq!(describe(true, false), "Split Zone Right|[Split Zone Down]|-|Close Zone");
        assert_eq!(describe(false, false), "[Split Zone Right]|[Split Zone Down]|-|Close Zone");
        let menu = menu_of(zone_menu_rows(true, false));
        assert!(menu.action(0).is_some_and(|a| a.partial_eq(&SplitZoneRight)));
        assert!(menu.action(1).is_none(), "a disabled split runs nothing");
        assert!(menu.action(3).is_some_and(|a| a.partial_eq(&CloseZone)));
        assert_eq!(menu.entries[3].item().and_then(|i| i.shortcut.clone()), Some("ctrl-shift-x".into()));
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
        let (view, cx) = app.add_window_view(|window, cx| {
            ZoneTreeView::new(profile, Host::Local, Theme::default(), None, ZoneStart::Agent(agent), window, cx)
        });
        let mut zone = zone;
        zone.feed(&[ready()]);
        settle(cx);
        view.update(cx, |view, cx| {
            assert!(view.active_terminal_view().is_none());
            assert!(view.active_terminal(cx).is_none());
            assert!(view.terminals(cx).is_empty());
            let content = view.active_content().expect("the agent zone");
            assert!(content.terminal_view().is_none());
            assert_eq!(content.title("Shell", cx).as_ref(), "Fake Agent");
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
        exited: Rc<Cell<bool>>,
        _subscriptions: Vec<Subscription>,
    }

    impl Tab {
        fn new(app: &mut gpui::TestAppContext, profile: Profile, start: impl FnOnce(&mut App) -> ZoneStart) -> Self {
            let start = app.update(start);
            let (view, cx) = app.add_window_view(|window, cx| {
                ZoneTreeView::new(profile, Host::Local, Theme::default(), None, start, window, cx)
            });
            let cx: &'static mut gpui::VisualTestContext = Box::leak(Box::new(cx.clone()));
            let (menus, exited) = (Rc::<RefCell<Vec<Vec<bool>>>>::default(), Rc::new(Cell::new(false)));
            let subscriptions = cx.update(|_, cx| {
                let menus = menus.clone();
                let exited = exited.clone();
                vec![
                    cx.subscribe(&view, move |_, request: &ZoneMenuRequest, _| {
                        menus.borrow_mut().push(split_items(&request.rows))
                    }),
                    cx.subscribe(&view, move |_, event: &TerminalEvent, _| {
                        if matches!(event, TerminalEvent::Exited) {
                            exited.set(true);
                        }
                    }),
                ]
            });
            Self { view, cx, menus, exited, _subscriptions: subscriptions }
        }

        /// Whether the only zone can split right and down, as
        /// [`ZoneTreeView::can_split`] says and as its menu shows.
        fn splits(&mut self) -> (bool, bool, Vec<bool>) {
            let view = self.view.clone();
            let can = self.cx.update(|window, cx| {
                let view = view.read(cx);
                let id = view.active;
                (view.can_split(id, SplitAxis::Horizontal, window, cx), view.can_split(id, SplitAxis::Vertical, window, cx))
            });
            view.update_in(self.cx, |view, window, cx| view.open_menu(view.active, Point::default(), None, window, cx));
            let menu = self.menus.borrow_mut().pop().expect("the zone asked for its menu");
            (can.0, can.1, menu)
        }

        fn zones(&mut self) -> usize {
            self.view.read_with(self.cx, |view, _| view.zones.len())
        }
    }

    /// An agent tab whose profile is now a terminal profile, and a terminal
    /// tab whose profile is gone, refuse splits: `can_split` says no, the
    /// header menu's split items are disabled, and the split actions do
    /// nothing. An agent tab of an agent profile can split.
    #[test]
    fn a_tab_refuses_splits_of_the_other_kind_or_a_gone_profile() {
        use crate::agent::test_support::{Zone, app};
        let mut app = app();
        use_config(
            &mut app,
            "[[profiles]]\nname = \"Shell\"\n\n[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = \"fake\"\n",
        );
        let profile = |name: &str| app.update(|cx| Settings::get(cx).config.profile(name).cloned().unwrap());
        let (shell, fake) = (profile("Shell"), profile("Fake"));

        // An agent tab whose profile (by name) is a terminal profile now.
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let mut changed = Tab::new(&mut app, shell, |_| ZoneStart::Agent(agent));
        assert_eq!(changed.splits(), (false, false, vec![false, false]));
        changed.view.update_in(changed.cx, |view, window, cx| {
            view.split_right(&SplitZoneRight, window, cx);
            view.split_down(&SplitZoneDown, window, cx);
        });
        assert_eq!(changed.zones(), 1, "no zone was started");

        // A terminal tab whose profile is gone from the config.
        let mut gone_profile = Profile::default_shell();
        gone_profile.name = "Gone".into();
        let mut gone = Tab::new(&mut app, gone_profile, |cx| {
            let config = Settings::get(cx).config.clone();
            ZoneStart::Terminal(cx.new(|cx| Terminal::message("gone", Theme::default(), &config, cx)))
        });
        assert_eq!(gone.splits(), (false, false, vec![false, false]));
        gone.view.update_in(gone.cx, |view, window, cx| view.split_right(&SplitZoneRight, window, cx));
        assert_eq!(gone.zones(), 1, "no zone was started");

        // An agent tab of an agent profile: its size isn't known yet, so it
        // can split either way.
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let mut same = Tab::new(&mut app, fake, |_| ZoneStart::Agent(agent));
        assert_eq!(same.splits(), (true, true, vec![true, true]));
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
}
