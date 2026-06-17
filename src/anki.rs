//! Thin blocking client over the AnkiConnect HTTP API (version 6).

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct AnkiConnect {
    url: String,
    client: reqwest::blocking::Client,
}

/// Shape of every AnkiConnect response: `{ "result": ..., "error": ... }`.
#[derive(Deserialize)]
struct AnkiResponse {
    result: Value,
    error: Option<String>,
}

/// New / learning / review (due) counts for a deck, as shown in Anki's overview.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeckCounts {
    pub new: u32,
    pub learn: u32,
    pub review: u32,
}

/// The current card shown in Anki's GUI reviewer (`guiCurrentCard`).
#[derive(Debug, Deserialize)]
pub struct CurrentCard {
    pub question: String,
    pub answer: String,
    #[serde(rename = "cardId")]
    pub card_id: i64,
    /// Ease values that have a grading button (e.g. `[1, 3]` or `[1, 2, 3, 4]`).
    #[serde(default)]
    pub buttons: Vec<i64>,
    /// Next-interval preview labels aligned with `buttons` (e.g. `["<1m", "10m"]`).
    #[serde(default, rename = "nextReviews")]
    pub next_reviews: Vec<String>,
    /// Raw note fields keyed by name. Unlike the rendered `question`/`answer`
    /// HTML (where Anki replaces `[sound:...]` with replay buttons), these still
    /// carry the original `[sound:...]` tokens, so they are the only reliable
    /// source of audio filenames.
    #[serde(default)]
    pub fields: std::collections::HashMap<String, Field>,
}

/// One note field value from `guiCurrentCard`.
#[derive(Debug, Deserialize)]
pub struct Field {
    pub value: String,
    #[serde(default)]
    pub order: i64,
}

/// Scheduling metadata for a single card (subset of `cardsInfo`).
#[derive(Debug, Deserialize)]
pub struct CardInfo {
    #[serde(rename = "deckName")]
    pub deck_name: String,
    #[serde(rename = "modelName")]
    pub model_name: String,
    /// Note id, which also encodes the note's creation time (epoch ms).
    pub note: i64,
    /// SM-2 ease factor in permille (2500 = 250%); 0 under FSRS.
    pub factor: i64,
    /// Current interval in days.
    pub interval: i64,
    /// Total reviews.
    pub reps: i64,
    /// Times the card lapsed (forgotten after graduating).
    pub lapses: i64,
    /// Scheduling queue (2 = review).
    pub queue: i64,
}

/// One revlog entry for a card (`getReviewsOfCards`).
#[derive(Debug, Deserialize)]
pub struct ReviewEntry {
    /// Review time, epoch ms.
    pub id: i64,
    /// Button pressed: 1 (Again) .. 4 (Easy).
    pub ease: i64,
    /// New interval: negative = seconds, positive = days.
    pub ivl: i64,
    /// Time taken, ms.
    pub time: i64,
    /// 0 = learn, 1 = review, 2 = relearn, 3 = filtered/cram.
    #[serde(rename = "type")]
    pub kind: i64,
}

impl AnkiConnect {
    pub fn new() -> Self {
        let url = std::env::var("ANKI_CONNECT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8765".to_string());
        Self {
            url,
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Perform a single AnkiConnect action and return its `result` value.
    fn invoke(&self, action: &str, params: Value) -> Result<Value> {
        // AnkiConnect's schema requires `params` to be an object, so paramless
        // actions must send `{}` rather than `null`.
        let params = if params.is_null() {
            json!({})
        } else {
            params
        };
        let body = json!({
            "action": action,
            "version": 6,
            "params": params,
        });
        let resp: AnkiResponse = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .with_context(|| {
                format!(
                    "could not reach AnkiConnect at {} — is Anki running with the AnkiConnect add-on?",
                    self.url
                )
            })?
            .json()
            .context("invalid response from AnkiConnect")?;

        if let Some(err) = resp.error {
            bail!("AnkiConnect error for `{action}`: {err}");
        }
        Ok(resp.result)
    }

    /// Full deck names mapped to their deck ids.
    pub fn deck_names_and_ids(&self) -> Result<HashMap<String, i64>> {
        let result = self.invoke("deckNamesAndIds", Value::Null)?;
        Ok(serde_json::from_value(result)?)
    }

    /// New / learning / review (due) counts for each given deck, keyed by deck id.
    pub fn deck_stats(&self, decks: &[String]) -> Result<HashMap<i64, DeckCounts>> {
        #[derive(Deserialize)]
        struct Raw {
            deck_id: i64,
            new_count: u32,
            learn_count: u32,
            review_count: u32,
        }
        let result = self.invoke("getDeckStats", json!({ "decks": decks }))?;
        let raw: HashMap<String, Raw> = serde_json::from_value(result)?;
        Ok(raw
            .into_values()
            .map(|r| {
                (
                    r.deck_id,
                    DeckCounts {
                        new: r.new_count,
                        learn: r.learn_count,
                        review: r.review_count,
                    },
                )
            })
            .collect())
    }

    /// Card ids matching an Anki search query (`findCards`). Used to count cards
    /// by state for the deck-stats popup.
    pub fn find_cards(&self, query: &str) -> Result<Vec<i64>> {
        let result = self.invoke("findCards", json!({ "query": query }))?;
        Ok(serde_json::from_value(result)?)
    }

    /// Start reviewing the given deck in Anki's GUI reviewer.
    pub fn gui_deck_review(&self, deck: &str) -> Result<bool> {
        let result = self.invoke("guiDeckReview", json!({ "name": deck }))?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// The card currently displayed by the reviewer, or `None` when the deck is done.
    pub fn gui_current_card(&self) -> Result<Option<CurrentCard>> {
        let result = match self.invoke("guiCurrentCard", Value::Null) {
            Ok(result) => result,
            // When the last card is graded Anki leaves the reviewer (showing its
            // own congrats screen), so this action errors instead of returning
            // null. Treat that as "no card left" rather than a failure.
            Err(e) if e.to_string().contains("Gui review is not currently active") => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(result)?))
    }

    /// Flip the reviewer to show the answer side.
    pub fn gui_show_answer(&self) -> Result<bool> {
        let result = self.invoke("guiShowAnswer", Value::Null)?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// Grade the current card. `ease` is 1 (Again) .. 4 (Easy).
    pub fn gui_answer_card(&self, ease: i64) -> Result<bool> {
        let result = self.invoke("guiAnswerCard", json!({ "ease": ease }))?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// Undo the last reviewer action (e.g. an accidental grade). Returns `true`
    /// if something was undone. Note: this reverts the collection but does NOT
    /// move Anki's GUI reviewer back to the undone card.
    pub fn gui_undo(&self) -> Result<bool> {
        let result = self.invoke("guiUndo", Value::Null)?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// Answer a specific card by id with the given ease, independent of the GUI
    /// reviewer's current card. Returns `true` if the card existed.
    pub fn answer_cards(&self, card_id: i64, ease: i64) -> Result<bool> {
        let result = self.invoke(
            "answerCards",
            json!({ "answers": [{ "cardId": card_id, "ease": ease }] }),
        )?;
        Ok(result
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    /// Suspend a card so it stops appearing in reviews. Returns `true` if the
    /// card wasn't already suspended. Note: this only changes the collection; it
    /// does NOT move Anki's GUI reviewer off the card.
    pub fn suspend(&self, card_id: i64) -> Result<bool> {
        let result = self.invoke("suspend", json!({ "cards": [card_id] }))?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// Scheduling metadata for a single card, or `None` if it doesn't exist.
    pub fn card_info(&self, card_id: i64) -> Result<Option<CardInfo>> {
        let result = self.invoke("cardsInfo", json!({ "cards": [card_id] }))?;
        let mut infos: Vec<CardInfo> = serde_json::from_value(result)?;
        Ok(infos.drain(..).next())
    }

    /// Full review history for a card, in the order AnkiConnect returns it.
    /// Note: this action only works with an integer card id, not a string.
    pub fn card_reviews(&self, card_id: i64) -> Result<Vec<ReviewEntry>> {
        let result = self.invoke("getReviewsOfCards", json!({ "cards": [card_id] }))?;
        // Shape: `{ "<card id>": [ {entry}, ... ] }`.
        let mut map: HashMap<String, Vec<ReviewEntry>> = serde_json::from_value(result)?;
        Ok(map.drain().next().map(|(_, v)| v).unwrap_or_default())
    }

    /// Synchronize the local collection with AnkiWeb (uses Anki's saved login).
    pub fn sync(&self) -> Result<()> {
        self.invoke("sync", Value::Null)?;
        Ok(())
    }

    /// Retrieve a media file's bytes by filename. Returns `None` if it doesn't exist.
    pub fn retrieve_media_file(&self, filename: &str) -> Result<Option<Vec<u8>>> {
        use base64::Engine;
        let result = self.invoke("retrieveMediaFile", json!({ "filename": filename }))?;
        match result {
            // AnkiConnect returns `false` when the file is missing.
            Value::Bool(false) => Ok(None),
            Value::String(b64) => Ok(Some(
                base64::engine::general_purpose::STANDARD
                    .decode(b64.as_bytes())
                    .context("media file was not valid base64")?,
            )),
            _ => Ok(None),
        }
    }
}
