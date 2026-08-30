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
