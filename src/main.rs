mod anki;
mod app;
mod media;
mod ui;

use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui_image::picker::Picker;

use crate::app::{App, Screen};

fn main() -> Result<()> {
    // Query the terminal for its graphics protocol and font size *before* we
    // take over the screen, so inline images use Kitty/Sixel/iTerm2 if available.
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());

    let mut app = App::new(picker)?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        // Timeout keeps the UI responsive for image (re)encoding.
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press {
                    handle_key(app, key.code);
                }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) {
    // `q` quits from anywhere.
    if let KeyCode::Char('q') = code {
        app.should_quit = true;
        return;
    }

    match app.screen {
        Screen::DeckList => match code {
            KeyCode::Char('j') | KeyCode::Down => app.select_next_deck(),
            KeyCode::Char('k') | KeyCode::Up => app.select_prev_deck(),
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => app.enter_review(),
            _ => {}
        },
        Screen::Review => match code {
            KeyCode::Char('d') => app.back_to_decks(),
            KeyCode::Char(' ') => app.show_answer(),
            KeyCode::Char('r') => app.replay_audio(),
            KeyCode::Char('j') | KeyCode::Down => app.scroll_down(),
            KeyCode::Char('k') | KeyCode::Up => app.scroll_up(),
            KeyCode::Char('1') => app.grade(1),
            KeyCode::Char('2') => app.grade(2),
            KeyCode::Char('3') => app.grade(3),
            KeyCode::Char('4') => app.grade(4),
            _ => {}
        },
    }
}
