# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`anki-tui` is a keyboard-driven terminal reviewer for Anki, written in Rust with
[ratatui](https://ratatui.rs). It is a _reviewer only_ (no card creation, browser,
or stats). It does not read the Anki collection directly — it is a thin blocking
HTTP client over the **AnkiConnect** add-on (API version 6) talking to a _running_
Anki desktop instance. Reviews go through Anki's own GUI reviewer, so scheduling
is entirely Anki's.

## Commands

```sh
cargo run                  # run against a running Anki (debug)
cargo run --release        # run (release; smoother image encoding)
cargo build --release      # binary at target/release/anki-tui
cargo test                 # unit tests (live in src/media.rs)
cargo test extracts_image  # run a single test by name
cargo clippy
```

Running the app requires Anki open with AnkiConnect (add-on `2055492159`) listening
on `http://127.0.0.1:8765`. Note the toolchain is **edition 2024**, which uses
let-chains (`if let ... && ...`) — keep that idiom when editing.

## Architecture

Single-binary event loop. `main.rs` owns the loop: `terminal.draw` → `event::poll`
(250 ms timeout, which also drives image re-encoding) → `handle_key`. All key
dispatch lives in `main.rs::handle_key`, branching on `app.screen` (and the
`searching` sub-mode); the matched key calls a method on `App`. Keep new
keybindings in that match and the behavior in an `App` method — don't scatter logic.

Module responsibilities:

- **`app.rs`** — the `App` state machine and all business logic. Two screens
  (`Screen::DeckList` / `Screen::Review`). Owns deck list + fold/search state and
  review state. This is where most changes go.
- **`anki.rs`** — `AnkiConnect` client. Every call goes through `invoke(action, params)`
  which wraps the `{result, error}` envelope. One method per AnkiConnect action.
- **`media.rs`** — `SideMedia`: per-card-side media. Hand-rolled scanners pull
  `<img src=...>` and `[sound:...]` out of card HTML, fetch bytes via
  `retrieveMediaFile`, build `ratatui-image` protocols (images) / temp files (audio).
  `strip_media_tokens` removes those tokens before `html2text` renders the rest.
  Unit tests live here.
- **`ui.rs`** — pure ratatui rendering from `&App`; no state mutation.

### Things that aren't obvious from one file

- **Review is GUI-driven, with one exception.** Normal grading uses
  `guiDeckReview` / `guiCurrentCard` / `guiShowAnswer` / `guiAnswerCard`. The
  catch: `guiUndo` reverts the collection but does _not_ move Anki's reviewer back
  to the undone card. So `undo()` restores the previously graded card from
  `prev_card` into an `undone` mode; while `undone` is true, the card is shown/graded
  _locally_ via `answerCards` (by card id) and the GUI reviewer (sitting on the next
  card) is left untouched. When touching review flow, account for the `undone` branch.
- **Deck tree.** Decks are a flat `Vec<DeckInfo>` sorted by lowercased name;
  hierarchy is derived from `::`-separated names. `has_children` = "is a strict
  prefix of another deck." `visible_decks()` filters by search query, else flat
  view, else hides decks under a collapsed ancestor. Selection indexes into the
  _visible_ list, so it must be re-clamped/re-found after any list change.
- **Persisted fold state.** Collapsed deck names are saved to
  `$XDG_STATE_HOME/anki-tui/collapsed.json` (fallback `~/.local/state/...`),
  best-effort. First launch defaults to collapsing every parent.
- **`Space`** reveals the answer, or grades Good (ease 3) once the answer is shown.
  Grading buttons available per card vary (`buttons` is `[1,3]`, `[1,2,3,4]`, etc.);
  pressing a missing grade is a no-op with a status message.

## Configuration (env vars)

- `ANKI_CONNECT_URL` — AnkiConnect endpoint (default `http://127.0.0.1:8765`).
- `ANKI_TUI_AUDIO_CMD` — audio player command, file path appended last
  (default `afplay`, macOS). Set this on non-macOS.
