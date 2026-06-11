//! Application state machine: deck list and review screens.

use std::collections::HashSet;
use std::path::PathBuf;

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

/// How many rows `Ctrl-d`/`Ctrl-u` jump in the deck list.
const PAGE_JUMP: usize = 10;

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

    // ----- Deck list -----

    /// Decks currently shown: filtered by the search query, or (when not
    /// searching) hiding any deck whose ancestor is collapsed.
    pub fn visible_decks(&self) -> Vec<&DeckInfo> {
        if !self.search.is_empty() {
            let query = self.search.to_lowercase();
            self.decks
                .iter()
                .filter(|d| d.name.to_lowercase().contains(&query))
                .collect()
        } else if self.flat_view {
            self.decks.iter().collect()
        } else {
            self.decks
                .iter()
                .filter(|d| !self.ancestor_collapsed(&d.name))
                .collect()
        }
    }

    /// `,`: toggle between the fold tree and a flat list of full deck names,
    /// keeping the same deck selected across the switch.
    pub fn toggle_view(&mut self) {
        let current = self
            .visible_decks()
            .get(self.deck_selected)
            .map(|d| d.name.clone());
        self.flat_view = !self.flat_view;
        match current.and_then(|name| self.visible_decks().iter().position(|d| d.name == name)) {
            Some(idx) => self.deck_selected = idx,
            None => self.clamp_selection(),
        }
    }

    /// True if any strict ancestor of `name` is in the collapsed set.
    fn ancestor_collapsed(&self, name: &str) -> bool {
        let parts: Vec<&str> = name.split("::").collect();
        let mut prefix = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push_str("::");
            }
            prefix.push_str(part);
            if self.collapsed.contains(&prefix) {
                return true;
            }
        }
        false
    }

    pub fn select_next_deck(&mut self) {
        let len = self.visible_decks().len();
        if len > 0 {
            self.deck_selected = (self.deck_selected + 1).min(len - 1);
        }
    }

    pub fn select_prev_deck(&mut self) {
        self.deck_selected = self.deck_selected.saturating_sub(1);
    }

    /// Vim `gg`: jump to the first deck.
    pub fn select_first(&mut self) {
        self.deck_selected = 0;
    }

    /// Vim `G`: jump to the last visible deck.
    pub fn select_last(&mut self) {
        self.deck_selected = self.visible_decks().len().saturating_sub(1);
    }

    /// Handle a `g` press: complete `gg` (jump to top) or arm the next `g`.
    pub fn press_g(&mut self) {
        if self.pending_g {
            self.select_first();
            self.pending_g = false;
        } else {
            self.pending_g = true;
        }
    }

    /// `l` / Right: expand a collapsed parent, otherwise review the deck.
    pub fn expand_or_review(&mut self) {
        let visible = self.visible_decks();
        let Some(d) = visible.get(self.deck_selected) else {
            return;
        };
        let name = d.name.clone();
        let has_children = d.has_children;
        drop(visible);
        if has_children && self.collapsed.contains(&name) {
            self.collapsed.remove(&name);
            self.save_collapsed();
        } else {
            self.enter_review();
        }
    }

    /// `h` / Left: collapse an expanded parent, otherwise jump to the parent deck.
    pub fn collapse_or_parent(&mut self) {
        let visible = self.visible_decks();
        let Some(d) = visible.get(self.deck_selected) else {
            return;
        };
        let name = d.name.clone();
        let has_children = d.has_children;
        drop(visible);
        if has_children && !self.collapsed.contains(&name) {
            self.collapsed.insert(name);
            self.save_collapsed();
        } else if let Some(pos) = name.rfind("::") {
            let parent = name[..pos].to_string();
            if let Some(idx) = self.visible_decks().iter().position(|x| x.name == parent) {
                self.deck_selected = idx;
            }
        }
    }

    /// Persist the current fold state to disk.
    fn save_collapsed(&self) {
        save_collapsed(&self.collapsed);
    }

    /// Sync the collection with AnkiWeb, then refresh deck counts.
    pub fn sync(&mut self) {
        match self.anki.sync() {
            Ok(()) => {
                if matches!(self.screen, Screen::DeckList) {
                    self.refresh_decks();
                }
                self.status = Some("Synced with AnkiWeb".to_string());
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Vim `Ctrl-d`: jump the selection down by half a page.
    pub fn select_page_down(&mut self) {
        let len = self.visible_decks().len();
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

    /// Clamp the selection to the current visible list length.
    fn clamp_selection(&mut self) {
        let len = self.visible_decks().len();
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
            .visible_decks()
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

    // Every full name that is a strict prefix of another deck has children.
    let mut parents: HashSet<String> = HashSet::new();
    for name in &names {
        let parts: Vec<&str> = name.split("::").collect();
        let mut prefix = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push_str("::");
            }
            prefix.push_str(part);
            parents.insert(prefix.clone());
        }
    }

    let mut decks: Vec<DeckInfo> = names_and_ids
        .into_iter()
        .map(|(name, id)| {
            let label = name.rsplit("::").next().unwrap_or(&name).to_string();
            let depth = name.matches("::").count();
            DeckInfo {
                has_children: parents.contains(&name),
                label,
                depth,
                counts: stats.get(&id).copied().unwrap_or_default(),
                name,
            }
        })
        .collect();
    decks.sort_by_key(|d| d.name.to_lowercase());
    Ok(decks)
}

/// Default fold state: collapse every parent so only top-level decks show.
fn default_collapsed(decks: &[DeckInfo]) -> HashSet<String> {
    decks
        .iter()
        .filter(|d| d.has_children)
        .map(|d| d.name.clone())
        .collect()
}

/// Path of the persisted fold-state file (`$XDG_STATE_HOME` or `~/.local/state`).
fn collapsed_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("anki-tui").join("collapsed.json"))
}

/// Load the persisted set of collapsed deck names, if any.
fn load_collapsed() -> Option<HashSet<String>> {
    let data = std::fs::read_to_string(collapsed_path()?).ok()?;
    let names: Vec<String> = serde_json::from_str(&data).ok()?;
    Some(names.into_iter().collect())
}

/// Write the collapsed deck names to disk (best-effort).
fn save_collapsed(collapsed: &HashSet<String>) {
    let Some(path) = collapsed_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut names: Vec<&String> = collapsed.iter().collect();
    names.sort();
    if let Ok(json) = serde_json::to_string_pretty(&names) {
        let _ = std::fs::write(path, json);
    }
}
