//! Application state machine: deck list and review screens.

use anyhow::Result;
use ratatui_image::picker::Picker;

use crate::anki::{AnkiConnect, DeckCounts};
use crate::media::SideMedia;

/// Which screen the user is currently looking at.
pub enum Screen {
    DeckList,
    Review,
}

/// A deck plus its new/learn/review counts for the overview.
pub struct DeckInfo {
    pub name: String,
    pub counts: DeckCounts,
}

/// The card currently under review, with media for both sides.
pub struct ReviewCard {
    pub card_id: i64,
    pub question: SideMedia,
    pub answer: SideMedia,
    pub buttons: Vec<i64>,
    pub next_reviews: Vec<String>,
}

pub struct App {
    pub anki: AnkiConnect,
    pub picker: Picker,
    pub screen: Screen,

    // Deck list state.
    pub decks: Vec<DeckInfo>,
    pub deck_selected: usize,
    /// Case-insensitive substring filter for the deck list.
    pub search: String,
    /// Whether the search input is currently capturing keystrokes.
    pub searching: bool,

    // Review state.
    pub deck_name: String,
    pub card: Option<ReviewCard>,
    pub answer_shown: bool,
    pub scroll: u16,
    pub deck_finished: bool,
    /// The previously graded card, kept so `u` can restore and re-grade it.
    pub prev_card: Option<ReviewCard>,
    /// True while showing a restored (undone) card that is re-graded locally
    /// via `answerCards` rather than the GUI reviewer.
    pub undone: bool,

    /// Transient status / error message shown in the footer.
    pub status: Option<String>,
    pub should_quit: bool,
}

/// The four grade labels, indexed by ease - 1.
pub const GRADE_LABELS: [&str; 4] = ["Again", "Hard", "Good", "Easy"];

/// How many rows `Ctrl-d`/`Ctrl-u` jump in the deck list.
const PAGE_JUMP: usize = 10;

impl App {
    pub fn new(picker: Picker) -> Result<Self> {
        let anki = AnkiConnect::new();
        let decks = load_decks(&anki)?;
        Ok(Self {
            anki,
            picker,
            screen: Screen::DeckList,
            decks,
            deck_selected: 0,
            search: String::new(),
            searching: false,
            deck_name: String::new(),
            card: None,
            answer_shown: false,
            scroll: 0,
            deck_finished: false,
            prev_card: None,
            undone: false,
            status: None,
        should_quit: false,
        })
    }

    // ----- Deck list -----

    /// Decks matching the current search filter, in display order.
    pub fn filtered_decks(&self) -> Vec<&DeckInfo> {
        if self.search.is_empty() {
            self.decks.iter().collect()
        } else {
            let query = self.search.to_lowercase();
            self.decks
                .iter()
                .filter(|d| d.name.to_lowercase().contains(&query))
                .collect()
        }
    }

    pub fn select_next_deck(&mut self) {
        let len = self.filtered_decks().len();
        if len > 0 {
            self.deck_selected = (self.deck_selected + 1).min(len - 1);
        }
    }

    pub fn select_prev_deck(&mut self) {
        self.deck_selected = self.deck_selected.saturating_sub(1);
    }

    /// Vim `Ctrl-d`: jump the selection down by half a page.
    pub fn select_page_down(&mut self) {
        let len = self.filtered_decks().len();
        if len > 0 {
            self.deck_selected = (self.deck_selected + PAGE_JUMP).min(len - 1);
        }
    }

    /// Vim `Ctrl-u`: jump the selection up by half a page.
    pub fn select_page_up(&mut self) {
        self.deck_selected = self.deck_selected.saturating_sub(PAGE_JUMP);
    }

    /// Begin capturing keystrokes into the search filter.
    pub fn start_search(&mut self) {
        self.searching = true;
    }

    /// Append a character to the search filter and reset the selection.
    pub fn push_search(&mut self, c: char) {
        self.search.push(c);
        self.deck_selected = 0;
    }

    /// Delete the last search character and reset the selection.
    pub fn backspace_search(&mut self) {
        self.search.pop();
        self.deck_selected = 0;
    }

    /// Keep the current filter but stop capturing keystrokes.
    pub fn confirm_search(&mut self) {
        self.searching = false;
    }

    /// Clear the filter and stop capturing keystrokes.
    pub fn cancel_search(&mut self) {
        self.searching = false;
        self.search.clear();
        self.deck_selected = 0;
    }

    /// Clamp the selection to the current filtered list length.
    fn clamp_selection(&mut self) {
        let len = self.filtered_decks().len();
        if self.deck_selected >= len {
            self.deck_selected = len.saturating_sub(1);
        }
    }

    /// Reload the deck list (e.g. when returning from review).
    pub fn refresh_decks(&mut self) {
        match load_decks(&self.anki) {
            Ok(decks) => {
                self.decks = decks;
                self.clamp_selection();
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Start reviewing the highlighted deck.
    pub fn enter_review(&mut self) {
        let Some(deck) = self
            .filtered_decks()
            .get(self.deck_selected)
            .map(|d| d.name.clone())
        else {
            return;
        };
        self.deck_name = deck.clone();
        self.screen = Screen::Review;
        self.deck_finished = false;
        self.status = None;
        self.prev_card = None;
        self.undone = false;
        if let Err(e) = self.anki.gui_deck_review(&deck) {
            self.status = Some(e.to_string());
        }
        self.load_current_card();
    }

    pub fn back_to_decks(&mut self) {
        self.screen = Screen::DeckList;
        self.card = None;
        self.prev_card = None;
        self.undone = false;
        self.answer_shown = false;
        self.scroll = 0;
        self.refresh_decks();
    }

    // ----- Review -----

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

/// Load decks with their new/learn/review counts, sorted case-insensitively.
fn load_decks(anki: &AnkiConnect) -> Result<Vec<DeckInfo>> {
    let names_and_ids = anki.deck_names_and_ids()?;
    let names: Vec<String> = names_and_ids.keys().cloned().collect();
    // Counts are best-effort; fall back to zeros if the stats call fails.
    let stats = anki.deck_stats(&names).unwrap_or_default();

    let mut decks: Vec<DeckInfo> = names_and_ids
        .into_iter()
        .map(|(name, id)| DeckInfo {
            name,
            counts: stats.get(&id).copied().unwrap_or_default(),
        })
        .collect();
    decks.sort_by_key(|d| d.name.to_lowercase());
    Ok(decks)
}
