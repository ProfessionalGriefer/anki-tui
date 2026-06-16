//! Review screen: card loading, revealing the answer, grading, and undo.

use anyhow::Result;
use chrono::{Local, TimeZone};

use super::{App, CardStats, GRADE_LABELS, ReviewCard, ReviewRow};
use crate::media::SideMedia;

impl App {
    /// Fetch the reviewer's current card and build its media.
    pub fn load_current_card(&mut self) {
        self.answer_shown = false;
        self.scroll = 0;
        self.undone = false;
        match self.anki.gui_current_card() {
            Ok(Some(c)) => {
                let question = SideMedia::build(&c.question, &self.anki, &self.picker);
                let answer = SideMedia::build(&c.answer, &self.anki, &self.picker);
                // `[sound:...]` tokens only survive in the raw fields, so collect
                // audio from there (ordered by the note's field order).
                let mut fields: Vec<_> = c.fields.into_values().collect();
                fields.sort_by_key(|f| f.order);
                let field_html: String = fields
                    .iter()
                    .map(|f| f.value.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let card = ReviewCard {
                    card_id: c.card_id,
                    question,
                    answer,
                    buttons: c.buttons,
                    next_reviews: c.next_reviews,
                    audio: crate::media::audio_from_html(&field_html, &self.anki),
                };
                self.card = Some(card);
                self.deck_finished = false;
            }
            Ok(None) => {
                self.card = None;
                self.deck_finished = true;
            }
            Err(e) => {
                self.status = Some(e.to_string());
                self.card = None;
            }
        }
    }

    /// Space bar: reveal the answer if hidden, else grade Good (like Anki).
    pub fn space(&mut self) {
        if self.answer_shown {
            self.grade(3);
        } else {
            self.show_answer();
        }
    }

    /// Reveal the answer side and play its audio.
    pub fn show_answer(&mut self) {
        if self.answer_shown || self.card.is_none() {
            return;
        }
        // A restored (undone) card is shown from memory, not the GUI reviewer,
        // so reveal it locally without touching the reviewer.
        if !self.undone
            && let Err(e) = self.anki.gui_show_answer()
        {
            self.status = Some(e.to_string());
            return;
        }
        self.answer_shown = true;
        self.scroll = 0;
        if let Some(card) = &self.card {
            card.answer.play_audio();
        }
    }

    /// Grade the current card with the given ease (1..=4), then advance.
    pub fn grade(&mut self, ease: i64) {
        if !self.answer_shown {
            return;
        }
        let Some(card) = &self.card else { return };
        if !card.buttons.contains(&ease) {
            self.status = Some(format!(
                "No '{}' button for this card",
                GRADE_LABELS
                    .get((ease - 1) as usize)
                    .copied()
                    .unwrap_or("?")
            ));
            return;
        }
        let card_id = card.card_id;

        if self.undone {
            // Re-grade the restored card by id; the GUI reviewer is sitting on
            // the next card and is left untouched.
            match self.anki.answer_cards(card_id, ease) {
                Ok(_) => {
                    self.status = None;
                    self.undone = false;
                    self.load_current_card();
                }
                Err(e) => self.status = Some(e.to_string()),
            }
            return;
        }

        match self.anki.gui_answer_card(ease) {
            Ok(_) => {
                self.status = None;
                // Stash the just-graded card so `u` can restore it.
                self.prev_card = self.card.take();
                self.load_current_card();
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Undo the last grade: revert it in Anki and restore the card for re-grading.
    pub fn undo(&mut self) {
        if self.undone {
            // Already showing the restored card.
            return;
        }
        let Some(prev) = self.prev_card.take() else {
            self.status = Some("Nothing to undo".to_string());
            return;
        };
        match self.anki.gui_undo() {
            Ok(true) => {
                self.card = Some(prev);
                self.undone = true;
                self.answer_shown = false;
                self.scroll = 0;
                self.deck_finished = false;
                self.status = Some("Undone — re-grade this card".to_string());
            }
            Ok(false) => {
                self.prev_card = Some(prev);
                self.status = Some("Nothing to undo".to_string());
            }
            Err(e) => {
                self.prev_card = Some(prev);
                self.status = Some(e.to_string());
            }
        }
    }

    /// Suspend the current card so it stops appearing in reviews, then advance.
    /// The card is stashed in `prev_card` so `u` can un-suspend and restore it
    /// (via the same `guiUndo` path as undoing a grade).
    pub fn suspend_card(&mut self) {
        let Some(card) = &self.card else { return };
        let card_id = card.card_id;
        match self.anki.suspend(card_id) {
            Ok(_) => {
                self.status = Some("Card suspended — u to undo".to_string());
                if self.undone {
                    // The GUI reviewer is already sitting on the next card; the
                    // suspended card was only shown locally, so just reload.
                    self.undone = false;
                    self.prev_card = self.card.take();
                    self.load_current_card();
                } else if let Err(e) = self.anki.gui_deck_review(&self.deck_name) {
                    // Suspending doesn't move the GUI reviewer off the card, so
                    // continue the deck review to advance to the next due card.
                    self.status = Some(e.to_string());
                } else {
                    self.prev_card = self.card.take();
                    self.load_current_card();
                }
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Replay the card's audio clips.
    pub fn replay_audio(&mut self) {
        match &self.card {
            Some(card) if !card.audio.is_empty() => crate::media::play_clips(&card.audio),
            Some(_) => self.status = Some("No audio on this card".to_string()),
            None => {}
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// Open the card-info popup for the current card, or close it if already open
    /// (mirrors Anki's `i` shortcut). A no-op when no card is shown.
    pub fn toggle_stats(&mut self) {
        if self.stats.is_some() {
            self.stats = None;
            return;
        }
        let Some(card) = &self.card else { return };
        match self.build_stats(card.card_id) {
            Ok(stats) => {
                self.stats = Some(stats);
                self.stats_scroll = 0;
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    pub fn stats_scroll_down(&mut self) {
        self.stats_scroll = self.stats_scroll.saturating_add(1);
    }

    pub fn stats_scroll_up(&mut self) {
        self.stats_scroll = self.stats_scroll.saturating_sub(1);
    }

    /// Gather scheduling metadata and review history for the given card and turn
    /// it into the label/value rows and history table shown in the popup.
    fn build_stats(&self, card_id: i64) -> Result<CardStats> {
        let info = self
            .anki
            .card_info(card_id)?
            .ok_or_else(|| anyhow::anyhow!("card not found"))?;
        let mut reviews = self.anki.card_reviews(card_id)?;
        reviews.sort_by_key(|r| r.id);

        let mut rows: Vec<(String, String)> = Vec::new();
        // The note id encodes the note's creation time (epoch ms).
        rows.push(("Added".into(), fmt_local(info.note, "%Y-%m-%d")));
        if let Some(first) = reviews.first() {
            rows.push(("First Review".into(), fmt_local(first.id, "%Y-%m-%d")));
        }
        if let Some(last) = reviews.last() {
            rows.push(("Latest Review".into(), fmt_local(last.id, "%Y-%m-%d")));
            // A review card's next due date is its last review plus the interval.
            if info.queue == REVIEW_QUEUE && info.interval > 0 {
                let due = last.id + info.interval * MS_PER_DAY;
                rows.push(("Due".into(), fmt_local(due, "%Y-%m-%d")));
            }
        }
        rows.push(("Interval".into(), fmt_days(info.interval)));
        // Ease factor only applies to SM-2 scheduling; FSRS reports 0.
        if info.factor > 0 {
            rows.push(("Ease".into(), format!("{}%", info.factor / 10)));
        }
        rows.push(("Reviews".into(), info.reps.to_string()));
        rows.push(("Lapses".into(), info.lapses.to_string()));
        if !reviews.is_empty() {
            let total_ms: i64 = reviews.iter().map(|r| r.time).sum();
            let avg_ms = total_ms / reviews.len() as i64;
            rows.push(("Average Time".into(), fmt_duration(avg_ms, true)));
            rows.push(("Total Time".into(), fmt_duration(total_ms, true)));
        }
        rows.push(("Note Type".into(), info.model_name));
        rows.push(("Deck".into(), info.deck_name));
        rows.push(("Card ID".into(), card_id.to_string()));
        rows.push(("Note ID".into(), info.note.to_string()));

        // Newest review first, as Anki shows it.
        let history = reviews
            .iter()
            .rev()
            .map(|r| ReviewRow {
                date: fmt_local(r.id, "%Y-%m-%d @ %H:%M"),
                kind: r.kind,
                rating: r.ease,
                interval: fmt_revlog_ivl(r.ivl),
                time: fmt_duration(r.time, false),
            })
            .collect();

        Ok(CardStats { rows, history })
    }
}

/// AnkiConnect's `queue` value for a card in the review (graduated) queue.
const REVIEW_QUEUE: i64 = 2;
/// Milliseconds in a day, for projecting a due date from a review timestamp.
const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// Format an epoch-ms timestamp in local time with the given `chrono` format.
fn fmt_local(ms: i64, fmt: &str) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format(fmt).to_string())
        .unwrap_or_else(|| "?".into())
}

/// Human-readable interval given a count of days (the card's current interval).
fn fmt_days(days: i64) -> String {
    if days <= 0 {
        "new".into()
    } else if days == 1 {
        "1 day".into()
    } else if days < 30 {
        format!("{days} days")
    } else if days < 365 {
        format!("{:.1} months", days as f64 / 30.0)
    } else {
        format!("{:.1} years", days as f64 / 365.0)
    }
}

/// Human-readable interval from a revlog `ivl` (negative = seconds, positive = days).
fn fmt_revlog_ivl(ivl: i64) -> String {
    if ivl >= 0 {
        return fmt_days(ivl);
    }
    let secs = -ivl;
    if secs < 60 {
        format!("{secs} second{}", plural(secs))
    } else if secs < 3600 {
        let mins = secs / 60;
        format!("{mins} minute{}", plural(mins))
    } else {
        format!("{:.1} hours", secs as f64 / 3600.0)
    }
}

/// `"s"` unless `n` is exactly 1, for simple pluralization.
fn plural(n: i64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Format a millisecond duration. `verbose` gives the long form for the
/// average/total rows (`19.6 seconds`), else the compact history form (`7.25s`).
fn fmt_duration(ms: i64, verbose: bool) -> String {
    let secs = ms as f64 / 1000.0;
    match (secs < 60.0, verbose) {
        (true, true) => format!("{secs:.1} seconds"),
        (true, false) => format!("{secs:.2}s"),
        (false, true) => format!("{:.2} minutes", secs / 60.0),
        (false, false) => format!("{:.1}m", secs / 60.0),
    }
}
