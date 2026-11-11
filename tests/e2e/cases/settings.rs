//! The settings dialog: it opens on Profiles, the first of its two
//! sections; a theme previews on the tabs behind it, cancel puts the old
//! theme back, and apply writes it to the config.
//!
//! The test config's custom themes list as `e2e` (active) then `e2e-alt`,
//! after the built-ins, so one `settings_down` selects `e2e-alt`. Its one
//! profile, `Shell`, is the default, so the profile list opens on it.

use super::{Shots, cells, step_time};
use crate::brindle::{self, Output, Run, theme};
use crate::capture::Rgb;
use crate::{Ctx, Outcome, Result, ensure};

/// Checks the terminal background in the tab's bottom-left cells, which the
/// dialog (centred near the top) doesn't cover.
fn expect_background(what: &str, out: &Output, color: Rgb) -> Result {
    let image = out.image.as_ref().ok_or("no capture")?;
    let cells = cells(out)?;
    let (_, rows) = out.dump.size.ok_or("no size")?;
    let r = cells.rect(0, rows - 2, 12, 2);
    let area = ((r.x1 - r.x0) * (r.y1 - r.y0)) as usize;
    image.expect(what, r, color, area * 3 / 4)
}

fn config_text(ctx: &Ctx) -> Result<String> {
    std::fs::read_to_string(ctx.dir.join("config.toml")).map_err(|e| e.to_string())
}

/// Opens the dialog (on Profiles) and goes to the Theme section, the
/// second and last. Two steps.
pub fn open_theme(run: Run) -> Run {
    run.action("open_settings").action("settings_section_next")
}

pub fn preview(ctx: &Ctx) -> Result<Outcome> {
    let out = open_theme(Run::new(ctx, "run")).action("settings_down").capture_at(4.3).dump_at(4.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("section=Theme mode=list selected=\"e2e-alt\" active=\"e2e\""),
        "dialog state: {settings:?}"
    );
    expect_background("previewed background", &out, theme::ALT_BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "previewing changed the config");
    Ok(Outcome::Pass)
}

pub fn cancel(ctx: &Ctx) -> Result<Outcome> {
    let out = open_theme(Run::new(ctx, "run"))
        .action("settings_down")
        .action("settings_cancel")
        .capture_at(5.3)
        .dump_at(5.8)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_focus_on_tab(&out)?;
    expect_background("background after cancel", &out, theme::BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "cancelling changed the config");

    // Opened over the command palette, which it replaces: closing still
    // puts focus back on the tab.
    let out = Run::new(ctx, "over-palette")
        .action("open_command_palette")
        .action("open_settings")
        .action("settings_cancel")
        .dump_at(4.0)
        .run()?;
    ensure!(out.dump.settings.is_none() && out.dump.palette.is_empty(), "an overlay is still open: {:?}", out.dump);
    expect_focus_on_tab(&out)?;
    Ok(Outcome::Pass)
}

/// Keyboard focus is back on the tab's terminal once the dialog has closed.
fn expect_focus_on_tab(out: &Output) -> Result {
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "focus after closing the dialog: {:?}", out.dump.focus);
    Ok(())
}

pub fn apply(ctx: &Ctx) -> Result<Outcome> {
    let out = open_theme(Run::new(ctx, "run"))
        .action("settings_down")
        .action("settings_confirm")
        .capture_at(5.5)
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_focus_on_tab(&out)?;
    expect_background("background after apply", &out, theme::ALT_BACKGROUND)?;
    // Only the theme line changed; everything else is byte for byte the same.
    let want = brindle::config("e2e-alt");
    ensure!(config_text(ctx)? == want, "config after apply:\n{}", config_text(ctx)?);
    Ok(Outcome::Pass)
}

/// Opens the profile editor on the `Shell` profile (selected when the
/// dialog opens), with focus on its name field. Two steps.
fn edit_shell(run: Run) -> Run {
    run.action("open_settings").action("settings_edit")
}

/// Opens the profile editor on `Shell` and moves focus to its theme
/// choice. Eight steps.
fn edit_shell_theme(run: Run) -> Run {
    tab_to_theme(edit_shell(run))
}

/// Tabs from the profile editor's name field to its theme choice: Name,
/// Remote, Command, Arguments, Directory, Environment, then Theme. Six steps.
fn tab_to_theme(run: Run) -> Run {
    (0..6).fold(run, |run, _| run.action("settings_next_field"))
}

/// After the theme choice has focus, picks through its menu: down opens it
/// on "follow global", end highlights the last theme, `e2e-alt`, and enter
/// picks it. Three steps.
fn choose_alt_theme(run: Run) -> Run {
    run.action("list_down").action("list_last").action("list_confirm")
}

pub fn profile_preview(ctx: &Ctx) -> Result<Outcome> {
    let out = choose_alt_theme(edit_shell_theme(Run::new(ctx, "preview"))).capture_at(11.7).dump_at(11.9).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("section=Profiles mode=editor name=\"Shell\" field=\"Theme\" theme=\"e2e-alt\""),
        "dialog state: {settings:?}"
    );
    expect_background("profile preview", &out, theme::ALT_BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "previewing changed the config");

    let out = choose_alt_theme(edit_shell_theme(Run::new(ctx, "cancel")))
        .action("settings_cancel")
        .action("settings_cancel")
        .capture_at(13.3)
        .dump_at(13.8)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_focus_on_tab(&out)?;
    expect_background("background after cancel", &out, theme::BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "cancelling changed the config");
    Ok(Outcome::Pass)
}

/// The test config with `theme = "e2e-alt"` added to the `Shell` profile,
/// after its last key, as the editor writes it.
fn with_shell_profile_theme(config: &str) -> Result<String> {
    let anchor = "args = [\"--norc\", \"--noprofile\"]\n";
    let end = config.find(anchor).ok_or("no Shell args in the test config")? + anchor.len();
    Ok(format!("{}theme = \"e2e-alt\"\n{}", &config[..end], &config[end..]))
}

pub fn profile_save(ctx: &Ctx) -> Result<Outcome> {
    let out = choose_alt_theme(edit_shell_theme(Run::new(ctx, "run")))
        .action("settings_confirm")
        .capture_at(12.3)
        .dump_at(12.8)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("section=Profiles mode=list selected=\"Shell\""), "dialog state: {settings:?}");
    expect_background("background after save", &out, theme::ALT_BACKGROUND)?;
    let want = with_shell_profile_theme(&brindle::config("e2e"))?;
    ensure!(config_text(ctx)? == want, "config after save:\n{}", config_text(ctx)?);
    Ok(Outcome::Pass)
}

pub fn profile_rename(ctx: &Ctx) -> Result<Outcome> {
    // Rename Shell to work and save, then edit `work`'s theme: the open tab
    // only shows it if it followed the rename.
    let run = edit_shell(Run::new(ctx, "run"))
        .action("text_input_select_home")
        .send("work")
        .action("settings_confirm")
        .action("settings_edit");
    let out = choose_alt_theme(tab_to_theme(run)).action("settings_confirm").capture_at(17.3).dump_at(17.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("section=Profiles mode=list selected=\"work\""), "dialog state: {settings:?}");
    let config = config_text(ctx)?;
    ensure!(
        config.contains("default_profile = \"work\"") && config.contains("name = \"work\"\ncommand = \"bash\""),
        "config after rename:\n{config}"
    );
    expect_background("renamed profile's theme", &out, theme::ALT_BACKGROUND)?;
    Ok(Outcome::Pass)
}

/// The dialog opens on Profiles with the profile list focused, and has
/// two sections: ctrl-tab shows Theme, and ctrl-tab again wraps back to
/// Profiles. Nothing in the dialog's state mentions tmux.
pub fn sections(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "open").action("open_settings").dump_at(step_time(1) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.starts_with("section=Profiles mode=list selected=\"Shell\" default=\"Shell\" profiles=[\"Shell\"]"),
        "dialog state on open: {settings:?}"
    );
    ensure!(!settings.contains("tmux"), "the dialog's state mentions tmux: {settings:?}");
    ensure!(out.dump.focus_stop.as_deref() == Some("list"), "focus on open: {:?}", out.dump.focus_stop);

    let out = Run::new(ctx, "next").action("open_settings").key("ctrl-tab").dump_at(step_time(2) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.starts_with("section=Theme mode=list selected=\"e2e\""), "after ctrl-tab: {settings:?}");

    let out = Run::new(ctx, "wrap")
        .action("open_settings")
        .key("ctrl-tab")
        .key("ctrl-tab")
        .dump_at(step_time(3) + 0.8)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.starts_with("section=Profiles mode=list"), "ctrl-tab twice: {settings:?}");
    Ok(Outcome::Pass)
}

/// Shift-tab from the first control wraps to the last, the dialog's close
/// button, and tab from there wraps back to the first; focus never reaches
/// the tab behind the dialog.
pub fn tab_wrap(ctx: &Ctx) -> Result<Outcome> {
    // Shift-tab from the name reaches the section list, the first stop,
    // then wraps to the close button, the last.
    let out = edit_shell(Run::new(ctx, "first")).action("settings_prev_field").dump_at(step_time(3) + 0.8).run()?;
    ensure!(out.dump.focus_stop.as_deref() == Some("sections"), "after shift-tab from the name: {:?}", out.dump.focus_stop);
    let out = edit_shell(Run::new(ctx, "back"))
        .action("settings_prev_field")
        .action("settings_prev_field")
        .dump_at(step_time(4) + 0.8)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("mode=editor name=\"Shell\" field=\"close\""), "after shift-tab twice: {settings:?}");
    ensure!(out.dump.focus.as_deref() == Some("settings"), "focus after shift-tab: {:?}", out.dump.focus);

    // Tab from the close button wraps to the section list, then the name;
    // down then does nothing in the name field: tab is what moves.
    let out = edit_shell(Run::new(ctx, "forward"))
        .action("settings_prev_field")
        .action("settings_prev_field")
        .action("settings_next_field")
        .action("settings_next_field")
        .action("settings_down")
        .dump_at(step_time(7) + 0.8)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("mode=editor name=\"Shell\" field=\"Name\""),
        "after tab from the last control, then down: {settings:?}"
    );
    ensure!(out.dump.focus.as_deref() == Some("settings"), "focus after tab: {:?}", out.dump.focus);
    Ok(Outcome::Pass)
}

/// With the theme menu open in the profile editor, escape closes only the
/// menu: the editor stays open with its edit and the theme is unchanged. A
/// second escape then discards the editor, which it would not do if the
/// first had left the menu open.
pub fn escape_menu(ctx: &Ctx) -> Result<Outcome> {
    // Rename to `work` (unsaved), go to Theme, open its menu on "follow
    // global" and highlight the next theme, then escape.
    let open_menu = |name: &'static str| {
        let run = edit_shell(Run::new(ctx, name)).action("text_input_select_home").send("work");
        tab_to_theme(run).action("list_down").action("list_down")
    };
    let out = open_menu("once").action("settings_cancel").dump_at(14.0).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("mode=editor name=\"work\" field=\"Theme\" theme=\"follow global\""),
        "after one escape: {settings:?}"
    );

    let out = open_menu("twice").action("settings_cancel").action("settings_cancel").dump_at(15.0).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("mode=list selected=\"Shell\""), "after two escapes: {settings:?}");
    ensure!(config_text(ctx)? == brindle::config("e2e"), "escaping changed the config");
    Ok(Outcome::Pass)
}

// ---- focus stops and rings, with real key dispatch (`--key`) ----
// (`Shots` and its ring check are in `cases/mod.rs`, shared with the zone
// cards' ring.)

/// A list's ring (the profile list's and the theme list's): its width and
/// height ranges in logical pixels.
const LIST_RING: ((f32, f32), (f32, f32)) = ((300.0, 760.0), (100.0, 460.0));

/// The profile list shows its ring when the dialog opens; a pointer press
/// hides it and a key (tab to the first button, then shift-tab back) shows
/// it again. After ctrl-pagedown to Theme, the theme list has focus with
/// its ring, and after ctrl-pageup back to Profiles the profile list shows
/// it again.
pub fn focus_rings(ctx: &Ctx) -> Result<Outcome> {
    let shots = Shots::default();
    let mut run = Run::new(ctx, "run").action("open_settings");
    run = shots.after(run, ctx, 1, "profiles-open");
    run = shots.after(run.modality("pointer"), ctx, 2, "profiles-pointer");
    run = shots.after(run.key("tab").key("shift-tab"), ctx, 4, "profiles-tabbed");
    run = shots.after(run.key("ctrl-pagedown"), ctx, 5, "theme");
    run = shots.after(run.modality("pointer"), ctx, 6, "theme-pointer");
    run = shots.after(run.key("ctrl-pageup"), ctx, 7, "profiles-back");
    let out = run.dump_at(step_time(7) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.starts_with("section=Profiles mode=list"), "dialog state: {settings:?}");
    ensure!(
        out.dump.focus_stop.as_deref() == Some("list") && out.dump.modality.as_deref() == Some("keyboard"),
        "focus: {:?} {:?} {:?}",
        out.dump.focus,
        out.dump.focus_stop,
        out.dump.modality
    );
    let scale = out.dump.grid()?.scale;
    let (w, h) = LIST_RING;
    shots.expect_ring(ctx, "the profile list on open", ("profiles-open", "profiles-pointer"), scale, w, h)?;
    shots.expect_ring(ctx, "the profile list after a key", ("profiles-pointer", "profiles-tabbed"), scale, w, h)?;
    shots.expect_ring(ctx, "the theme list", ("theme", "theme-pointer"), scale, w, h)?;
    shots.expect_ring(ctx, "the profile list after Theme", ("profiles-pointer", "profiles-back"), scale, w, h)?;
    Ok(Outcome::Pass)
}

/// Tab from the profile editor's name reaches its Remote select, which
/// shows its ring, and hides it after a pointer press.
pub fn editor_select_ring(ctx: &Ctx) -> Result<Outcome> {
    let shots = Shots::default();
    let run = edit_shell(Run::new(ctx, "run")).key("tab");
    let run = shots.after(run, ctx, 3, "remote-key");
    let run = shots.after(run.modality("pointer"), ctx, 4, "remote-pointer");
    let out = run.dump_at(step_time(4) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("mode=editor name=\"Shell\" field=\"Remote\""), "dialog state: {settings:?}");
    ensure!(out.dump.focus_stop.as_deref() == Some("Remote"), "focus stop: {:?}", out.dump.focus_stop);
    let scale = out.dump.grid()?.scale;
    shots.expect_ring(ctx, "the Remote select", ("remote-key", "remote-pointer"), scale, (150.0, 700.0), (36.0, 48.0))?;
    Ok(Outcome::Pass)
}

/// The theme list is one Tab stop with a ring: end, home and down move its
/// selection, and tab goes on to the first footer button. The profile
/// list takes the same keys.
pub fn list_stop(ctx: &Ctx) -> Result<Outcome> {
    let shots = Shots::default();
    let run = open_theme(Run::new(ctx, "themes")).key("end").key("home").key("down");
    let run = shots.after(run, ctx, 5, "list-key");
    let run = shots.after(run.modality("pointer"), ctx, 6, "list-pointer");
    let out = run.key("tab").dump_at(step_time(7) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    // The list starts with the built-ins: brindle-dark, then brindle-light.
    ensure!(settings.contains("mode=list selected=\"brindle-light\""), "theme list after end, home, down: {settings:?}");
    ensure!(out.dump.focus_stop.as_deref() == Some("New"), "focus after tab from the list: {:?}", out.dump.focus_stop);
    let scale = out.dump.grid()?.scale;
    let (w, h) = LIST_RING;
    shots.expect_ring(ctx, "the theme list", ("list-key", "list-pointer"), scale, w, h)?;

    // With a second profile, end on the profile list (focused on open)
    // selects it.
    let out = Run::new(ctx, "profiles")
        .config("\n[[profiles]]\nname = \"Other\"\ncommand = \"sh\"\n")
        .action("open_settings")
        .key("end")
        .dump_at(step_time(2) + 0.8)
        .run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.starts_with("section=Profiles mode=list selected=\"Other\""),
        "profile list after end: {settings:?}"
    );
    ensure!(out.dump.focus_stop.as_deref() == Some("list"), "focus stop: {:?}", out.dump.focus_stop);
    Ok(Outcome::Pass)
}

/// The section a dump shows, from the settings line's `section=`.
fn shown_section(out: &Output) -> String {
    let settings = out.dump.settings.as_deref().unwrap_or("");
    settings.strip_prefix("section=").and_then(|s| s.split(' ').next()).unwrap_or("").to_string()
}

/// One section stop, wrapped as a check: the section list has focus.
fn expect_on_sections(out: &Output, what: &str) -> Result {
    ensure!(out.dump.focus_stop.as_deref() == Some("sections"), "{what}: focus on {:?}", out.dump.focus_stop);
    Ok(())
}

/// The section list is the dialog's first Tab stop, a vertical tab list,
/// all with real key dispatch: shift-tab from the profile list reaches it
/// and it rings; down shows Theme with focus staying on it; tab goes into
/// the section; end, home and up (wrapping) work; an editor with unsaved
/// edits holds the section; leaving Theme drops its preview.
pub fn section_list(ctx: &Ctx) -> Result<Outcome> {
    let open = |label: &'static str| Run::new(ctx, label).action("open_settings").key("shift-tab");

    let shots = Shots::default();
    let run = shots.after(open("ring"), ctx, 2, "sections-key");
    let run = shots.after(run.modality("pointer"), ctx, 3, "sections-pointer");
    let out = run.dump_at(step_time(3) + 0.8).run()?;
    expect_on_sections(&out, "shift-tab from the profile list")?;
    ensure!(shown_section(&out) == "Profiles", "section {:?}", out.dump.settings);
    let scale = out.dump.grid()?.scale;
    shots.expect_ring(ctx, "the section list", ("sections-key", "sections-pointer"), scale, (120.0, 140.0), (28.0, 46.0))?;

    let out = open("theme").key("down").dump_at(step_time(3) + 0.8).run()?;
    expect_on_sections(&out, "down from Profiles")?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.starts_with("section=Theme mode=list"), "after down: {settings:?}");

    let out = open("theme-tab").key("down").key("tab").dump_at(step_time(4) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.starts_with("section=Theme mode=list selected=\"e2e\""), "after down, tab: {settings:?}");
    ensure!(out.dump.focus_stop.as_deref() == Some("list"), "tab from the section list: {:?}", out.dump.focus_stop);

    let out = open("ends").key("end").key("home").key("up").dump_at(step_time(5) + 0.8).run()?;
    expect_on_sections(&out, "end, home, up")?;
    ensure!(shown_section(&out) == "Theme", "up from Profiles wraps to Theme: {:?}", out.dump.settings);

    // Unsaved edits in the profile editor hold the section.
    let out = edit_shell(Run::new(ctx, "held"))
        .action("text_input_select_home")
        .send("work")
        .key("shift-tab")
        .key("down")
        .dump_at(step_time(6) + 0.8)
        .run()?;
    expect_on_sections(&out, "down with unsaved edits")?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.starts_with("section=Profiles mode=editor name=\"work\"") && settings.contains("Save or discard"),
        "the editor didn't hold the section: {settings:?}"
    );

    // Theme's preview (down to e2e-alt) goes when the section changes.
    let out = open_theme(Run::new(ctx, "preview"))
        .key("down")
        .key("shift-tab")
        .key("up")
        .capture_at(step_time(5) + 0.6)
        .dump_at(step_time(5) + 0.9)
        .run()?;
    expect_on_sections(&out, "up from Theme")?;
    ensure!(shown_section(&out) == "Profiles", "after up from Theme: {:?}", out.dump.settings);
    expect_background("background after leaving Theme", &out, theme::BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "leaving Theme changed the config");
    Ok(Outcome::Pass)
}

/// The dialog-level section shortcuts work from every kind of stop: a
/// list, a select, a text field, a button. Each run presses one with
/// `--key` and checks the section it lands on.
pub fn section_shortcuts(ctx: &Ctx) -> Result<Outcome> {
    let section = shown_section;
    let cases: [(&'static str, &[&str], &str, &str); 6] = [
        // From the profile list, focused on open.
        ("profile-list", &["open_settings"], "ctrl-tab", "Theme"),
        // From the (clean) profile editor's Remote select.
        ("select", &["open_settings", "settings_edit", "settings_next_field"], "ctrl-tab", "Theme"),
        // From the (clean) profile editor's name field.
        ("text-field", &["open_settings", "settings_edit"], "ctrl-pagedown", "Theme"),
        // From the theme list, backwards.
        ("theme-list", &["open_settings", "settings_section_next"], "ctrl-shift-tab", "Profiles"),
        // From the theme list, wrapping forwards.
        ("wrap", &["open_settings", "settings_section_next"], "ctrl-pagedown", "Profiles"),
        // From the profile list's first button, wrapping backwards.
        ("button", &["open_settings", "settings_next_field"], "ctrl-pageup", "Theme"),
    ];
    for (label, actions, key, want) in cases {
        let run = actions.iter().fold(Run::new(ctx, label), |run, a| run.action(a));
        let out = run.key(key).dump_at(step_time(actions.len() + 1) + 0.8).run()?;
        ensure!(section(&out) == want, "{key} from the {label}: {:?}", out.dump.settings);
    }
    Ok(Outcome::Pass)
}

/// Enter on a focused button presses that button alone, with real key
/// dispatch: Discard in the profile editor discards an edit without
/// saving it, and Cancel in the theme list closes without applying.
pub fn enter_on_button(ctx: &Ctx) -> Result<Outcome> {
    // Rename to `work`, then shift-tab from the name: the section list,
    // the close X, Save, then Discard.
    let to_discard = |name: &'static str| {
        edit_shell(Run::new(ctx, name))
            .action("text_input_select_home")
            .send("work")
            .key("shift-tab")
            .key("shift-tab")
            .key("shift-tab")
            .key("shift-tab")
    };
    let out = to_discard("on-discard").dump_at(step_time(8) + 0.8).run()?;
    ensure!(out.dump.focus_stop.as_deref() == Some("Discard"), "focus before enter: {:?}", out.dump.focus_stop);
    let out = to_discard("discard").key("enter").dump_at(step_time(9) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(settings.contains("mode=list selected=\"Shell\""), "after enter on Discard: {settings:?}");
    ensure!(config_text(ctx)? == brindle::config("e2e"), "enter on Discard changed the config");

    // Down previews e2e-alt; shift-tab from the list: the section list, the
    // close X, Apply, then Cancel.
    let out = open_theme(Run::new(ctx, "cancel"))
        .key("down")
        .key("shift-tab")
        .key("shift-tab")
        .key("shift-tab")
        .key("shift-tab")
        .key("enter")
        .capture_at(step_time(8) + 0.6)
        .dump_at(step_time(8) + 0.9)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open: {:?}", out.dump.settings);
    expect_focus_on_tab(&out)?;
    expect_background("background after enter on Cancel", &out, theme::BACKGROUND)?;
    ensure!(config_text(ctx)? == brindle::config("e2e"), "enter on Cancel changed the config");
    Ok(Outcome::Pass)
}
