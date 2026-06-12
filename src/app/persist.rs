//! Disk persistence and deck loading: the standalone (non-`&self`) helpers
//! behind the `App` state machine.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use directories::ProjectDirs;

use super::DeckInfo;
use crate::anki::AnkiConnect;

/// Load decks with their new/learn/review counts, sorted case-insensitively.
pub(super) fn load_decks(anki: &AnkiConnect) -> Result<Vec<DeckInfo>> {
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
pub(super) fn default_collapsed(decks: &[DeckInfo]) -> HashSet<String> {
    decks
        .iter()
        .filter(|d| d.has_children)
        .map(|d| d.name.clone())
        .collect()
}

/// Path of the persisted fold-state file under the platform state/data dir.
fn collapsed_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "anki-tui")?;
    let base = dirs.state_dir().unwrap_or_else(|| dirs.data_dir());
    Some(base.join("collapsed.json"))
}

/// Load the persisted set of collapsed deck names, if any.
pub(super) fn load_collapsed() -> Option<HashSet<String>> {
    let data = std::fs::read_to_string(collapsed_path()?).ok()?;
    let names: Vec<String> = serde_json::from_str(&data).ok()?;
    Some(names.into_iter().collect())
}

/// Write the collapsed deck names to disk (best-effort).
pub(super) fn save_collapsed(collapsed: &HashSet<String>) {
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
