//! A local tab's binary split tree of zones, normalized geometry, and
//! native GPUI zone chrome.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, CursorStyle, Entity, EventEmitter, FocusHandle,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    Size, StatefulInteractiveElement, Styled, Subscription, Window, div, px, size,
};
use serde::{Deserialize, Serialize};

use crate::actions::*;
use crate::config::{Config, Profile, TmuxMode};
use crate::settings::Settings;
use crate::terminal::process::Snapshot;
use crate::terminal::status::State;
use crate::terminal::{Terminal, TerminalEvent};
use crate::terminal_element::cell_metrics;
use crate::terminal_view::TerminalView;
use crate::theme::Theme;
use crate::tmux::Direction;
use crate::workspace::terminal_automatic_title;

/// Height of the native zone header bar shown when a tab has 2+ zones.
pub const ZONE_HEADER_HEIGHT: f32 = 28.0;
/// Thickness of the split divider between zones.
pub const ZONE_DIVIDER_SIZE: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneId(pub u64);

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

/// Whether a zone of outer pixel size `zone_size` (including its header bar
/// when `has_header` is true) can be split in half along `axis` while fitting
/// two minimum 2x2 terminal grids, the 1px divider, and a header bar on each
/// resulting zone.
pub fn can_split_zone(
    zone_size: Size<Pixels>,
    has_header: bool,
    axis: SplitAxis,
    padding: Pixels,
    cell_width: Pixels,
    line_height: Pixels,
) -> bool {
    let min_term_w = f32::from(cell_width) * 2.0 + f32::from(padding) * 2.0;
    let min_term_h = f32::from(line_height) * 2.0 + f32::from(padding) * 2.0;
    let min_zone_w = min_term_w;
    let min_zone_h = ZONE_HEADER_HEIGHT + min_term_h;

    let w = f32::from(zone_size.width);
    let h = f32::from(zone_size.height);
    let _ = has_header;

    match axis {
        SplitAxis::Horizontal => {
            let child_w = (w - ZONE_DIVIDER_SIZE) * 0.5;
            child_w >= min_zone_w && h >= min_zone_h
        }
        SplitAxis::Vertical => {
            let child_h = (h - ZONE_DIVIDER_SIZE) * 0.5;
            w >= min_zone_w && child_h >= min_zone_h
        }
    }
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

fn fmt_unit(v: f64) -> String {
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

/// What a zone holds. Starting with terminals; structured as an enum so future
/// zone types (diff viewers, previews, etc.) can live in the same split tree.
pub enum ZoneContent {
    Terminal(Entity<TerminalView>),
}

impl ZoneContent {
    pub fn as_terminal_view(&self) -> &Entity<TerminalView> {
        match self {
            Self::Terminal(view) => view,
        }
    }
}

pub struct Zone {
    pub content: ZoneContent,
    pub title: SharedString,
    pub status: Option<State>,
    pub last_focused: u64,
    _subscriptions: Vec<Subscription>,
}

pub struct ZoneTreeView {
    pub profile: Profile,
    pub theme: Theme,
    pub font_size_override: Option<f32>,
    tree: ZoneTree,
    zones: HashMap<ZoneId, Zone>,
    active: ZoneId,
    next_id: u64,
    focus_serial: u64,
    shutting_down: bool,
    focus_handle: FocusHandle,
}

impl EventEmitter<TerminalEvent> for ZoneTreeView {}

impl ZoneTreeView {
    /// Creates a `ZoneTreeView` with a single initial terminal zone.
    pub fn new(
        profile: Profile,
        theme: Theme,
        font_size_override: Option<f32>,
        terminal: Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = ZoneId(1);
        let mut this = Self {
            profile,
            theme,
            font_size_override,
            tree: ZoneTree::new(id),
            zones: HashMap::new(),
            active: id,
            next_id: 2,
            focus_serial: 0,
            shutting_down: false,
            focus_handle: cx.focus_handle(),
        };
        this.insert_terminal_zone(id, terminal, window, cx);
        this
    }

    /// Creates a `ZoneTreeView` from a restored split tree and pre-spawned
    /// terminals for each leaf.
    pub fn from_restored(
        profile: Profile,
        theme: Theme,
        font_size_override: Option<f32>,
        tree: ZoneTree,
        terminals: Vec<(ZoneId, Entity<Terminal>)>,
        active: ZoneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let max_id = terminals.iter().map(|(id, _)| id.0).max().unwrap_or(1);
        let mut this = Self {
            profile,
            theme,
            font_size_override,
            tree,
            zones: HashMap::new(),
            active,
            next_id: max_id + 1,
            focus_serial: 0,
            shutting_down: false,
            focus_handle: cx.focus_handle(),
        };
        for (id, terminal) in terminals {
            this.insert_terminal_zone(id, terminal, window, cx);
        }
        // Give the recorded active zone the highest focus serial.
        this.focus_serial += 1;
        if let Some(zone) = this.zones.get_mut(&active) {
            zone.last_focused = this.focus_serial;
        }
        this
    }

    fn insert_terminal_zone(
        &mut self,
        id: ZoneId,
        terminal: Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.theme.clone();
        terminal.update(cx, |t, _| t.theme = theme);
        let font_size = self.font_size_override;
        let view = cx.new(|cx| TerminalView::new(terminal.clone(), font_size, window, cx));
        let focus = view.read(cx).focus_handle.clone();
        let focus_in = cx.on_focus_in(&focus, window, move |this, _, cx| {
            this.on_zone_focused(id, cx);
        });
        let events = cx.subscribe_in(&terminal, window, move |this, _, event, window, cx| {
            this.on_zone_terminal_event(id, event, window, cx);
        });
        self.focus_serial += 1;
        let initial_title: SharedString = {
            let t = terminal.read(cx);
            terminal_automatic_title(t.title(), &self.profile.name, || None).into()
        };
        let status = terminal.read(cx).status().summary();
        let zone = Zone {
            content: ZoneContent::Terminal(view),
            title: initial_title,
            status,
            last_focused: self.focus_serial,
            _subscriptions: vec![focus_in, events],
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
            TerminalEvent::Bell | TerminalEvent::Notify(_) => {
                cx.emit(event.clone());
            }
            TerminalEvent::Status(_) => {
                if let Some(zone) = self.zones.get_mut(&id) {
                    let status = zone.content.as_terminal_view().read(cx).terminal.read(cx).status().summary();
                    zone.status = status;
                }
                cx.emit(event.clone());
                cx.notify();
            }
            TerminalEvent::TitleChanged => {
                let mut snapshot = None;
                let changed = self.refresh_zone_title(id, &mut snapshot, cx);
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
            .map(|z| z.content.as_terminal_view().read(cx).focus_handle.clone())
            .unwrap_or_else(|| self.focus_handle.clone())
    }

    pub fn active_view(&self) -> Option<Entity<TerminalView>> {
        self.zones.get(&self.active).map(|z| z.content.as_terminal_view().clone())
    }

    pub fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        Some(self.active_view()?.read(cx).terminal.clone())
    }

    pub fn active_id(&self) -> ZoneId {
        self.active
    }

    pub fn tree(&self) -> &ZoneTree {
        &self.tree
    }


    pub fn zone_working_directory(&self, id: ZoneId, cx: &App) -> Option<PathBuf> {
        let zone = self.zones.get(&id)?;
        zone.content.as_terminal_view().read(cx).terminal.read(cx).working_directory()
    }

    /// Automatic title for the active zone (used for the tab's automatic title).
    pub fn automatic_title(&self, snapshot: &mut Option<Snapshot>, cx: &App) -> String {
        let Some(zone) = self.zones.get(&self.active) else {
            return self.profile.name.clone();
        };
        let terminal = zone.content.as_terminal_view().read(cx).terminal.read(cx);
        terminal_automatic_title(terminal.title(), &self.profile.name, || {
            terminal.foreground_process_name(snapshot.get_or_insert_with(Snapshot::take))
        })
    }

    /// Refreshes cached zone header titles for all zones in this tab.
    pub fn refresh_titles(&mut self, snapshot: &mut Option<Snapshot>, cx: &App) -> bool {
        let ids: Vec<ZoneId> = self.zones.keys().copied().collect();
        let mut any = false;
        for id in ids {
            any |= self.refresh_zone_title(id, snapshot, cx);
        }
        any
    }

    fn refresh_zone_title(&mut self, id: ZoneId, snapshot: &mut Option<Snapshot>, cx: &App) -> bool {
        let profile_name = self.profile.name.clone();
        let Some(zone) = self.zones.get_mut(&id) else { return false };
        let terminal = zone.content.as_terminal_view().read(cx).terminal.read(cx);
        let title: SharedString = terminal_automatic_title(terminal.title(), &profile_name, || {
            terminal.foreground_process_name(snapshot.get_or_insert_with(Snapshot::take))
        })
        .into();
        let changed = zone.title != title;
        zone.title = title;
        changed
    }

    /// The most pressing program status across all zones in this tab.
    pub fn status_summary(&self, cx: &App) -> Option<State> {
        self.zones
            .values()
            .filter_map(|z| z.content.as_terminal_view().read(cx).terminal.read(cx).status().summary())
            .max()
    }

    pub fn set_profile(&mut self, profile: Profile, theme: Theme, font_size: Option<f32>, cx: &mut Context<Self>) {
        self.profile = profile;
        self.theme = theme.clone();
        self.font_size_override = font_size;
        for zone in self.zones.values() {
            let theme = theme.clone();
            zone.content.as_terminal_view().update(cx, |view, cx| {
                view.font_size_override = font_size;
                view.terminal.update(cx, |t, _| t.theme = theme);
            });
        }
        cx.notify();
    }

    /// Shuts down every zone's terminal when the tab itself is removed.
    pub fn shutdown_all(&mut self, cx: &mut Context<Self>) {
        self.shutting_down = true;
        let terminals: Vec<Entity<Terminal>> = self
            .zones
            .values()
            .map(|z| z.content.as_terminal_view().read(cx).terminal.clone())
            .collect();
        for terminal in terminals {
            terminal.update(cx, |t, _| t.shutdown());
        }
    }

    pub fn describe_zones(&self) -> String {
        describe_zone_tree(&self.tree, self.active)
    }

    /// Outer pixel size of `id` for the minimum-split-size check.
    fn zone_outer_size(&self, id: ZoneId, window: &Window, cx: &App) -> Option<Size<Pixels>> {
        let padding = px(Settings::get(cx).config.padding);
        let has_header = self.zones.len() >= 2;
        if let Some(grid) = self.zones.get(&id).and_then(|z| z.content.as_terminal_view().read(cx).layout.get()) {
            let w = grid.cell_width * grid.cols as f32 + padding * 2.0;
            let h = grid.line_height * grid.rows as f32
                + padding * 2.0
                + if has_header { px(ZONE_HEADER_HEIGHT) } else { px(0.0) };
            return Some(size(w, h));
        }
        // Fallback before the first paint: estimate from the window's inner bounds
        // and the zone's normalized rectangle.
        let (_, rect) = self.tree.rects().into_iter().find(|(zid, _)| *zid == id)?;
        let win_size = window.inner_window_bounds().get_bounds().size;
        let content_h = (f32::from(win_size.height) - 38.0).max(0.0);
        Some(size(px(f32::from(win_size.width) * rect.w as f32), px(content_h * rect.h as f32)))
    }

    pub fn split_zone(&mut self, target: ZoneId, axis: SplitAxis, window: &mut Window, cx: &mut Context<Self>) {
        if !self.zones.contains_key(&target) {
            return;
        }
        let metrics = cell_metrics(self.font_size_override, window, cx);
        let padding = px(Settings::get(cx).config.padding);
        let has_header = self.zones.len() >= 2;
        if let Some(zone_size) = self.zone_outer_size(target, window, cx)
            && !can_split_zone(zone_size, has_header, axis, padding, metrics.cell_width, metrics.line_height)
        {
            return;
        }

        let config: Config = Settings::get(cx).config.clone();
        // A split inherits the target zone's current working directory; if
        // unavailable, `Terminal::spawn_pty` falls back to the profile's cwd or home.
        let cwd = self.zone_working_directory(target, cx);
        let command = match self.profile.tmux {
            TmuxMode::Plain => {
                let mut args = self.profile.tmux_args.clone();
                args.extend([
                    "new-session".into(),
                    "-A".into(),
                    "-s".into(),
                    self.profile.tmux_session_name().to_string(),
                ]);
                Some(("tmux".into(), args))
            }
            _ => None,
        };

        let new_id = ZoneId(self.next_id);
        self.next_id += 1;
        if !self.tree.split(target, axis, new_id) {
            return;
        }

        let terminal = cx.new(|cx| Terminal::spawn_pty(&self.profile, command, cwd, &config, cx));
        // Refresh the existing zone's title so its header bar has an up-to-date label.
        let mut snapshot = None;
        self.refresh_zone_title(target, &mut snapshot, cx);
        self.insert_terminal_zone(new_id, terminal, window, cx);
        self.refresh_zone_title(new_id, &mut snapshot, cx);
        self.active = new_id;
        let focus = self.focus_handle(cx);
        window.focus(&focus);
        cx.emit(TerminalEvent::TitleChanged);
        cx.notify();
    }

    pub fn close_zone_by_id(&mut self, target: ZoneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(removed) = self.zones.remove(&target) else { return };
        removed.content.as_terminal_view().read(cx).terminal.clone().update(cx, |t, _| t.shutdown());

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

    fn render_tree(&self, node: &ZoneTree, cx: &mut Context<Self>) -> AnyElement {
        match node {
            ZoneTree::Leaf(id) => self.render_leaf(*id, cx),
            ZoneTree::Split { axis, first, second } => {
                let border = self.theme.chrome_border().hsla();
                let first_el = self.render_tree(first, cx);
                let second_el = self.render_tree(second, cx);
                let divider = match axis {
                    SplitAxis::Horizontal => div().flex_none().w(px(ZONE_DIVIDER_SIZE)).h_full().bg(border),
                    SplitAxis::Vertical => div().flex_none().h(px(ZONE_DIVIDER_SIZE)).w_full().bg(border),
                };
                div()
                    .flex()
                    .when(*axis == SplitAxis::Horizontal, |d| d.flex_row())
                    .when(*axis == SplitAxis::Vertical, |d| d.flex_col())
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().min_h_0().size_full().child(first_el))
                    .child(divider)
                    .child(div().flex_1().min_w_0().min_h_0().size_full().child(second_el))
                    .into_any_element()
            }
        }
    }

    fn render_leaf(&self, id: ZoneId, cx: &mut Context<Self>) -> AnyElement {
        let Some(zone) = self.zones.get(&id) else {
            return div().size_full().into_any_element();
        };
        let view = zone.content.as_terminal_view().clone();
        if self.zones.len() <= 1 {
            return view.into_any_element();
        }

        let active = id == self.active;
        let t = &self.theme;
        let chrome = t.chrome_background().hsla();
        let border = t.chrome_border().hsla();
        let fg = t.foreground.hsla();
        let muted = t.muted_foreground().hsla();
        let accent = t.accent().hsla();
        let hover_bg = t.hover_background().mix(t.foreground, 0.1).hsla();

        let status_color = zone.status.map(|state| match state {
            State::Blocked => t.ansi[3].hsla(),
            State::Error => t.ansi[1].hsla(),
            State::Done => t.ansi[2].hsla(),
            State::Working | State::Idle => muted,
        });
        let title = zone.title.clone();

        let header_btn = |btn_id: (&'static str, usize), label: &'static str| {
            div()
                .id(btn_id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.0))
                .rounded_sm()
                .text_xs()
                .text_color(muted)
                .cursor(CursorStyle::Arrow)
                .hover(move |d| d.bg(hover_bg).text_color(fg))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };

        let header = div()
            .id(("zone-header", id.0 as usize))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(px(ZONE_HEADER_HEIGHT))
            .pl_2p5()
            .pr_1()
            .gap_1p5()
            .bg(chrome)
            .border_b_1()
            .border_color(border)
            .text_xs()
            .cursor(CursorStyle::Arrow)
            .when(active, |d| d.text_color(fg))
            .when(!active, |d| d.text_color(muted))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                this.focus_zone(id, window, cx);
                cx.stop_propagation();
            }))
            .when(active, |d| {
                d.child(div().absolute().top_0().left_0().right_0().h(px(2.0)).bg(accent))
            })
            .when_some(status_color, |d, color| {
                d.child(div().flex_none().size(px(6.0)).rounded_full().bg(color))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_0p5()
                    .child(
                        header_btn(("zone-split-right", id.0 as usize), "◫").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.split_zone(id, SplitAxis::Horizontal, window, cx);
                            },
                        )),
                    )
                    .child(
                        header_btn(("zone-split-down", id.0 as usize), "⬒").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.split_zone(id, SplitAxis::Vertical, window, cx);
                            },
                        )),
                    )
                    .child(
                        header_btn(("zone-close", id.0 as usize), "×").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.close_zone_by_id(id, window, cx);
                            },
                        )),
                    ),
            );

        div()
            .id(("zone", id.0 as usize))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(header)
            .child(div().flex_1().min_w_0().min_h_0().size_full().child(view))
            .into_any_element()
    }
}

impl Render for ZoneTreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.tree.clone();
        let content = self.render_tree(&tree, cx);
        div()
            .id("zone-tree")
            .key_context("ZoneTree")
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, window, cx| {
                let position = window.mouse_position();
                for zone in this.zones.values() {
                    zone.content.as_terminal_view().update(cx, |view, cx| {
                        view.modifiers_changed(position, e.modifiers, cx);
                    });
                }
            }))
            .on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::split_down))
            .on_action(cx.listener(Self::close_active_zone))
            .on_action(cx.listener(Self::focus_left))
            .on_action(cx.listener(Self::focus_right))
            .on_action(cx.listener(Self::focus_up))
            .on_action(cx.listener(Self::focus_down))
            .relative()
            .size_full()
            .bg(self.theme.background.hsla())
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

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

    #[test]
    fn minimum_split_size_and_describe_zones() {
        let cw = px(8.0);
        let lh = px(16.0);
        let pad = px(4.0);
        // Min terminal is 2*8 + 2*4 = 24px wide, 2*16 + 2*4 = 40px high.
        // With header (28px), each zone needs >= 24px wide and >= 68px high.
        // Horizontal split needs width >= 2 * 24 + 1 = 49px, height >= 68px.
        // Vertical split needs width >= 24px, height >= 2 * 68 + 1 = 137px.
        assert!(can_split_zone(size(px(49.0), px(68.0)), false, SplitAxis::Horizontal, pad, cw, lh));
        assert!(!can_split_zone(size(px(48.0), px(68.0)), false, SplitAxis::Horizontal, pad, cw, lh));
        assert!(!can_split_zone(size(px(100.0), px(67.0)), false, SplitAxis::Horizontal, pad, cw, lh));

        assert!(can_split_zone(size(px(24.0), px(137.0)), true, SplitAxis::Vertical, pad, cw, lh));
        assert!(!can_split_zone(size(px(24.0), px(136.0)), true, SplitAxis::Vertical, pad, cw, lh));

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
}
