//! Thin blocking client over the AnkiConnect HTTP API (version 6).

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

/// The current card shown in Anki's GUI reviewer (`guiCurrentCard`).
#[derive(Debug, Deserialize)]
pub struct CurrentCard {
    pub question: String,
    pub answer: String,
    // Part of the response; reviewing is driven through the GUI so we don't act on it.
    #[allow(dead_code)]
    #[serde(rename = "cardId")]
    pub card_id: i64,
    /// Ease values that have a grading button (e.g. `[1, 3]` or `[1, 2, 3, 4]`).
    #[serde(default)]
    pub buttons: Vec<i64>,
    /// Next-interval preview labels aligned with `buttons` (e.g. `["<1m", "10m"]`).
    #[serde(default, rename = "nextReviews")]
    pub next_reviews: Vec<String>,
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

    /// All deck names.
    pub fn deck_names(&self) -> Result<Vec<String>> {
        let result = self.invoke("deckNames", Value::Null)?;
        Ok(serde_json::from_value(result)?)
    }

    /// Start reviewing the given deck in Anki's GUI reviewer.
    pub fn gui_deck_review(&self, deck: &str) -> Result<bool> {
        let result = self.invoke("guiDeckReview", json!({ "name": deck }))?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// The card currently displayed by the reviewer, or `None` when the deck is done.
    pub fn gui_current_card(&self) -> Result<Option<CurrentCard>> {
        let result = self.invoke("guiCurrentCard", Value::Null)?;
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
