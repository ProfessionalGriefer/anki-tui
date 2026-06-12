//! Deck-list screen: navigation, search, fold tree, and sync.

use super::persist::{load_decks, save_collapsed};
use super::{App, DeckInfo, Screen};

/// How many rows `Ctrl-d`/`Ctrl-u` jump in the deck list.
const PAGE_JUMP: usize = 10;

impl App {
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
    pub(super) fn clamp_selection(&mut self) {
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
}
