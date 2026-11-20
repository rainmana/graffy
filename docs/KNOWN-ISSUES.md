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

## Plain terminal: a line typed ahead can answer an approval gate

- **Seen**: 2026-11-19, in a macOS pseudo-terminal, on `main` and on the branch
  that fixed issue #35. Typing `first`, Enter, `y`, Enter before the gate's
  question appeared approved the gate on graph `conversation.gated`: the answer
  was released on a `y` typed before the payload was on screen.
- **Why**: a terminal holds finished lines until the program reads them, and the
  approval prompt takes the next line whenever it was typed. The shared stdin
  reader (`src/input.rs`) keeps lines in order and reads only on request; it does
  not know when a line was typed.
- **Reach**: seen in plain `graffy chat` at a terminal, on a graph with an
  approval gate. Plain `graffy run` uses the same prompt and was not tried. Piped
  chat never approves. `chat --tui` decides gates with Ctrl chords, not typed
  lines.
- **Fix when picked up**: discard pending terminal input (`tcflush`) just before
  the question is printed, or reject any line that arrived before it. Either
  needs terminal control the plain path does not have today.
- **Status**: recorded, not fixed. No test covers it.

## `graffy chat --tui`: not yet looked at on every terminal

- **What**: the view was driven in a Linux pseudo-terminal, has render tests at
  normal and tiny sizes, and has been used by @rainmana on macOS. It has not
  been checked on Windows Terminal or WSL. Treat layout and color there as
  unreviewed.
- **Composer**: the message box has a cursor (`←`/`→`, `Home`/`End`, `Delete`,
  `Ctrl-W`). `↑`/`↓` scroll the transcript rather than moving between lines of
  a multi-line draft, and there is no selection or undo.
