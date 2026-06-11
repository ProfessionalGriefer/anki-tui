# anki-tui

A fast, keyboard-driven terminal UI for reviewing your Anki cards, written in
Rust with [ratatui](https://ratatui.rs). It talks to your _running_ Anki desktop
instance over the [AnkiConnect](https://foosoft.net/projects/anki-connect/)
add-on, and renders card **images** and plays card **audio** directly in the
terminal.

This is intentionally minimal: it is a **reviewer**, not a card manager. There is
no card creation, no browser, and no statistics — just decks and reviews.

## Features

- **Deck list** — see all your decks and pick one to study.
- **Review screen** — go through a deck's due cards using Anki's own scheduler
  (learning steps, intervals, and ease are all handled by Anki itself via
  AnkiConnect's GUI reviewer actions, so your reviews count exactly as if you
  did them in the app).
- **Images** — `<img>` tags in cards are rendered inline using the
  [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/),
  which **Ghostty** supports natively. Falls back to Sixel / iTerm2 / half-blocks
  on other terminals (handled automatically by
  [`ratatui-image`](https://crates.io/crates/ratatui-image)).
- **Audio** — `[sound:...]` references are fetched from Anki's media collection
  and played through the system audio player (`afplay` on macOS).
- **Vim-style keybindings** (see below).

## Requirements

- **Rust** (edition 2024 toolchain; `cargo`).
- **Anki desktop** running, with the **AnkiConnect** add-on installed
  (add-on code `2055492159`). By default AnkiConnect listens on
  `http://127.0.0.1:8765`.
- A terminal with image-passthrough support for inline images. Developed and
  tested against **Ghostty** (Kitty graphics protocol). Other supported
  protocols: Sixel, iTerm2. Without any of these you still get a half-block
  approximation.
- **macOS** for audio playback out of the box (uses the built-in `afplay`).
  See [Configuration](#configuration) to change the player on other platforms.

## Installation

```sh
git clone <this-repo>
cd anki-tui
cargo build --release
```

The binary is then at `target/release/anki-tui`.

## Usage

1. Start Anki (with AnkiConnect installed) and leave it running.
2. Run the TUI:

   ```sh
   cargo run --release
   ```

3. Pick a deck with `j`/`k`, press `l`/`Enter` to start reviewing.
4. Read the front, press `Space` to reveal the answer, then grade with `1`–`4`.

> [!NOTE]
> Because reviews are driven through Anki's GUI reviewer, the deck you select
> becomes the active deck in the Anki main window. Keep the Anki window on the
> deck-list/reviewer screen for best results.

## Keybindings

These apply globally unless noted.

| Key           | Action                                                 |
| ------------- | ------------------------------------------------------ |
| `j`           | Move down (next deck in deck view / scroll card down)  |
| `k`           | Move up (previous deck in deck view / scroll card up)  |
| `l` / `Enter` | Open the selected deck and start reviewing (deck view) |
| `Space`       | Show the answer (review view)                          |
| `1`           | Grade **Again** (review view, answer shown)            |
| `2`           | Grade **Hard**                                         |
| `3`           | Grade **Good**                                         |
| `4`           | Grade **Easy**                                         |
| `r`           | Replay the current card's audio (review view)          |
| `d`           | Go back to the deck list                               |
| `q`           | Quit                                                   |

> Cards can expose fewer than four grading buttons (e.g. a card in learning may
> only offer Again/Good). Pressing a number with no corresponding button is a
> no-op; the available grades and their next-interval previews are shown on the
> answer screen.

## How it works

`anki-tui` is a thin client over AnkiConnect (API version 6). The relevant
actions used:

- `deckNames` — populate the deck list.
- `guiDeckReview` — start reviewing a chosen deck using Anki's scheduler.
- `guiCurrentCard` — fetch the current card's question/answer HTML, `cardId`,
  available `buttons` (ease values) and `nextReviews` previews.
- `guiShowAnswer` — flip to the answer.
- `guiAnswerCard` — submit a grade (ease `1`–`4`).
- `retrieveMediaFile` — pull image/audio bytes (base64) out of Anki's media
  folder for rendering/playback.

Card HTML is scanned for `<img src="...">` and `[sound:...]` references; the
referenced media files are fetched via `retrieveMediaFile`, written to a temp
directory, then rendered (images) or played (audio).

## Configuration

Environment variables:

| Variable             | Default                 | Purpose                                                                              |
| -------------------- | ----------------------- | ------------------------------------------------------------------------------------ |
| `ANKI_CONNECT_URL`   | `http://127.0.0.1:8765` | AnkiConnect endpoint.                                                                |
| `ANKI_TUI_AUDIO_CMD` | `afplay` (macOS)        | Command used to play an audio file; the file path is appended as the final argument. |

## Project layout

```
src/
  main.rs    # terminal setup + event loop
  anki.rs    # AnkiConnect HTTP client and response types
  media.rs   # media extraction, image protocol building, audio playback
  app.rs     # application state machine (deck list / review)
  ui.rs      # ratatui rendering
```

## Limitations

- Requires Anki to be open; this tool does not read your collection directly.
- HTML rendering is text-only (tags are stripped) apart from images — complex
  card templates with heavy styling will look plain.
- Audio playback shells out to an external player; non-macOS users must set
  `ANKI_TUI_AUDIO_CMD`.

## License

MIT
