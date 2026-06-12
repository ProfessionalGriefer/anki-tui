//! Review screen: card loading, revealing the answer, grading, and undo.

use anyhow::Result;
use chrono::{Local, TimeZone};

use super::{App, CardStats, GRADE_LABELS, ReviewCard, ReviewKind, ReviewRow};
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
                let card = ReviewCard {
                    card_id: c.card_id,
                    question,
                    answer,
                    buttons: c.buttons,
                    next_reviews: c.next_reviews,
                };
                // Autoplay the question's audio.
                card.question.play_audio();
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

    /// Replay audio for the side currently shown.
    pub fn replay_audio(&self) {
        if let Some(card) = &self.card {
            if self.answer_shown {
                card.answer.play_audio();
            } else {
                card.question.play_audio();
            }
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
        rows.push(("Added".into(), fmt_date(info.note)));
        if let Some(first) = reviews.first() {
            rows.push(("First Review".into(), fmt_date(first.id)));
        }
        if let Some(last) = reviews.last() {
            rows.push(("Latest Review".into(), fmt_date(last.id)));
            // A review card's next due date is its last review plus the interval.
            if info.queue == 2 && info.interval > 0 {
                rows.push(("Due".into(), fmt_date_plus_days(last.id, info.interval)));
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
            rows.push(("Average Time".into(), fmt_duration_long(avg_ms)));
            rows.push(("Total Time".into(), fmt_duration_long(total_ms)));
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
                date: fmt_datetime(r.id),
                kind: ReviewKind::from_code(r.kind),
                rating: r.ease,
                interval: fmt_revlog_ivl(r.ivl),
                time: fmt_duration_short(r.time),
            })
            .collect();

        Ok(CardStats { rows, history })
    }
}

/// Format an epoch-ms timestamp as a local `YYYY-MM-DD` date.
fn fmt_date(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => "?".into(),
    }
}

/// Format an epoch-ms timestamp as a local `YYYY-MM-DD @ HH:MM`.
fn fmt_datetime(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%Y-%m-%d @ %H:%M").to_string(),
        None => "?".into(),
    }
}

/// Format `ms` plus a whole number of days, as a local date.
fn fmt_date_plus_days(ms: i64, days: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => (dt + chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string(),
        None => "?".into(),
    }
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

/// Compact duration for the history table's time column (e.g. `7.25s`, `1.8m`).
fn fmt_duration_short(ms: i64) -> String {
    let secs = ms as f64 / 1000.0;
    if secs < 60.0 {
        format!("{secs:.2}s")
    } else {
        format!("{:.1}m", secs / 60.0)
    }
}

/// Verbose duration for the average/total time rows (e.g. `19.6 seconds`).
fn fmt_duration_long(ms: i64) -> String {
    let secs = ms as f64 / 1000.0;
    if secs < 60.0 {
        format!("{secs:.1} seconds")
    } else {
        format!("{:.2} minutes", secs / 60.0)
    }
}
