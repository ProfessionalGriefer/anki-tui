//! Application state machine: deck list and review screens.

use anyhow::Result;
use ratatui_image::picker::Picker;

use crate::anki::AnkiConnect;
use crate::media::SideMedia;

/// Which screen the user is currently looking at.
pub enum Screen {
    DeckList,
    Review,
}

/// The card currently under review, with media for both sides.
pub struct ReviewCard {
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
    pub decks: Vec<String>,
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

    /// Transient status / error message shown in the footer.
    pub status: Option<String>,
    pub should_quit: bool,
}

/// The four grade labels, indexed by ease - 1.
pub const GRADE_LABELS: [&str; 4] = ["Again", "Hard", "Good", "Easy"];

impl App {
    pub fn new(picker: Picker) -> Result<Self> {
        let anki = AnkiConnect::new();
        let decks = load_sorted_decks(&anki)?;
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
            status: None,
        should_quit: false,
        })
    }

    // ----- Deck list -----

    /// Deck names matching the current search filter, in display order.
    pub fn filtered_decks(&self) -> Vec<&String> {
        if self.search.is_empty() {
            self.decks.iter().collect()
        } else {
            let query = self.search.to_lowercase();
            self.decks
                .iter()
                .filter(|d| d.to_lowercase().contains(&query))
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
        match load_sorted_decks(&self.anki) {
            Ok(decks) => {
                self.decks = decks;
                self.clamp_selection();
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Start reviewing the highlighted deck.
    pub fn enter_review(&mut self) {
        let Some(deck) = self.filtered_decks().get(self.deck_selected).map(|d| (*d).clone())
        else {
            return;
        };
        self.deck_name = deck.clone();
        self.screen = Screen::Review;
        self.deck_finished = false;
        self.status = None;
        if let Err(e) = self.anki.gui_deck_review(&deck) {
            self.status = Some(e.to_string());
        }
        self.load_current_card();
    }

    pub fn back_to_decks(&mut self) {
        self.screen = Screen::DeckList;
        self.card = None;
        self.answer_shown = false;
        self.scroll = 0;
        self.refresh_decks();
    }

    // ----- Review -----

    /// Fetch the reviewer's current card and build its media.
    pub fn load_current_card(&mut self) {
        self.answer_shown = false;
        self.scroll = 0;
        match self.anki.gui_current_card() {
            Ok(Some(c)) => {
                let question = SideMedia::build(&c.question, &self.anki, &self.picker);
                let answer = SideMedia::build(&c.answer, &self.anki, &self.picker);
                let card = ReviewCard {
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

    /// Reveal the answer side and play its audio.
    pub fn show_answer(&mut self) {
        if self.answer_shown || self.card.is_none() {
            return;
        }
        match self.anki.gui_show_answer() {
            Ok(_) => {
                self.answer_shown = true;
                self.scroll = 0;
                if let Some(card) = &self.card {
                    card.answer.play_audio();
                }
            }
            Err(e) => self.status = Some(e.to_string()),
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
        match self.anki.gui_answer_card(ease) {
            Ok(_) => {
                self.status = None;
                self.load_current_card();
            }
            Err(e) => self.status = Some(e.to_string()),
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

/// Load deck names sorted case-insensitively for stable display.
fn load_sorted_decks(anki: &AnkiConnect) -> Result<Vec<String>> {
    let mut decks = anki.deck_names()?;
    decks.sort_by_key(|d| d.to_lowercase());
    Ok(decks)
}
