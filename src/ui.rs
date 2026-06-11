//! ratatui rendering for the deck list and review screens.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use crate::anki::DeckCounts;
use crate::app::{App, GRADE_LABELS, Screen};

/// Width reserved by the list's highlight symbol (`"▶ "`).
const HIGHLIGHT_WIDTH: u16 = 2;
/// Width of each right-aligned count column.
const COUNT_WIDTH: usize = 5;

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

    let filtered = app.filtered_decks();
    // Width available for a row: inner area minus the borders and highlight gutter.
    let row_width = chunks[0].width.saturating_sub(2 + HIGHLIGHT_WIDTH);
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|d| ListItem::new(deck_row(&d.name, &d.counts, row_width)))
        .collect();

    // Title reflects the active filter and match count.
    let title = if app.search.is_empty() {
        format!(" Decks ({}) ", filtered.len())
    } else {
        format!(" Decks ({}) — /{} ", filtered.len(), app.search)
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
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
    if !filtered.is_empty() {
        state.select(Some(app.deck_selected));
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    // Footer: live search prompt while typing, else key hints.
    if app.searching {
        let search_line = Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(app.search.as_str()),
            Span::styled("█", Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled(
                "Enter: keep  Esc: clear",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(search_line), chunks[1]);
    } else {
        let hint = footer(
            " j/k: move   l/Enter: review   /: search   q: quit ",
            app.status.as_deref(),
        );
        frame.render_widget(hint, chunks[1]);
    }
}

/// Build a deck-list row: the name on the left, then right-aligned new/learn/
/// review counts colored like Anki (new = blue, learn = red, review = green).
fn deck_row(name: &str, counts: &DeckCounts, width: u16) -> Line<'static> {
    let counts_width = COUNT_WIDTH * 3;
    let name_width = (width as usize).saturating_sub(counts_width).max(1);

    // Truncate the name with an ellipsis if it doesn't fit, else pad it.
    let name_field = if name.chars().count() > name_width {
        let kept: String = name.chars().take(name_width.saturating_sub(1)).collect();
        format!("{kept}…")
    } else {
        format!("{name:<name_width$}")
    };

    Line::from(vec![
        Span::raw(name_field),
        count_span(counts.new, Color::Blue),
        count_span(counts.learn, Color::Red),
        count_span(counts.review, Color::Green),
    ])
}

/// A right-aligned count cell; zero is dimmed so non-zero counts stand out.
fn count_span(n: u32, color: Color) -> Span<'static> {
    let style = if n == 0 {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(color)
    };
    Span::styled(format!("{n:>w$}", w = COUNT_WIDTH), style)
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
        let done = Paragraph::new("\n🎉 No more cards due in this deck.\n\nPress 'u' to undo the last card, or 'd' to go back to the deck list.")
            .alignment(ratatui::layout::Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(done, chunks[1]);
        let hint = footer(" u: undo   d: decks   q: quit ", app.status.as_deref());
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
    // Render the HTML to text wrapped to the block's inner width.
    let text_width = body_chunks[0].width.saturating_sub(2);
    let text = side.to_text(text_width);
    let paragraph = Paragraph::new(text)
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
        " space: show answer   j/k: scroll   r: replay   u: undo   d: decks   q: quit ".to_string()
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
    format!(" {}   r: replay   u: undo   d: decks   q: quit ", parts.join("  "))
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
