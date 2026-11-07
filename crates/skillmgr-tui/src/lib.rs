//! Promptui dashboard for skillmgr.

pub mod app;
pub mod ui;

use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

use skillmgr_core::Error;
use skillmgr_core::ops::{self, Context};

use app::App;

/// Run the interactive dashboard until the user quits.
pub fn run(ctx: Context) -> skillmgr_core::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new(ctx);
    app.refresh();

    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> skillmgr_core::Result<()> {
    loop {
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(Error::Io)?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        app.tick();

        if let Some(path) = app.take_edit() {
            ratatui::restore();
            let result = ops::open_editor(&path);
            *terminal = ratatui::init();
            if let Err(e) = result {
                app.notify_error(e.to_string());
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
