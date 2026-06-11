//! ratatui rendering for the deck list and review screens.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use crate::app::{App, GRADE_LABELS, Screen};

pub fn render(frame: &mut Frame, app: &mut App) {
    match app.screen {
        Screen::DeckList => render_deck_list(frame, app),
        Screen::Review => render_review(frame, app),
    }
}

fn render_deck_list(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let items: Vec<ListItem> = app
        .decks
        .iter()
        .map(|d| ListItem::new(d.as_str()))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Decks ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    if !app.decks.is_empty() {
        state.select(Some(app.deck_selected));
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let hint = footer(" j/k: move   l/Enter: review   q: quit ", app.status.as_deref());
    frame.render_widget(hint, chunks[1]);
}

fn render_review(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    // Title bar.
    let title = Line::from(vec![
        Span::styled(
            format!(" {} ", app.deck_name),
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        ),
        Span::raw(if app.answer_shown { " [answer] " } else { " [question] " }),
    ]);
    frame.render_widget(Paragraph::new(title), chunks[0]);

    if app.deck_finished {
        let done = Paragraph::new("\n🎉 No more cards due in this deck.\n\nPress 'd' to go back to the deck list.")
            .alignment(ratatui::layout::Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(done, chunks[1]);
        let hint = footer(" d: decks   q: quit ", app.status.as_deref());
        frame.render_widget(hint, chunks[2]);
        return;
    }

    let Some(card) = app.card.as_mut() else {
        let msg = Paragraph::new("Loading…").block(Block::default().borders(Borders::ALL));
        frame.render_widget(msg, chunks[1]);
        let hint = footer(" d: decks   q: quit ", app.status.as_deref());
        frame.render_widget(hint, chunks[2]);
        return;
    };

    // Choose which side to display.
    let side = if app.answer_shown {
        &mut card.answer
    } else {
        &mut card.question
    };

    // Split body into a text area and (if there are images) an image area.
    let body_chunks = if side.images.is_empty() {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)])
            .split(chunks[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1])
    };

    let text_block = Block::default()
        .borders(Borders::ALL)
        .title(if app.answer_shown { " Answer " } else { " Question " });
    let paragraph = Paragraph::new(side.text.as_str())
        .block(text_block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    frame.render_widget(paragraph, body_chunks[0]);

    // Render images side by side in the image area.
    if body_chunks.len() > 1 && !side.images.is_empty() {
        render_images(frame, body_chunks[1], &mut side.images);
    }

    // Footer: grading hints once the answer is shown.
    let hint_text = if app.answer_shown {
        grade_hint(&card.buttons, &card.next_reviews)
    } else {
        " space: show answer   j/k: scroll   r: replay audio   d: decks   q: quit ".to_string()
    };
    let hint = footer(&hint_text, app.status.as_deref());
    frame.render_widget(hint, chunks[2]);
}

/// Lay images out in equal-width columns and render each one.
fn render_images(
    frame: &mut Frame,
    area: Rect,
    images: &mut [ratatui_image::protocol::StatefulProtocol],
) {
    let n = images.len() as u32;
    let constraints: Vec<Constraint> = (0..images.len())
        .map(|_| Constraint::Ratio(1, n))
        .collect();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    for (img, col) in images.iter_mut().zip(cols.iter()) {
        let widget = StatefulImage::new();
        frame.render_stateful_widget(widget, *col, img);
    }
}

/// Build the grading footer from the card's available buttons/intervals.
fn grade_hint(buttons: &[i64], next_reviews: &[String]) -> String {
    let mut parts = Vec::new();
    for (i, label) in GRADE_LABELS.iter().enumerate() {
        let ease = (i + 1) as i64;
        if let Some(pos) = buttons.iter().position(|b| *b == ease) {
            let interval = next_reviews.get(pos).map(String::as_str).unwrap_or("");
            if interval.is_empty() {
                parts.push(format!("{ease}:{label}"));
            } else {
                parts.push(format!("{ease}:{label}({interval})"));
            }
        }
    }
    format!(" {}   r: replay   d: decks   q: quit ", parts.join("  "))
}

/// A one-line footer; shows a status/error message when present, else the hint.
fn footer<'a>(hint: &'a str, status: Option<&'a str>) -> Paragraph<'a> {
    match status {
        Some(msg) => Paragraph::new(Line::from(Span::styled(
            format!(" {msg} "),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ))),
        None => Paragraph::new(Line::from(hint.dim())),
    }
}
