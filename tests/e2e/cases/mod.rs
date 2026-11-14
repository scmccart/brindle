//! The cases. Each is one function; `all` lists them in run order.
//!
//! Windows runs only its own cases (`win`), which include the `local-server-*`
//! ones: the others need X11.
#![cfg_attr(windows, allow(dead_code))]

#[cfg(windows)]
mod win;
mod cards;
mod colors;
mod commands;
mod dump;
mod input;
mod instance;
mod links;
mod local_server;
mod multi_window;
mod restore;
mod notify;
mod palette;
mod profile_menu;
mod remote;
mod reload;
mod search;
mod settings;
mod shell;
mod status;
mod tab_menu;
mod tabs;
mod zone_menu;
mod zones;

use std::sync::{Arc, Mutex};

use crate::brindle::{Output, Run};
use crate::capture::{self, Cells, Changes, Image};
use crate::{Case, Ctx, Result, ensure};

pub fn all() -> Vec<Case> {
    #[cfg(windows)]
    return win::all();
    #[cfg(not(windows))]
    unix()
}

fn unix() -> Vec<Case> {
    let display = |name, run| Case { name, display: true, run };
    vec![
        display("dump-grid-geometry", dump::grid_geometry),
        display("dump-wait-for-timeout", dump::wait_for_timeout),
        display("dump-wait-for-pending", dump::wait_for_pending),
        display("dump-wait-for-clean", dump::wait_for_clean),
        display("remote-container-exec", remote::container_exec),
        display("remote-ssh-container", remote::ssh_container),
        display("colors-local", colors::local),
        display("colors-split-zone", colors::split_zone),
        display("colors-reload", colors::reload),
        display("colors-palette", colors::palette),
        display("colors-palette-reload", colors::palette_reload),
        display("settings-preview", settings::preview),
        display("settings-cancel", settings::cancel),
        display("settings-apply", settings::apply),
        display("settings-profile-preview", settings::profile_preview),
        display("settings-profile-save", settings::profile_save),
        display("settings-profile-rename", settings::profile_rename),
        display("settings-sections", settings::sections),
        display("settings-tab-wrap", settings::tab_wrap),
        display("settings-escape-menu", settings::escape_menu),
        display("settings-focus-rings", settings::focus_rings),
        display("settings-editor-select-ring", settings::editor_select_ring),
        display("settings-list-stop", settings::list_stop),
        display("settings-enter-on-button", settings::enter_on_button),
        display("settings-section-shortcuts", settings::section_shortcuts),
        display("settings-section-list", settings::section_list),
        display("multi-process-local", multi_window::second_process_local),
        display("input-palette", input::palette),
        display("input-palette-run", input::palette_run),
        display("input-select-all", input::select_all),
        display("input-settings-tab", input::settings_tab),
        display("input-prompt-preselected", input::prompt_preselected),
        display("links-local-click", links::local_click),
        display("links-mouse-mode", links::mouse_mode),
        display("restore-local-tabs", restore::local_tabs),
        display("restore-fallbacks", restore::fallbacks),
        display("restore-merge-windows", restore::merge_windows),
        display("restore-forget-on-exit", restore::forget_on_exit),
        display("restore-close-window", restore::close_window),
        display("restore-second-instance", restore::second_instance),
        display("tmux-legacy-config", restore::tmux_legacy_config),
        display("shell-prompt-jump", shell::prompt_jump),
        display("shell-no-marks-scroll-line", shell::no_marks_scroll_line),
        display("shell-failed-mark", shell::failed_mark),
        display("shell-select-output", shell::select_output),
        display("shell-clear-up-down", shell::clear_up_down),
        display("notify-osc-local", notify::osc_local),
        display("notify-background-tab", notify::background_tab),
        display("status-local", status::local),
        display("status-background-tab", status::background_tab),
        display("search-local", search::local),
        display("search-step", search::step),
        display("search-long-query", search::long_query),
        display("search-input-selection", search::input_selection),
        display("search-invalid-regex", search::invalid_regex),
        display("search-close-restores", search::close_restores),
        display("search-zones", search::zones),
        display("palette-zone-action", palette::zone_action),
        display("palette-lists-for-tab", palette::lists_for_tab),
        display("palette-escape", palette::escape),
        display("palette-tab-stays", palette::tab_stays),
        display("palette-wrap", palette::wrap),
        display("palette-home-end", palette::home_end),
        display("palette-prompt-focus", palette::prompt_focus),
        display("palette-fits-window", palette::fits_window),
        display("commands-rename-tab", commands::rename_tab),
        display("reload-theme-attached", reload::theme_attached),
        display("zones-split-and-focus", zones::split_and_focus),
        display("zones-close", zones::close),
        display("zones-restore", zones::restore),
        display("zones-header-colors", zones::header_colors),
        display("cards-zone-border", cards::zone_border),
        display("cards-zone-corners", cards::zone_corners),
        display("cards-zone-gap", cards::zone_gap),
        display("cards-zone-ring", cards::zone_ring),
        display("tabs-strip-colors", tabs::strip_colors),
        display("tabs-strip-keys", tabs::strip_keys),
        display("tabs-actions-keep-focus", tabs::actions_keep_focus),
        display("profile-menu-items", profile_menu::items),
        display("profile-menu-pick", profile_menu::pick),
        display("profile-menu-palette-exclusive", profile_menu::palette_exclusive),
        display("tab-menu-items", tab_menu::items),
        display("tab-menu-choose", tab_menu::choose),
        display("tab-menu-exclusive", tab_menu::exclusive),
        display("zone-menu-items", zone_menu::items),
        display("zone-menu-choose", zone_menu::choose),
        display("zone-menu-too-small", zone_menu::too_small),
        display("local-server-session", local_server::session),
        display("local-server-close", local_server::close),
        display("local-server-quit", local_server::quit),
        display("local-server-crash", local_server::crash),
        display("local-server-close-zone", local_server::close_zone),
        display("local-server-close-window", local_server::close_window),
        display("local-server-close-command", local_server::close_command),
        display("local-server-reattach-quit", local_server::reattach_quit),
        display("local-server-reattach-crash", local_server::reattach_crash),
        display("local-server-reattach-killed", local_server::reattach_killed),
        display("local-server-reattach-inactive", local_server::reattach_inactive),
        display("local-server-reattach-closed-tab", local_server::reattach_closed_tab),
        display("local-server-reattach-no-restore", local_server::reattach_no_restore),
        display("local-server-reattach-second", local_server::reattach_second),
        display("local-server-palette", local_server::palette),
        display("local-server-attach-flag", local_server::attach_flag),
        display("instance-second-launch", instance::second_launch),
        display("instance-attach", instance::attach),
        display("local-server-env", local_server::env),
        display("local-server-core-restart", local_server::core_restart),
        display("local-server-replay-quiet", local_server::replay_quiet),
        display("local-server-holder-lost", local_server::holder_lost),
        display("local-server-no-helper", local_server::no_helper),
        display("local-server-upgrade", local_server::upgrade),
        display("local-server-manage", local_server::manage),
    ]
}

/// The active terminal's cell grid, from the run's dump.
fn cells(out: &Output) -> Result<Cells> {
    Ok(Cells::from_pane(out.dump.grid()?, 0, 0))
}

/// The gap round a local tab's zone cards (`ZONE_GAP`), and a card's border (`brindle_ui::CARD_BORDER`), in
/// logical pixels: a card's content starts this far in from the tab's area.
const ZONE_GAP: f32 = 6.0;
const CARD_BORDER: f32 = 1.0;
const CARD_INSET: f32 = ZONE_GAP + CARD_BORDER;

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

/// Window captures taken during a run, by name.
#[derive(Clone, Default)]
struct Shots(Arc<Mutex<Vec<(&'static str, Image)>>>);

impl Shots {
    /// Captures the window as `name` a little after step `step` ran, before
    /// the next one.
    fn after<'a>(&self, run: Run<'a>, ctx: &Ctx, step: usize, name: &'static str) -> Run<'a> {
        let (display, shots, slow) = (ctx.display.clone(), self.0.clone(), ctx.secs(1.0));
        run.call_at(step_time(step) + 0.55, move |pid| {
            let image = capture::grab(&display, pid, slow)?;
            shots.lock().unwrap().push((name, image));
            Ok(())
        })
    }

    /// Runs `check` on captures `a` and `b`, and saves both to the case
    /// directory when it fails.
    fn compare<T>(&self, ctx: &Ctx, (a, b): (&str, &str), check: impl FnOnce(&Image, &Image) -> Result<T>) -> Result<T> {
        let shots = self.0.lock().unwrap();
        let find = |name: &str| shots.iter().find(|(n, _)| *n == name).map(|(_, i)| i).ok_or(format!("no capture {name:?}"));
        let (first, second) = (find(a)?, find(b)?);
        let result = check(first, second);
        if result.is_err() {
            first.save_ppm(&ctx.dir.join(format!("{a}.ppm")));
            second.save_ppm(&ctx.dir.join(format!("{b}.ppm")));
        }
        result.map_err(|e| format!("{a} against {b}: {e}"))
    }

    /// Captures `a` and `b` show the same view except for one focus ring:
    /// the pixels that differ form a band about 3 px wide around a box
    /// `width` by `height` logical pixels (ring included) within the given
    /// ranges. Both captures are saved to the case directory if not.
    fn expect_ring(
        &self,
        ctx: &Ctx,
        what: &str,
        pair: (&str, &str),
        scale: f32,
        width: (f32, f32),
        height: (f32, f32),
    ) -> Result<Changes> {
        self.compare(ctx, pair, |a, b| ring_between(a, b, scale, width, height)).map_err(|e| format!("{what} ({e})"))
    }
}

fn ring_between(a: &Image, b: &Image, scale: f32, width: (f32, f32), height: (f32, f32)) -> Result<Changes> {
    let changes = a.changes(b, a.all(), 16)?;
    let (count, bbox) = (changes.count, changes.bbox.ok_or("no pixel differs: no ring")?);
    let (w, h) = ((bbox.x1 - bbox.x0) as f32 / scale, (bbox.y1 - bbox.y0) as f32 / scale);
    ensure!(
        (width.0..=width.1).contains(&w) && (height.0..=height.1).contains(&h),
        "the change spans {w}x{h} at {bbox:?}, want {width:?} x {height:?}"
    );
    // A 3 px ring (and a 1 px border inside it) along the box's edges, not
    // a filled box.
    let band = 2.0 * (w + h) * scale * 3.0 * scale;
    ensure!(
        (count as f32) >= band * 0.5 && (count as f32) <= band * 2.0,
        "{count} pixels differ in {w}x{h}, want a ring of about {band}"
    );
    Ok(changes)
}
