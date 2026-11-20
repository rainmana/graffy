# ADR-0014: Context policy for continued sessions (stop-gap)

- Status: **Implemented as a stop-gap (2026-11-19); to be superseded by a compaction graph**
- Date: 2026-11-19
- Decider: W. Alec Akin (@rainmana) set the rule in issue #30. The specific
  choices under "Choices made while implementing" were made by the implementing
  agent and are his to change.
- Related: GitHub issue #30, ADR-0010 (chat through graphs), ADR-0011 (token
  reserve), `docs/design/phase-3-learning.md` (compaction)

## Problem

Every continued turn hydrates the session's whole visible boundary history and
each `model` node puts all of it in front of the request. Nothing bounded it.
A long chat ended in one of two ways, both without guidance:

- the provider rejected a request larger than the model's context window; or
- the adapter's input reserve (one token per UTF-8 byte, ADR-0011) left no
  output allowance and the run stopped as budget-exhausted.

## The rule (owner, issue #30)

Before a turn, either warn and stop, or apply a **disclosed** bounded selection.
Never silently drop earlier constraints. IU-aware compaction comes later as its
own graph with its own evidence.

## What is implemented

`graffy_core::context` holds the policy as pure functions. `src/turn.rs` applies
it through `TurnHistory::select`, the only way a frontend can give history to
the shared turn runner, so `graffy run --session`, `graffy chat` and
`graffy chat --tui` cannot differ.

1. **Measure.** The request plus the hydrated history is measured exactly as a
   model node assembles it (`context::assemble` builds that text for both), in
   UTF-8 bytes.
2. **Fits:** the history is carried whole. Nothing changes and nothing is said.
3. **Does not fit:** a bounded selection is made — the **opening human turn**,
   then as many of the **most recent** turns as fit, newest first, stopping at
   the first that does not fit so the recent window has no hidden gap.
4. **Disclose, three ways.**
   - To the model: a line labelled `graffy: [context policy] …` stands exactly
     where the turns were left out. It gives the count and size and tells the
     model not to guess what they said.
   - To the journal: that line is hydrated like any other history entry, so it
     is a recorded IU (`role = boundary-history`, `actor = graffy`). The
     artifact shows what the model was and was not given.
   - To the human: the frontend shows a `context policy:` notice for that turn
     (stderr `warning:` in the plain frontends, a notice line in the chat TUI)
     with the counts, the limit and where the limit came from.
5. **Cannot fit even the request, the opening turn and the notice:** the turn
   is not sent. No run starts, no model is called, and the message says what to
   do: shorten the request, start a new chat, or raise the limit.

A new conversation (no history) is never measured here. An oversized single
request is the token budget's business and already fails loudly.

## Choices made while implementing

**Bytes, not tokens.** graffy has no tokenizer and does not know any model's
vocabulary. A token is at least one byte, so bytes are an upper bound on
tokens, and it is the unit the adapter already reserves input with. Reporting
"tokens" here would be an invented number.

**Where the limit comes from.**

- `GRAFFY_CONTEXT_MAX_BYTES`, when the human sets it. It wins in both
  directions. graffy does not know the context window of the model a tier is
  bound to, so this is the human's knob for it. A value that is not a positive
  whole number stops `chat` and `run` before anything is sent.
- Otherwise, from the graph's own token budget: `policy.budget.max_tokens`
  divided by the number of `model` and `verify` nodes. Each of those nodes
  dispatches the history once on a clean pass, and the adapter reserves one
  token per byte each time, so under that same accounting this is the most the
  history can take. For the built-in conversation graph that is
  200,000 ÷ 2 = 100,000 bytes.
- A graph with no token budget, or no model-calling node, implies no limit.
  graffy does not invent one; the history is carried whole, as before.

The derived limit is a planning share, **not a guarantee**. System text,
outputs and revise loops also spend the budget, and a real tokenizer usually
needs far fewer tokens than bytes. Running out is still a loud
`BudgetExhausted`. It is conservative on purpose: it can trim a session that
would have fit a real tokenizer's count. The human's override is the way out.

**Which turns are kept.** graffy does not read wording to decide which turns
hold constraints (the same rule as corrections: an explicit declaration, never
inferred). The opening human turn is kept because it is where a conversation's
task and standing constraints are usually stated; the newest turns are kept
because follow-ups refer to them. Everything else is omitted **and counted**.

## MCW reading

Trimming context is compression of the shared record. Done silently it is
Overcompression that neither party can see, which is Constraint Opacity: the
human believes a constraint is still in force and the model no longer has it.
The disclosure does not restore the omitted turns. It makes the loss visible on
both sides of the boundary so the human can repair it by restating — the
omission notice is an invitation to repair, not a repair.

## Limits of this stop-gap (honest list)

- **A constraint stated in the middle of a long session can be omitted.** It is
  disclosed, never silent, but it is gone from that turn's context.
- **Explicit `/correct` turns are not pinned.** The hydration payload carries
  actor and text, not the boundary role, so the selection cannot tell a
  correction from any other human turn. Pinning them is the obvious next
  increment.
- One limit for the whole turn, not per tier. A graph that escalates to a tier
  bound to a smaller model can still overflow that model.
- The measure covers the request and the history. A node's system text,
  context passed from earlier nodes and review feedback are not counted.
- Nothing is summarized. That is deliberate: a summary is model output and
  would need its own graph, evidence and review.

## Superseded by

An IU-aware compaction graph that preserves IUs and their falsification
conditions (ROADMAP, Phase 3). When it lands, this selection becomes its
fallback or goes away; the disclosure rule stays.
