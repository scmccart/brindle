//! The cases. Each is one function; `all` lists them in run order.
//!
//! Windows runs only its own cases (`win`), which include the `local-server-*`
//! ones: the others need tmux and X11.
#![cfg_attr(windows, allow(dead_code))]

#[cfg(windows)]
mod win;
mod colors;
mod commands;
mod dividers;
mod dump;
mod geometry;
mod input;
mod instance;
mod links;
mod local_server;
mod multi_window;
mod restore;
mod notify;
mod palette;
mod profile_menu;
mod reattach;
mod remote;
mod reload;
mod selection;
mod search;
mod session_end;
mod settings;
mod shell;
mod status;
mod styles;
mod tab_menu;
mod tabs;
mod titles;
mod tmux;
mod windows;
mod zone_menu;
mod zones;

use crate::capture::Cells;
use crate::brindle::Output;
use crate::tmux::TmuxServer;
use crate::{Case, Result};

pub fn all() -> Vec<Case> {
    #[cfg(windows)]
    return win::all();
    #[cfg(not(windows))]
    unix()
}

fn unix() -> Vec<Case> {
    let display = |name, run| Case { name, display: true, run };
    let headless = |name, run| Case { name, display: false, run };
    vec![
        headless("tmux-subscription-format", tmux::subscription_format),
        headless("tmux-border-status-no-layout-change", tmux::border_status_no_layout_change),
        headless("tmux-rotate-no-layout-change", tmux::rotate_no_layout_change),
        headless("tmux-color-report-precedence", tmux::color_report_precedence),
        headless("tmux-osc4-unanswered", tmux::osc4_unanswered),
        headless("tmux-osc133-passthrough", shell::tmux_passthrough),
        headless("tmux-osc-in-output", notify::tmux_osc_in_output),
        display("tmux-subscriptions-removed", tmux::subscriptions_removed),
        display("tmux-poll-session-end-inside", tmux::poll_session_end_inside),
        display("attach-without-locale", tmux::attach_without_locale),
        display("dump-grid-geometry", dump::grid_geometry),
        display("remote-container-exec", remote::container_exec),
        display("remote-ssh-container", remote::ssh_container),
        display("remote-tmux-connection-lost", remote::tmux_connection_lost),
        display("geometry-plain", geometry::plain),
        display("geometry-top-before-attach", geometry::top_before_attach),
        display("geometry-top-while-attached", geometry::top_while_attached),
        display("geometry-fullscreen-less", geometry::fullscreen_less),
        display("geometry-bottom", geometry::bottom),
        display("geometry-zoom", geometry::zoom),
        display("geometry-background-window", geometry::background_window),
        display("geometry-rotate", geometry::rotate),
        display("geometry-rotate-unequal", geometry::rotate_unequal),
        display("colors-attach", colors::attach),
        display("colors-new-pane", colors::new_pane),
        display("colors-reload", colors::reload),
        display("colors-styled-pane", colors::styled_pane),
        display("colors-palette", colors::palette),
        display("colors-palette-reload", colors::palette_reload),
        display("colors-palette-tmux-answers", colors::palette_tmux_answers),
        display("colors-palette-tty-client", colors::palette_tty_client),
        display("titles-default-format", titles::default_format),
        display("titles-claude-code-format", titles::claude_code_format),
        display("titles-rename", titles::rename),
        display("titles-bottom-and-off", titles::bottom_and_off),
        display("titles-stacked", titles::stacked),
        display("styles-tint", styles::tint),
        display("styles-active-pane", styles::active_pane),
        display("styles-borders", styles::borders),
        display("styles-chrome-fallback", styles::chrome_fallback),
        display("settings-preview", settings::preview),
        display("settings-cancel", settings::cancel),
        display("settings-apply", settings::apply),
        display("settings-profile-preview", settings::profile_preview),
        display("settings-profile-save", settings::profile_save),
        display("settings-profile-rename", settings::profile_rename),
        display("settings-general", settings::general),
        display("settings-tab-wrap", settings::tab_wrap),
        display("settings-escape-menu", settings::escape_menu),
        display("settings-focus-rings", settings::focus_rings),
        display("settings-editor-select-ring", settings::editor_select_ring),
        display("settings-list-stop", settings::list_stop),
        display("settings-enter-on-button", settings::enter_on_button),
        display("settings-section-shortcuts", settings::section_shortcuts),
        display("settings-section-list", settings::section_list),
        display("session-end-close", session_end::close),
        display("session-end-detach-close", session_end::detach_close),
        display("session-end-new-tab", session_end::new_tab),
        display("session-end-ask", session_end::ask),
        display("session-end-close-tab", session_end::close_tab),
        display("profile-picker-open-state", windows::profile_picker_state),
        display("profile-shown-switches", windows::shown_switches),
        display("tmux-new-session", windows::new_session),
        display("tmux-new-session-duplicate", windows::new_session_duplicate),
        display("restore-merge-windows", windows::restore_merge),
        display("multi-process-one-client", multi_window::second_process_one_client),
        display("multi-process-independent", multi_window::second_process_independent),
        display("multi-process-local", multi_window::second_process_local),
        display("multi-process-dismiss", multi_window::second_process_dismiss),
        display("multi-process-new-session", multi_window::second_process_new_session),
        display("multi-process-shortcut", multi_window::second_process_shortcut),
        display("multi-process-other-client", multi_window::other_client),
        display("multi-process-session-ended", multi_window::second_process_session_ended),
        display("input-palette", input::palette),
        display("input-palette-run", input::palette_run),
        display("input-select-all", input::select_all),
        display("input-settings-tab", input::settings_tab),
        display("input-prompt-preselected", input::prompt_preselected),
        display("links-local-click", links::local_click),
        display("links-mouse-mode", links::mouse_mode),
        display("links-tmux", links::tmux),
        display("restore-local-tabs", restore::local_tabs),
        display("restore-fallbacks", restore::fallbacks),
        display("restore-tmux-session", restore::tmux_session),
        display("restore-forget-on-exit", restore::forget_on_exit),
        display("restore-close-window", restore::close_window),
        display("restore-second-instance", restore::second_instance),
        display("shell-prompt-jump", shell::prompt_jump),
        display("shell-no-marks-scroll-line", shell::no_marks_scroll_line),
        display("shell-failed-mark", shell::failed_mark),
        display("shell-select-output", shell::select_output),
        display("shell-tmux-prompt-jump", shell::tmux_prompt_jump),
        display("shell-clear-up-down", shell::clear_up_down),
        display("shell-tmux-clear-up-down", shell::tmux_clear_up_down),
        display("notify-osc-local", notify::osc_local),
        display("notify-tmux-background", notify::tmux_background),
        display("status-local", status::local),
        display("status-tmux-pane", status::tmux_pane),
        display("search-local", search::local),
        display("search-step", search::step),
        display("search-long-query", search::long_query),
        display("search-input-selection", search::input_selection),
        display("search-invalid-regex", search::invalid_regex),
        display("search-close-restores", search::close_restores),
        display("search-tmux-pane", search::tmux_pane),
        display("reattach-alt-screen", reattach::alt_screen),
        display("reattach-mouse-modes", reattach::mouse_modes),
        display("selection-current-window", selection::current_window),
        display("selection-active-pane", selection::active_pane),
        display("selection-split-active-pane", selection::split_active_pane),
        display("selection-background-window", selection::background_window),
        display("palette-tmux-action", palette::tmux_action),
        display("palette-lists-for-tab", palette::lists_for_tab),
        display("palette-escape", palette::escape),
        display("palette-tab-stays", palette::tab_stays),
        display("palette-wrap", palette::wrap),
        display("palette-home-end", palette::home_end),
        display("palette-prompt-focus", palette::prompt_focus),
        display("palette-fits-window", palette::fits_window),
        display("commands-break-pane", commands::break_pane),
        display("commands-layouts", commands::layouts),
        display("commands-rotate-swap", commands::rotate_swap),
        display("commands-detach-beside-local", commands::detach_beside_local),
        display("commands-rename-tab", commands::rename_tab),
        display("dividers-tiled", dividers::tiled),
        display("dividers-t-junction", dividers::t_junction),
        display("reload-theme-attached", reload::theme_attached),
        display("zones-split-and-focus", zones::split_and_focus),
        display("zones-close", zones::close),
        display("zones-restore", zones::restore),
        display("zones-header-colors", zones::header_colors),
        display("tabs-strip-colors", tabs::strip_colors),
        display("tabs-strip-keys", tabs::strip_keys),
        display("tabs-actions-keep-focus", tabs::actions_keep_focus),
        display("profile-menu-items", profile_menu::items),
        display("profile-menu-pick", profile_menu::pick),
        display("profile-menu-shown", profile_menu::shown),
        display("profile-menu-palette-exclusive", profile_menu::palette_exclusive),
        display("tab-menu-items", tab_menu::items),
        display("tab-menu-choose", tab_menu::choose),
        display("tab-menu-exclusive", tab_menu::exclusive),
        display("tab-menu-tmux", tab_menu::tmux),
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

/// A server with one pane running the plain test shell.
fn server() -> Result<TmuxServer> {
    TmuxServer::new(113, 33)
}

/// The window's cell grid, from the run's dump and tmux's geometry for the
/// pane that was active when it was taken.
fn cells(server: &TmuxServer, out: &Output, active: &str) -> Result<Cells> {
    let [left, top, _, _] = server.geometry(active)?;
    Ok(Cells::from_pane(out.dump.grid()?, left, top))
}

/// Skips a case when a program it needs isn't installed.
fn require(program: &str) -> Option<crate::Outcome> {
    let found = std::process::Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .is_ok_and(|o| o.status.success());
    (!found).then(|| crate::Outcome::Skip(format!("{program} is not installed")))
}
