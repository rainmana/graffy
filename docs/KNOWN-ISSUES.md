# Known issues

Tracked observations that are real but deliberately not being chased yet.
Each entry says what was seen, what we suspect, and what would confirm it.

## TUI: overlapping text on some views (Windows Terminal / WSL)

- **Seen**: 2026-08-29, a couple of versions before v0.1.0-alpha.3, on
  Windows (WSL) — some TUI pages showed overlapping text. Not reproduced
  elsewhere yet.
- **Suspicion** (@rainmana): Windows' window management of WSL terminal
  sessions rather than graffy's rendering — resize/reflow events may not
  reach the TUI cleanly through that stack.
- **Confirm by**: reproducing on macOS/Linux native terminals. If it only
  happens under WSL, it's likely resize-event handling (ratatui needs a
  redraw on `Event::Resize`; worth checking we don't skip it) or Windows
  Terminal's cell-width handling of the glyphs we use (◐ ⊘ ⏸ etc.).
- **Cheap mitigations when we do pick it up**: handle `Event::Resize`
  explicitly with a full clear+redraw; offer the ASCII glyph fallback that
  the accessibility roadmap already plans.
- **Status**: noted, parked by request — do not dive in until confirmed on a
  second platform.
- **New candidate cause (2026-11-15)**: default `INFO` log lines were written
  to the same terminal the live view was drawing on (seen in a Linux
  pseudo-terminal: `node running node=draft …` lines over the TUI). Default
  logging is now off while a TUI owns the screen. **Not confirmed** as the WSL
  cause — retest on WSL before closing issue #15; the resize/glyph hypotheses
  above stay open until then.

## `graffy chat`: an abandoned approval prompt can swallow the next line

- **What**: the plain-terminal approval handler reads stdin from a detached
  thread. If a turn ends (deadline, Ctrl-C) while an approval prompt is still
  waiting, that thread is still blocked on stdin and will consume the next line
  typed — which the user meant as a chat message. The line is discarded; it is
  never treated as an approval and releases nothing.
- **Reach**: only graphs with an approval gate, and only when the turn ends
  before the human answers. `graffy run` is unaffected (the process exits).
- **Fix when it bites**: one owned stdin reader for the process, handing lines
  to whichever of chat/approval is waiting.
- **Status**: known ceiling, recorded at `graffy chat` introduction; not
  reproduced in a test.
