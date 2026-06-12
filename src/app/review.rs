//! Review screen: card loading, revealing the answer, grading, and undo.

use super::{App, GRADE_LABELS, ReviewCard};
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
}
