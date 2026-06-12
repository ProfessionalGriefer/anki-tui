//! Application state machine: deck list and review screens.

use std::collections::HashSet;

use anyhow::Result;
use ratatui_image::picker::Picker;

use crate::anki::{AnkiConnect, DeckCounts};
use crate::media::SideMedia;

mod decklist;
mod persist;
mod review;

use persist::{default_collapsed, load_collapsed, load_decks};

/// Which screen the user is currently looking at.
pub enum Screen {
    DeckList,
    Review,
}

/// A deck plus its new/learn/review counts for the overview.
pub struct DeckInfo {
    /// Full `A::B::C` name.
    pub name: String,
    /// Last `::` component, shown in the tree.
    pub label: String,
    /// Nesting depth (number of `::` separators).
    pub depth: usize,
    /// Whether any other deck is nested under this one.
    pub has_children: bool,
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
    /// Full names of decks whose children are folded away. Persisted to disk.
    pub collapsed: HashSet<String>,
    /// When true, show a flat list of full deck names instead of the fold tree.
    pub flat_view: bool,
    /// True after a lone `g`, so the next `g` completes a vim `gg` (jump to top).
    pub pending_g: bool,
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

impl App {
    pub fn new(picker: Picker) -> Result<Self> {
        let anki = AnkiConnect::new();
        let decks = load_decks(&anki)?;
        // Restore the saved fold state, or default to collapsing every parent
        // (so only top-level decks show on first launch).
        let collapsed = load_collapsed().unwrap_or_else(|| default_collapsed(&decks));
        Ok(Self {
            anki,
            picker,
            screen: Screen::DeckList,
            decks,
            deck_selected: 0,
            collapsed,
            flat_view: false,
            pending_g: false,
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
}
