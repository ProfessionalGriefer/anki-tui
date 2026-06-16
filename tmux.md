# Running anki-tui in a tmux split pane, toggled by Claude Code or Codex

This sets up `anki-tui` to live in a **tmux split pane** that:

- runs in a **persistent, detached tmux session** — the same `anki-tui` instance
  survives being hidden/shown (it is never quit and restarted);
- **auto-shows while the agent is working** and **auto-hides when it needs you**
  (so you review cards while the agent thinks, and drop back to the chat when
  it's your turn) — wired for both **Claude Code** and the **Codex CLI**;
- can also be **toggled manually** with a tmux key binding any time.

> **Why a split pane and not a `display-popup`?** A popup is modal — it grabs
> every keystroke, so typing meant for the agent leaks into `anki-tui` — and tmux
> does **not** route image escape sequences into popups, so card images never
> render. A real split pane only takes keys when focused and gets graphics
> passthrough, so inline card images work.

It works by parking the single `anki-tui` process in a detached holding session
named `anki` (next to a tiny keep-alive pane so the session never dies). The pane
is tagged with the `@anki` user option so the script can find it wherever it
currently lives. "Showing" `join-pane`s it into your current window; "hiding"
`join-pane`s it back into the holding session — the process keeps running either
way.

## 1. The toggle script

Save this as `~/.claude/anki-pane.sh` and `chmod +x` it.

```sh
#!/usr/bin/env bash
# Split-pane anki-tui for tmux, toggled by Claude Code hooks.
#
#   anki-pane.sh show   # split anki-tui into the current window
#   anki-pane.sh hide   # park it back out of view (anki-tui keeps running)
#
# Why a split pane and not a display-popup: a popup is modal (it grabs every
# keystroke, so typing meant for Claude leaks into anki-tui) and tmux does not
# route image escape sequences into popups (so card images never render). A real
# pane only takes keys when focused and gets graphics passthrough.
#
# The single anki-tui process is never killed. When hidden it lives in a
# detached holding session ("anki") next to a tiny keep-alive pane (so the
# session never dies); showing/hiding just moves that one pane in and out of the
# current window with join-pane. The pane is tagged with the @anki user option so
# we can find it regardless of where it currently lives.
set -uo pipefail

# Make sure tmux/anki-tui are found even when invoked from a bare tmux binding.
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

SESSION="anki"
# `exec` so the pane's shell replaces itself with anki-tui instead of running it
# as a child. Otherwise (e.g. with nushell as default-shell) tmux runs
# `nu -c anki-tui`, nushell stays in the foreground process group, and it echoes
# j/k keystrokes instead of letting anki-tui's raw-mode reader consume them.
START_CMD="${ANKI_TUI_CMD:-exec anki-tui}"
SIZE="${ANKI_PANE_WIDTH:-40%}"   # width of the split when shown

# Print "pane_id|window_id|session_name" of the tagged anki-tui pane, if it
# exists anywhere on the server. Empty if it doesn't exist yet.
anki_pane() {
  tmux list-panes -a -F '#{@anki}|#{pane_id}|#{window_id}|#{session_name}' 2>/dev/null \
    | awk -F'|' '$1=="1"{print $2"|"$3"|"$4; exit}'
}

ensure_holding() {
  tmux has-session -t "=$SESSION" 2>/dev/null && return 0
  # Keep-alive pane keeps the holding session alive while anki-tui is shown.
  tmux new-session -d -s "$SESSION" "exec sleep 2147483647"
}

# Create the anki-tui pane (tagged, detached in the holding session) if missing.
ensure_pane() {
  [ -n "$(anki_pane)" ] && return 0
  ensure_holding
  local id
  id=$(tmux split-window -d -t "=$SESSION:" -P -F '#{pane_id}' "$START_CMD") || return 1
  tmux set -p -t "$id" @anki 1
}

case "${1:-}" in
  show)
    [ -n "${TMUX:-}" ] || exit 0          # nothing to draw on outside tmux
    ensure_pane || exit 0
    info=$(anki_pane); [ -n "$info" ] || exit 0
    pid=${info%%|*}; rest=${info#*|}; pwin=${rest%%|*}
    cur_win=$(tmux display -p -t "${TMUX_PANE:-}" '#{window_id}' 2>/dev/null)
    [ "$pwin" = "$cur_win" ] && exit 0    # already visible in this window
    # Split it into the current window to the right and focus it, so you can
    # study (j/k, grading) immediately while the agent works. Parking it on
    # `hide` returns focus to the agent's pane automatically.
    tmux join-pane -h -l "$SIZE" -s "$pid" -t "${TMUX_PANE:-}" 2>/dev/null || exit 0
    ;;
  hide)
    info=$(anki_pane); [ -n "$info" ] || exit 0
    pid=${info%%|*}; psess=${info##*|}
    [ "$psess" = "$SESSION" ] && exit 0   # already parked
    ensure_holding
    tmux join-pane -d -s "$pid" -t "=$SESSION:" 2>/dev/null || true
    ;;
  attach)
    # Foreground attach to the holding session, for manual debugging.
    ensure_pane
    exec tmux attach -t "=$SESSION"
    ;;
  is-open)
    # Exit 0 if anki-tui is currently visible (joined into a non-holding window).
    info=$(anki_pane); [ -n "$info" ] || exit 1
    psess=${info##*|}
    [ "$psess" != "$SESSION" ]
    ;;
  *)
    echo "usage: ${0##*/} show|hide|attach|is-open" >&2
    exit 1
    ;;
esac
```

Tunables via environment: `ANKI_TUI_CMD` (default `exec anki-tui`) and
`ANKI_PANE_WIDTH` (default `40%`, the width of the split when shown).

## 2. The tmux key binding

Add to `~/.config/tmux/tmux.conf` (adjust the path if your tmux config lives
elsewhere). This binds `prefix C-a` to **toggle** the split pane:

```tmux
# Toggle anki-tui split pane (reuses the persistent session shared with Claude
# Code hooks). If it's visible, park it; otherwise split it into this window.
bind-key -N "Toggle anki-tui" C-a if-shell "~/.claude/anki-pane.sh is-open" \
  "run-shell '~/.claude/anki-pane.sh hide'" \
  "run-shell '~/.claude/anki-pane.sh show'"
```

`if-shell` runs `is-open` first: if the pane is currently joined into a window it
parks it (`hide`), otherwise it splits it in (`show`). Delegating both branches to
the script keeps the binding and the hooks using the exact same join/park logic,
so they never fight or duplicate the pane. Reload the config with `prefix r` (or
`tmux source-file ~/.config/tmux/tmux.conf`) — otherwise the binding won't exist
yet and the key will appear to do nothing.

Then press **`prefix C-a`** (e.g. `Ctrl-b` then `Ctrl-a`) to split anki-tui into
the current window, and again to park it. `anki-tui` keeps running either way, so
the next show resumes the same instance.

## 3. The Claude Code hooks

These make the pane follow Claude's attention. Add to the project's
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
    "PreToolUse": [
      {
        "hooks": [
          { "type": "command", "command": "~/.claude/anki-pane.sh hide" }
        ]
      }
    ],
    "PostToolUse": [
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

| Event              | Action           | Meaning                                                              |
| ------------------ | ---------------- | -------------------------------------------------------------------- |
| `UserPromptSubmit` | `show` (`async`) | you send a prompt → Claude starts working → anki-tui splits in       |
| `PreToolUse`       | `hide`           | a tool is about to run (and before any approval prompt) → pane parks |
| `PostToolUse`      | `show` (`async`) | the tool finished → pane splits back in so you keep reviewing        |
| `Stop`             | `hide`           | Claude finishes and hands control back → pane parks                  |
| `Notification`     | `hide`           | Claude needs input mid-turn (permission/question) → pane parks       |

`show` is marked `async: true` so the blocking `join-pane` never stalls Claude's
turn.

**Why the `PreToolUse`/`PostToolUse` pair?** Claude Code has no dedicated
permission-request hook, and `PreToolUse` fires _before_ the approval prompt (it
is the gate). So `PreToolUse`→`hide` parks the pane before any prompt renders,
and `PostToolUse`→`show` only fires after the tool actually runs — i.e. after you
approve it. The tradeoff is a brief hide/show flicker around each tool execution,
and that denying a tool skips `PostToolUse`, so the pane stays parked until your
next prompt or `Stop`.

> **Reload caveat:** Claude Code's settings watcher only watches `.claude/`
> directories that already had a settings file when the session started. If you
> just created `.claude/settings.json`, the hooks won't fire until you open
> `/hooks` once (which reloads config) or restart Claude Code.

## 4. The Codex CLI hooks

Codex (the OpenAI `codex` CLI) has its own hooks system that drives the same
script. Codex auto-discovers a `hooks.json` in each config folder — `~/.codex/`
(global, every project) or `<project>/.codex/` (that repo only) — so no
`config.toml` pointer is needed. To make the pane follow Codex's attention
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
    "PreToolUse": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/Users/vincent/.claude/anki-pane.sh hide"
          }
        ]
      }
    ],
    "PostToolUse": [
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

| Event               | Action           | Meaning                                                              |
| ------------------- | ---------------- | -------------------------------------------------------------------- |
| `UserPromptSubmit`  | `show` (`async`) | you send a prompt → Codex starts working → anki-tui splits in        |
| `PreToolUse`        | `hide`           | a tool is about to run (and before any approval prompt) → pane parks |
| `PostToolUse`       | `show` (`async`) | the tool finished → pane splits back in so you keep reviewing        |
| `Stop`              | `hide`           | Codex finishes and hands control back → pane parks                   |
| `PermissionRequest` | `hide`           | Codex asks to approve a command mid-turn → pane parks                |

Codex 0.139 supports the same hook events as Claude Code — `SessionStart`,
`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`,
`PreCompact`, `Notification`, and `Stop` — so the wiring mirrors §3 exactly.

Differences from the Claude config above:

- **Event keys are the same CamelCase names**, but the file is `hooks.json` (not
  `settings.json`) and the hook entries live under a top-level `"hooks"` object.
- `PostToolUse`→`show` is the piece that re-splits the pane in after a tool runs
  without needing you. Without it, a tool that triggers a `PermissionRequest`
  parks the pane and nothing reopens it until your next prompt.
- `PermissionRequest` is an extra Codex hook (Claude approvals go through
  `PreToolUse` instead) — it fires when Codex needs you to approve something, so
  parking there keeps the pane from covering the approval prompt.
- Unlike Claude, Codex's command must be an **absolute path** (it does not expand
  `~`), hence `/Users/vincent/.claude/anki-pane.sh`.

> **Trust caveat:** Codex gates newly added hooks behind a **trust prompt** — they
> won't run until you approve them on the next `codex` launch. To confirm the
> wiring without the prompt, launch once with
> `codex --dangerously-bypass-hook-trust`.

## How show/hide stays the same instance

- The single `anki-tui` pane is **tagged with the `@anki` user option**, so the
  script finds it with `list-panes -a` no matter which session it currently lives
  in. It is never killed.
- **Show** `join-pane`s that tagged pane into your current window (a `40%` split
  to the right) and focuses it. If it's already visible in this window, `show`
  no-ops.
- **Hide** `join-pane`s it back into the detached `anki` holding session, which
  also returns focus to the agent's pane. If it's already parked there, `hide`
  no-ops.
- The holding session is kept alive by a tiny `sleep` keep-alive pane, so parking
  anki-tui there never lets the session (and the process) die.
- The manual binding and the hooks both call the same `show`/`hide`, so they
  never fight or duplicate the pane.

## Notes

- **Requires Anki running** with the AnkiConnect add-on on
  `http://127.0.0.1:8765`. If it isn't up, `anki-tui` exits immediately and the
  split pane closes itself.
- **The split pane only grabs input when focused.** `show` focuses it so you can
  review immediately while the agent works; switch focus back to the agent's pane
  (e.g. `prefix` + arrow / your select-pane binding) any time without parking it.
- **Inline card images work** because a split pane — unlike a `display-popup` —
  gets tmux's graphics passthrough.
- **Quitting anki-tui with `q`** ends the pane, but that's recoverable: the next
  `show`/`attach` runs `ensure_pane` and starts a fresh instance.
