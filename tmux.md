# Running anki-tui in a floating tmux popup, toggled by Claude Code or Codex

This sets up `anki-tui` to live in a **floating tmux popup** that:

- runs in a **persistent, detached tmux session** — the same `anki-tui` instance
  survives being hidden/shown (it is never quit and restarted);
- **auto-shows while the agent is working** and **auto-hides when it needs you**
  (so you review cards while the agent thinks, and drop back to the chat when
  it's your turn) — wired for both **Claude Code** and the **Codex CLI**;
- can also be **opened manually** with a tmux key binding any time.

It works by keeping `anki-tui` in a tmux session named `anki`. "Showing" attaches
a popup to that session; "hiding" detaches the popup's client — the session, and
the `anki-tui` process inside it, keep running.

## 1. The toggle script

Save this as `~/.claude/anki-pane.sh` and `chmod +x` it.

```sh
#!/usr/bin/env bash
# Floating anki-tui popup for tmux, toggled by Claude Code hooks.
#
#   anki-pane.sh show      # float anki-tui over the current tmux client (async/hook use)
#   anki-pane.sh hide      # close the popup (anki-tui keeps running)
#   anki-pane.sh attach    # foreground attach, for a manual key binding
#   anki-pane.sh is-open   # exit 0 if the popup is currently up (for toggling)
set -uo pipefail

# Make sure tmux/anki-tui are found even when invoked from a bare tmux binding.
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

SESSION="anki"
START_CMD="${ANKI_TUI_CMD:-anki-tui}"
WIDTH="${ANKI_PANE_WIDTH:-90%}"
HEIGHT="${ANKI_PANE_HEIGHT:-90%}"

ensure_session() {
  tmux has-session -t "=$SESSION" 2>/dev/null && return 0
  tmux new-session -d -s "$SESSION" "$START_CMD"
}

popup_open() {
  # A client is attached to the session only while the popup is up.
  [ -n "$(tmux list-clients -t "=$SESSION" -F '#{client_name}' 2>/dev/null)" ]
}

case "${1:-}" in
  show)
    [ -n "${TMUX:-}" ] || exit 0      # nothing to draw on outside tmux
    ensure_session
    popup_open && exit 0              # already visible; don't stack popups
    # Backgrounded so the (blocking) popup doesn't hold up the hook/turn.
    tmux display-popup -E -w "$WIDTH" -h "$HEIGHT" -T " anki-tui " \
      "tmux attach -t '=$SESSION'" >/dev/null 2>&1 &
    disown
    ;;
  hide)
    tmux detach-client -s "=$SESSION" 2>/dev/null || true
    ;;
  attach)
    # Foreground attach, for running inside a manually-opened display-popup
    # (e.g. a tmux key binding). Reuses the same persistent session.
    ensure_session
    exec tmux attach -t "=$SESSION"
    ;;
  is-open)
    # Exit 0 if the popup is currently up (a client is attached to the session).
    popup_open
    ;;
  *)
    echo "usage: ${0##*/} show|hide|attach|is-open" >&2
    exit 1
    ;;
esac
```

Tunables via environment: `ANKI_TUI_CMD` (default `anki-tui`), `ANKI_PANE_WIDTH`,
`ANKI_PANE_HEIGHT` (default `90%`).

## 2. The tmux key binding

Add to `~/.config/tmux/tmux.conf` (adjust the path if your tmux config lives
elsewhere). This binds `prefix C-a` to **toggle** the popup:

```tmux
# Toggle anki-tui (reuses the persistent session shared with Claude Code hooks).
# If the popup is already up, close it; otherwise open it. This also closes the
# popup when pressed from inside it, instead of stacking another one.
bind-key -N "Toggle anki-tui" C-a if-shell "~/.claude/anki-pane.sh is-open" \
  "detach-client -s =anki" \
  "display-popup -E -w 90% -h 90% -T ' anki-tui ' '~/.claude/anki-pane.sh attach'"
```

`if-shell` runs `is-open` first: if a popup client is attached it closes it
(`detach-client`), otherwise it opens the popup. Because the popup shares your
tmux config, pressing `prefix C-a` _inside_ the popup hits the same binding and
closes it — so it toggles instead of nesting another popup. Reload the config
with `prefix r` (or `tmux source-file ~/.config/tmux/tmux.conf`) — otherwise the
binding won't exist yet and the key will appear to do nothing.

Then press **`prefix C-a`** (e.g. `Ctrl-b` then `Ctrl-a`) to float anki-tui, and
again to dismiss it. `anki-tui` keeps running either way, so the next open
resumes the same instance. (`Ctrl-b d` / `Escape` inside the popup also closes
it.)

## 3. The Claude Code hooks

These make the popup follow Claude's attention. Add to the project's
`.claude/settings.json` (or `~/.claude/settings.json` to apply everywhere — but
then it fires in every project, not just this one):

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "~/.claude/anki-pane.sh show",
            "async": true
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          { "type": "command", "command": "~/.claude/anki-pane.sh hide" }
        ]
      }
    ],
    "Notification": [
      {
        "hooks": [
          { "type": "command", "command": "~/.claude/anki-pane.sh hide" }
        ]
      }
    ]
  }
}
```

| Event              | Action           | Meaning                                                          |
| ------------------ | ---------------- | ---------------------------------------------------------------- |
| `UserPromptSubmit` | `show` (`async`) | you send a prompt → Claude starts working → anki-tui floats up   |
| `Stop`             | `hide`           | Claude finishes and hands control back → popup closes            |
| `Notification`     | `hide`           | Claude needs input mid-turn (permission/question) → popup closes |

`show` is marked `async: true` so the blocking popup never stalls Claude's turn.

> **Reload caveat:** Claude Code's settings watcher only watches `.claude/`
> directories that already had a settings file when the session started. If you
> just created `.claude/settings.json`, the hooks won't fire until you open
> `/hooks` once (which reloads config) or restart Claude Code.

## 4. The Codex CLI hooks

Codex (the OpenAI `codex` CLI) has its own hooks system that drives the same
script. Codex auto-discovers a `hooks.json` in each config folder — `~/.codex/`
(global, every project) or `<project>/.codex/` (that repo only) — so no
`config.toml` pointer is needed. To make the popup follow Codex's attention
everywhere, save this as `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/Users/vincent/.claude/anki-pane.sh show",
            "async": true
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/Users/vincent/.claude/anki-pane.sh hide"
          }
        ]
      }
    ],
    "PermissionRequest": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/Users/vincent/.claude/anki-pane.sh hide"
          }
        ]
      }
    ]
  }
}
```

| Event               | Action           | Meaning                                                       |
| ------------------- | ---------------- | ------------------------------------------------------------- |
| `UserPromptSubmit`  | `show` (`async`) | you send a prompt → Codex starts working → anki-tui floats up |
| `Stop`              | `hide`           | Codex finishes and hands control back → popup closes          |
| `PermissionRequest` | `hide`           | Codex asks to approve a command mid-turn → popup closes       |

Differences from the Claude config above:

- **Event keys are the same CamelCase names**, but the file is `hooks.json` (not
  `settings.json`) and the hook entries live under a top-level `"hooks"` object.
- `PermissionRequest` is Codex's analog of Claude's `Notification` — it fires
  when Codex needs you to approve something, so hiding there keeps the popup from
  covering the approval prompt.
- Unlike Claude, Codex's command must be an **absolute path** (it does not expand
  `~`), hence `/Users/vincent/.claude/anki-pane.sh`.

> **Trust caveat:** Codex gates newly added hooks behind a **trust prompt** — they
> won't run until you approve them on the next `codex` launch. To confirm the
> wiring without the prompt, launch once with
> `codex --dangerously-bypass-hook-trust`.

## How show/hide stays the same instance

- `anki-tui` runs in the detached `anki` session, independent of any popup.
- **Show** opens a `display-popup` whose command is `tmux attach -t =anki` — it
  draws the live session.
- **Hide** runs `tmux detach-client -s =anki`, which closes the popup's client.
  The session is untouched, so `anki-tui` keeps running where you left it.
- The manual binding and the hooks both target the same `anki` session, so they
  never fight or stack popups: the `prefix C-a` binding toggles via `is-open`,
  and the `show` hook no-ops when a popup is already up.

## Notes

- **Requires Anki running** with the AnkiConnect add-on on
  `http://127.0.0.1:8765`. If it isn't up, `anki-tui` exits immediately and you
  get a brief empty popup.
- **The popup grabs input focus** while open — that's intentional: you review
  cards there while Claude works. Close it (`Ctrl-b d` / `Escape`) to peek back
  at Claude before it's done; it reappears on your next prompt.
- **Quitting anki-tui with `q`** ends the session, but that's recoverable: the
  next `show`/`attach` runs `ensure_session` and starts a fresh instance.
