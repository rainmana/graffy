# ADR-0010: Conversation interface over graph execution

- Status: **Accepted product direction; implementation proposed**
- Date: 2026-10-17
- Decider: W. Alec Akin (@rainmana)
- Related: ADR-0002, ADR-0004, ADR-0005; GitHub issue #17

## Decision

After bug fixes and MCW integration work, graffy will offer a chat-like interface
in **Ratatui and CLI**. Every submitted prompt still executes through a compiled,
budgeted, journaled graph. The user sees a conversation by default and can inspect
the graph's recorded activity in an optional reasoning layer. Graph visualization
is a nice-to-have, not a prerequisite for chat.

Desktop and web interfaces are **tabled indefinitely** by explicit owner direction.
This reaffirms ADR-0002; it does not introduce a browser server, desktop shell, or
second rendering stack.

The reasoning layer presents observable execution: chosen graph, node progress,
tools, evidence references, review outcomes, approval decisions, retries, and
short explanations. It must distinguish recorded events from presentation
summaries and must not fabricate a model's hidden reasoning or present a graph
trace as proof that an answer is correct.

## Why the current architecture fits

The runtime already has a conversation floor, session IDs, attempt groups,
external boundary events, a history hydration path, and a journal event tap.
The TUI already folds the tap into an inspector. A chat surface can reuse these
without adding a direct provider-call path.

Today, however, `src/main.rs::run_graph` owns retries, history loading, indexing,
and printing, while `graffy_tui::run_live` constructs a separate one-shot execution.
The TUI does not accept session continuation or retries. Copying the CLI loop
into a composer would allow these paths to drift further.

## Proposed first implementation

The details below are a development plan, not delivered functionality.

1. Extract a shared application/session runner for graph selection, history,
   attempts, approvals, indexing, cancellation, and completion. It emits events
   and structured outcomes rather than printing. Keep dependencies outside core's
   engine traits; core must not depend on providers, storage, or the TUI. Introduce
   an application crate only if sharing requires it.
2. Add a CLI chat loop with plain-text parity, initially using the default
   conversation graph. Proposed surface: `graffy chat`, with optional graph pin,
   session continuation, offline mode, and retry configuration. Final flag spelling
   can follow the existing command conventions.
3. Add a Ratatui conversation view with a composer, scrollable transcript, one
   active turn at a time, visible progress, inline approval requests, and
   new/resume-session actions. Preserve access to existing journal browsing.
4. Show the accepted answer only after the graph's release decision. Keep drafts,
   reviewer output, and automatic retry feedback in the inspection surface.
   Failures and cancellations have visible statuses and do not masquerade as
   assistant answers. Record what was actually delivered at the UI/CLI boundary.
5. Start with event-level progress. Current providers return whole completions;
   token streaming is a separate capability. If added, unverified drafts must not
   leak into the ordinary answer stream.
6. Add an optional “Reasoning” or “Run details” view using existing journal data.
   A compact step list is sufficient initially. Opening it must never change
   execution, trigger model calls, or alter the canonical conversation.

```text
Ratatui composer / CLI chat loop
                 |
         shared session runner
         | history + graph choice
         | one attempt group per human turn
                 |
          compiler + executor
          intake -> ground -> draft -> verify -> respond
                               ^          |
                               +--revise--+
                 |
         journal events + outcome
          /                  \
 visible conversation     optional run details
```

## Identity and MCW requirements

- One session spans the conversation; each new external request creates a fresh
  attempt group; each automatic attempt gets its own run ID and journal.
- Retry prompts and internal critiques do not become extra human turns.
- Explicit user corrections, clarifications, approvals, and rejections retain
  their boundary roles. Distinguish a deliberate rejection from EOF/disconnection.
- History comes from visible boundary turns, with stable source references.
  Missing/corrupt history is reported, not silently treated as an empty session.
- Resuming a conversation means restoring its transcript and starting a new turn;
  resuming a paused executor mid-node is a separate durability feature.
- Long-session context selection must preserve provenance and disclose omissions.
  Compaction, when implemented, runs as a graph and is not an implicit UI summary.
- Default chat should not display unvalidated H/R/D/M scores as a health gauge.
  Human rating remains a separate, explicitly labeled calibration workflow.

## Later routing and visualization

Issue #17 captures the owner's original prompt-first routing design. After the
fixed-floor chat works, a lawful journaled router can select an eligible registered
graph. Explicit graph pins win; uncertain selection falls back to conversation;
explicit-only graphs cannot be selected automatically. Explain the chosen graph
briefly and allow the user to override it.

On-the-fly graph generation is a separate increment: proposed TOML must compile,
remain inspectable, and obey approval/adoption rules. Do not make the first chat
release depend on model-assisted decomposition, vector memory, subgraph support,
or an animated graph layout.

## Acceptance journeys

- A new user opens chat, sends a prompt, receives a released answer, and asks a
  dependent follow-up. The same session carries the necessary history.
- Two identical prompts sent deliberately are two external turns; an automatic
  retry of one prompt remains inside that turn's attempt group.
- A rejected draft causes visible progress and bounded repair, then either a
  released answer or an honest failure. It never becomes an extra chat answer.
- An approval shows the exact proposed action. Reject, edit, EOF, quit, and later
  approval requests have defined behavior and cannot silently release work or hang.
- Cancellation records an outcome and stops further scheduling; tool side effects
  already performed are disclosed and are not claimed to have been rolled back.
- Restart and resume reconstruct the conversation from recorded boundary events.
  Opening run details or replaying a journal never re-executes tools.
- CLI and Ratatui use the same runner and preserve equivalent execution/boundary
  records. Plain output, keyboard navigation, small terminal sizes, and resize are
  checked before release.

Use deterministic invokers for these acceptance journeys through the real runtime.
Then validate live-provider behavior and terminal presentation separately; passing
offline orchestration tests does not establish model quality or visual usability.
