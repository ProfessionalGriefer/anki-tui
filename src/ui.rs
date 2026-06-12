//! ratatui rendering for the deck list and review screens.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use crate::anki::DeckCounts;
use crate::app::{App, CardStats, GRADE_LABELS, Screen};
use crate::media::{Block as ContentBlock, SideMedia, render_html};

/// Width reserved by the list's highlight gutter (selection shown via bg color,
/// so the fold arrows aren't shifted).
const HIGHLIGHT_WIDTH: u16 = 0;
/// Width of each right-aligned count column.
const COUNT_WIDTH: usize = 5;

pub fn render(frame: &mut Frame, app: &mut App) {
    match app.screen {
        Screen::DeckList => render_deck_list(frame, app),
        Screen::Review => {
            render_review(frame, app);
            // Overlay the card-info popup once `render_review`'s mutable borrow
            // of `app` has ended, so we can read `app.stats` here.
            if let Some(stats) = &app.stats {
                let area = frame.area();
                render_stats_popup(frame, area, stats, app.stats_scroll);
                // Replace the footer hint with the popup's own controls.
                let footer_rect = Rect {
                    x: area.x,
                    y: area.y + area.height.saturating_sub(1),
                    width: area.width,
                    height: 1,
                };
                let hint = footer(" i/Esc: close   j/k: scroll   q: quit ", app.status.as_deref());
                frame.render_widget(Clear, footer_rect);
                frame.render_widget(hint, footer_rect);
            }
        }
    }
}

fn render_deck_list(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let filtered = app.visible_decks();
    // Width available for a row: inner area minus the borders and highlight gutter.
    let row_width = chunks[0].width.saturating_sub(2 + HIGHLIGHT_WIDTH);
    let searching = !app.search.is_empty();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|d| {
            if app.flat_view {
                // Flat view: full deck names, no indentation or fold markers.
                ListItem::new(deck_row(&d.name, 0, "", &d.counts, row_width))
            } else {
                // Fold marker: ▶ collapsed, ▼ expanded, blank for leaves. While
                // searching the tree is flattened, so don't show collapse arrows.
                let marker = if d.has_children {
                    if !searching && app.collapsed.contains(&d.name) {
                        "▶ "
                    } else {
                        "▼ "
                    }
                } else {
                    "  "
                };
                ListItem::new(deck_row(&d.label, d.depth, marker, &d.counts, row_width))
            }
        })
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
        );

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
            " j/k: move   h/l: fold   ,: flat/tree   Enter: review   /: search   y: sync   q: quit ",
            app.status.as_deref(),
        );
        frame.render_widget(hint, chunks[1]);
    }
}

/// Build a deck-list row: indentation + fold marker + label on the left, then
/// right-aligned new/learn/review counts colored like Anki (new = blue,
/// learn = red, review = green).
fn deck_row(label: &str, depth: usize, marker: &str, counts: &DeckCounts, width: u16) -> Line<'static> {
    let counts_width = COUNT_WIDTH * 3;
    let name_width = (width as usize).saturating_sub(counts_width).max(1);

    let left = format!("{}{}{}", "  ".repeat(depth), marker, label);
    // Truncate with an ellipsis if it doesn't fit, else pad to the column width.
    let name_field = if left.chars().count() > name_width {
        let kept: String = left.chars().take(name_width.saturating_sub(1)).collect();
        format!("{kept}…")
    } else {
        format!("{left:<name_width$}")
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

    let title = if app.answer_shown {
        " Answer "
    } else {
        " Question "
    };
    render_card_body(frame, chunks[1], side, app.scroll, title);

    // Footer: grading hints once the answer is shown.
    let hint_text = if app.answer_shown {
        grade_hint(&card.buttons, &card.next_reviews)
    } else {
        " space: show answer   j/k: scroll   r: replay   i: info   u: undo   d: decks   q: quit "
            .to_string()
    };
    let hint = footer(&hint_text, app.status.as_deref());
    frame.render_widget(hint, chunks[2]);
}

/// Render a card side: text and image blocks stacked vertically in document
/// order inside one bordered area, scrolled by `scroll` rows. Each text block
/// takes exactly the rows it needs and each image is drawn at its natural cell
/// size, so images appear inline with the surrounding text.
fn render_card_body(frame: &mut Frame, area: Rect, side: &mut SideMedia, scroll: u16, title: &str) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let view_h = inner.height as i32;
    // `y` is each block's top row relative to the top of `inner`, shifted up by
    // the scroll offset. A 1-row gap separates consecutive blocks.
    let mut y: i32 = -(scroll as i32);
    // Take the blocks out so we can borrow `side.images` mutably while iterating.
    let blocks = std::mem::take(&mut side.blocks);
    for (idx, blk) in blocks.iter().enumerate() {
        if idx > 0 {
            y += 1;
        }
        match blk {
            ContentBlock::Text(html) => {
                let text = render_html(html, inner.width);
                let h = text.lines.len() as i32;
                // Visible portion of this block within the viewport.
                if y + h > 0 && y < view_h {
                    let skip = (-y).max(0); // rows of this block scrolled off the top
                    let screen_y = inner.y + y.max(0) as u16;
                    let draw_h = (h - skip).min(view_h - y.max(0)).max(0) as u16;
                    if draw_h > 0 {
                        let rect = Rect {
                            x: inner.x,
                            y: screen_y,
                            width: inner.width,
                            height: draw_h,
                        };
                        let paragraph = Paragraph::new(text)
                            .wrap(Wrap { trim: false })
                            .scroll((skip as u16, 0));
                        frame.render_widget(paragraph, rect);
                    }
                }
                y += h;
            }
            ContentBlock::Image(i) => {
                let img = &mut side.images[*i];
                let h = img.rows as i32;
                // Only draw when the top is on-screen; ratatui-image can't clip a
                // partially scrolled image cleanly, so we keep it all-or-nothing.
                if y >= 0 && y < view_h {
                    let avail_h = (view_h - y) as u16;
                    let rect = Rect {
                        x: inner.x,
                        y: inner.y + y as u16,
                        width: img.cols.min(inner.width),
                        height: img.rows.min(avail_h),
                    };
                    frame.render_stateful_widget(StatefulImage::new(), rect, &mut img.protocol);
                }
                y += h;
            }
        }
    }
    side.blocks = blocks;
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
    format!(
        " {}   space: good   r: replay   i: info   u: undo   d: decks   q: quit ",
        parts.join("  ")
    )
}

/// A centered rectangle covering `pct_x`/`pct_y` percent of `area`.
fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vert[1])[1]
}

/// Render the card-info popup (Anki's `i` panel): label/value rows followed by
/// the review-history table, vertically scrollable.
fn render_stats_popup(frame: &mut Frame, area: Rect, stats: &CardStats, scroll: u16) {
    let area = centered_rect(80, 80, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Card Info ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    // Width of the label column, sized to the longest label.
    let label_w = stats
        .rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = stats
        .rows
        .iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(
                    format!("{label:<label_w$}  "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(value.clone()),
            ])
        })
        .collect();

    if !stats.history.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("{:<18}{:<9}{:<8}{:<13}{}", "Date", "Type", "Rating", "Interval", "Time"),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        for row in &stats.history {
            // Map the raw revlog type to Anki's label and color.
            let (label, color) = match row.kind {
                0 => ("Learn", Color::Blue),
                2 => ("Relearn", Color::Red),
                3 => ("Filtered", Color::Cyan),
                _ => ("Review", Color::Green),
            };
            // Again (1) is the only rating Anki colors (red).
            let rating_style = if row.rating == 1 {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::raw(format!("{:<18}", row.date)),
                Span::styled(format!("{label:<9}"), Style::default().fg(color)),
                Span::styled(format!("{:<8}", row.rating), rating_style),
                Span::raw(format!("{:<13}", row.interval)),
                Span::raw(row.time.clone()),
            ]));
        }
    }

    let para = Paragraph::new(lines).scroll((scroll, 0));
    frame.render_widget(para, inner);
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
