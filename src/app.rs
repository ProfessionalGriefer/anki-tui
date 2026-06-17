//! Application state machine: deck list and review screens.

use std::collections::HashSet;
use std::path::PathBuf;

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
    /// Audio clips pulled from the note's fields, replayed on demand with `r`.
    /// Card-level (not per-side) because the rendered HTML doesn't reveal which
    /// side a `[sound:...]` belongs to.
    pub audio: Vec<PathBuf>,
}

/// One row of the review-history table in the card-info popup. `kind` is the
/// raw revlog type (0 learn, 1 review, 2 relearn, 3 filtered); the UI maps it
/// to a label and color.
pub struct ReviewRow {
    pub date: String,
    pub kind: i64,
    pub rating: i64,
    pub interval: String,
    pub time: String,
}

/// Card-info popup contents: a list of label/value rows plus the review history,
/// built on demand when the user presses `i`. FSRS-only stats (stability,
/// difficulty, retrievability) are omitted because AnkiConnect doesn't expose them.
pub struct CardStats {
    pub rows: Vec<(String, String)>,
    pub history: Vec<ReviewRow>,
}

/// Deck-stats popup contents (Anki's `t` panel): label/value rows for the
/// selected deck, built on demand from `getDeckStats` and `findCards` counts.
pub struct DeckStats {
    pub name: String,
    pub rows: Vec<(String, String)>,
}

/// Today's review activity for the deck-list footer, mirroring Anki's
/// "Studied N cards in T minutes today (Xs/card)" line. `cards` counts reviews
/// (revlog entries), `total_ms` is the time spent on them.
#[derive(Default, Clone, Copy)]
pub struct StudiedToday {
    pub cards: u32,
    pub total_ms: i64,
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
    /// Collection-wide review activity today, shown in the deck-list footer.
    pub studied_today: StudiedToday,
    /// True after a lone `g`, so the next `g` completes a vim `gg` (jump to top).
    pub pending_g: bool,
    /// Case-insensitive substring filter for the deck list.
    pub search: String,
    /// Whether the search input is currently capturing keystrokes.
    pub searching: bool,
    /// When `Some`, the deck-stats popup is open over the deck list.
    pub deck_stats: Option<DeckStats>,

    // Review state.
    pub deck_name: String,
    /// Remaining new/learn/review counts for the deck under review, shown in the
    /// title bar like Anki's reviewer. Refreshed each time a card loads.
    pub review_counts: DeckCounts,
    pub card: Option<ReviewCard>,
    pub answer_shown: bool,
    pub scroll: u16,
    pub deck_finished: bool,
    /// The previously graded card, kept so `u` can restore and re-grade it.
    pub prev_card: Option<ReviewCard>,
    /// True while showing a restored (undone) card that is re-graded locally
    /// via `answerCards` rather than the GUI reviewer.
    pub undone: bool,
    /// When `Some`, the card-info popup is open over the review screen.
    pub stats: Option<CardStats>,
    /// Vertical scroll offset within the card-info popup.
    pub stats_scroll: u16,
    /// Whether the keybinding help popup is open.
    pub help_open: bool,

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
        let mut app = Self {
            anki,
            picker,
            screen: Screen::DeckList,
            decks,
            deck_selected: 0,
            collapsed,
            flat_view: false,
            studied_today: StudiedToday::default(),
            pending_g: false,
            search: String::new(),
            searching: false,
            deck_stats: None,
            deck_name: String::new(),
            review_counts: DeckCounts::default(),
            card: None,
            answer_shown: false,
            scroll: 0,
            deck_finished: false,
            prev_card: None,
            undone: false,
            stats: None,
            stats_scroll: 0,
            help_open: false,
            status: None,
            should_quit: false,
        };
        app.refresh_studied_today();
        Ok(app)
    }

    pub fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
    }
}
