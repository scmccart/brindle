//! Live reload: a theme change while a session is attached redraws tmux
//! panes and local tabs without restarting them. (The colors programs get
//! from OSC 10/11 after a reload are `colors-reload`.)

use super::{cells, server};
use crate::brindle::{Run, theme};
use crate::capture::Cells;
use crate::{Ctx, Outcome, Result, ensure};

pub fn theme_attached(ctx: &Ctx) -> Result<Outcome> {
    // Two tmux panes with a line of plain text each.
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    for pane in ["%0", "%1"] {
        server.type_line(pane, "clear; echo plain text")?;
    }
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "panes").theme_at(1.5, "e2e-alt").capture_at(4.0).dump_at(5.0).run()?;
    let image = out.image.as_ref().unwrap();
    let cells = cells(&server, &out, &server.active_pane()?)?;
    for pane in ["%0", "%1"] {
        let [left, ..] = server.geometry(pane)?;
        let background = image.dominant(cells.rect(left + 2, 8, 20, 10));
        ensure!(background == Some(theme::ALT_BACKGROUND), "{pane} background {:?}", background.map(|c| c.hex()));
        let text = cells.rect(left, 0, 10, 1);
        image.expect(&format!("{pane} text"), text, theme::ALT_FOREGROUND, 10)?;
        image.expect_none(&format!("{pane} text"), text, theme::FOREGROUND)?;
    }

    // A local tab beside an attached session, shown after the switch.
    let out = Run::new(ctx, &server, "local")
        .profile("Shell")
        .action("new_tab_with_profile_2")
        .send("")
        .action("activate_tab_1")
        .theme_at(2.5, "e2e-alt")
        .capture_at(5.0)
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[0].1, "tabs {:?}", out.dump.tabs);
    let image = out.image.as_ref().unwrap();
    let cells = Cells::from_pane(out.dump.grid()?, 0, 0);
    let background = image.dominant(cells.rect(2, 8, 20, 10));
    ensure!(background == Some(theme::ALT_BACKGROUND), "local tab background {:?}", background.map(|c| c.hex()));
    Ok(Outcome::Pass)
}
