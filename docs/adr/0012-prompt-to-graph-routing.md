# ADR-0012: Prompt-to-graph routing

- Status: **Accepted direction; implementation not started**
- Date: 2026-11-15
- Decider: W. Alec Akin (@rainmana)
- Related: ADR-0010 ("Later routing"), GitHub issue #17

Nothing in this ADR is shipped. It records decisions the owner made on
2026-11-15 so they do not evaporate, plus verified facts about two external
models he proposed. Work starts after he has tried `graffy chat --tui` and
PR #20 is merged; routing goes in its own PR.

## Decisions (owner)

1. **The router is its own lawful graph run.** A routed turn first runs a small
   built-in router graph with its own journal, budget and events. It is not an
   event tucked into the target run.
2. **Graphs opt in by declaring trigger phrases.** A graph is eligible for
   automatic selection only if its TOML declares phrases (working name
   `route_on`). No phrases means explicit-only. This is deliberately more
   conservative than issue #17's "everything unless marked explicit-only":
   an imported tool-using graph can never be picked by accident.
3. **Deterministic routing is the default; a decision model is an option**
   (owner's idea): besides the phrase matcher, routing may be delegated to a
   small "decision model" — hosted **Jev** (TypeSafe AI) or a local
   **Strands Decider** — chosen by configuration.
4. **Order:** owner tries the TUI chat first; routing follows.

Already fixed by ADR-0010 and issue #17: an explicit graph pin always wins;
uncertain selection falls back to the conversation floor; the choice is
explained in one plain line with a way to override it.

## Deterministic router (default)

A graph matches when the prompt contains one of its declared phrases
(case-insensitive, whole words). **Exactly one** matching graph is selected.
Zero matches, or matches on more than one graph, fall back to conversation.
There is no score and no threshold, so there is nothing to tune or to
misreport, and the explanation is exact: *"running Decision Framework because
your message says 'pros and cons'"*.

## Decision-model router (optional)

Verified on 2026-11-15, not from memory:

- **Jev** — `GET https://api.typesafe.ai/v1/models` lists `jev-latest` and
  `jev-preview`. `POST /v1/systemone` takes `model`, `state` (the content) and
  named `questions`; a `choice` question takes `criteria` (choice name →
  when it applies) and returns `choice`, `confidence` and per-choice
  `probabilities`, plus `usage` token counts and the concrete model that
  answered. One live request with three candidate graphs returned a choice with
  its probabilities; that is a wiring check, not an accuracy evaluation.
- **Strands Decider** (github.com/strands-labs/strands-decider, Apache-2.0) —
  a family of open decision models that run locally, including on CPU. Its
  bundled server exposes the same `POST /v1/systemone` shape (`state`,
  `questions`, answers with `choice`/`noul`/`score`). It is a Python package
  the user installs and runs; graffy would talk to it over HTTP and bundle
  nothing, so there is no license inflow (ADR-0013).

Because both speak the same wire shape, one adapter with a configurable base
URL can serve either. Design constraints:

- A new seam in `graffy-core` (working name `Decider`), implemented outside
  core, in the same spirit as `ModelInvoker`: the only doorway to a decision
  model. The router node calls the seam; it never issues HTTP itself.
- **No hardcoded model names or endpoints.** Base URL and model come from
  configuration; a missing or unreachable decider fails loudly — it does not
  silently degrade to another router unless the user configured that fallback.
- The decider only chooses among **opted-in** graphs plus the conversation
  floor. Each candidate's criterion text is authored in that graph's TOML.
- The journal records what the decider reported — choice, confidence,
  probabilities, the answering model, token usage — attributed to that model.
  A confidence is the model's own report, not a graffy measurement, and a
  routing choice is never presented as proof the graph was the right one.
- **The acceptance threshold is an open question.** Strands publishes a
  calibration claim for its own models (answers at confidence ≥ 0.9 are right
  about 95% of the time on its short unseen classification tasks). graffy has
  no such evidence for either model on graffy prompts, so it must not ship an
  invented default: either the user sets the threshold explicitly, or a
  default is adopted only with a cited source and a stated scope.
- **Hosted means the prompt leaves the machine.** Using Jev sends the user's
  prompt to api.typesafe.ai. It must be explicit opt-in and disclosed in the
  routing explanation. A local Strands Decider keeps the prompt local.
- `cost_usd` stays 0.0 (no pricing tables, AGENT.md rule 3); reported token
  usage is journaled as reported.

## Identity and MCW requirements

A router run is not a conversation turn. It needs a distinct run kind and a
routing-decision event (protobuf additions), and — exactly like rating runs
today — it must be excluded from session history hydration, boundary
exchanges, HRDM windows and the per-turn attempt counts in `graffy metrics`.
One human message stays one external exchange whether or not it was routed.
The target run should reference the router run that selected it.

MCW reading (from issue #17): routing is a Selection-stage act on the user's
intent IUs; a mis-route is an early Drift signal; the one-line explanation
keeps the constraint legible (anti-Constraint-Opacity). "No, just chat" is a
human correction and keeps its boundary role.

## Not in scope here

Graphify-on-the-fly for prompts no graph covers, embeddings/vector matching,
and turning mis-routes into Lessons (C4) remain later increments under issue
#17.
