# graffy roadmap

Phases gate scope; nothing ships half-instrumented. Telemetry schemas (journal + MCW) landed in
Phase 0 precisely so later phases never retrofit observability.

## Phase 0 — Foundation ✅ (this repo state)

Workspace scaffold (11 crates), ADRs 0001–0009, MCW + journal protobuf schemas, TOML graph spec
format with parser + cycle-guard compiler, default conversation graph, CI (fmt/clippy/test on
Linux + macOS) and tag-triggered release binaries.

## Phase 1 — The thesis, visibly true

Goal: a stranger can watch a prompt refuse to run raw.

- **M1 — journal writer/reader**: length-delimited RunEvent frames + replay fold.
- **M2 — executor**: tokio step loop over the compiled topology; guarded back-edges consume
  budget; provider calls via rig (Anthropic, OpenAI + compatibles, Ollama); routing ladder with
  journaled `RoutingDecision`s; `agent.conversation` nodes via rigs.
- **M3 — TUI**: live run view (node states, edge pulses), step inspector (IUs, evidence, model
  calls), novice-mode explanations.
- **M4 — persistence**: libSQL schema v1 — sessions, journal mirror, graph registry.
- **M5 — durable sharing**: `graffy graph export/import` (TOML + provenance + spec hash);
  `graffy run --replay <journal>`.
- **M6 — acceptance demo**: `graffy run graphs/conversation.default.toml --prompt "..."`
  executes against a real provider, visibly, and exports a shareable bundle.

## Phase 2 — Tools and graphification

- MCP client via rmcp: every connected server's tools become `tool` nodes; results become
  evidence artifacts.
- **Skill/prompt ingestion** with three involvement modes: *adopt* (existing graphs absorb the
  skill, light oversight), *guided* (user checkpoints on outcomes), *collaborative* (node-by-node
  co-design). Output is always a durable TOML graph.
- Built-in catalog expansion: Clear-Thought-derived templates (mental models, OODA loop,
  Ulysses protocol, collaborative reasoning, socratic method…), each optionally embeddable as a
  subgraph in user graphs.
- Quality-gate escalation live end-to-end (verify → routing ladder → stronger model / local
  fine-tune / section-level regeneration).

## Phase 3 — Memory and MCW, live

- libSQL memory stack: verbatim episodic log (FTS), native vector search (fastembed local or
  cloud embeddings), temporal knowledge graph with validity windows, MemPalace-style layered
  wake-up (identity → essentials → scoped recall → deep search).
- MCW detectors watching journal streams (drift, overcompression, false alignment…);
  repair graphs (`repair.*`) triggerable automatically or by hand.
- **Compaction-by-graph** replaces nothing (there was never naive summarization): context
  pressure runs the compaction graph, which must preserve IUs and falsification conditions or
  raise `Overcompression` with receipts.
- H/R/D/M heuristic + model-judge sampling over time.

## Phase 4 — Evals, surveys, sharing

- Benchmark runner: industry-standard adapters + MCW-native metrics; results as portable
  bundles (data + the process that produced it: spec hash + journal).
- In-TUI human survey flow with anchored ordinal rubrics (versioned; MCW test-bed aligned).
- Over-time dashboards (H/R/D/M series, benchmark trends) in the TUI.
- New benchmarks as data: a benchmark bundle = task set + rubric + runner graph, shareable and
  forkable like any graph.

## Phase 5 — The learning loop

- Hermes-inspired closed loop: repeated successful patterns propose new graph templates;
  proposals are versioned, human-approved, roll-back-able (capability snapshots).
- Graph registry UX: install/inspect/diff/pin shared graphs; provenance and signing.
- Novice onboarding polish; docs site.

## Tabled by request (post-Phase-1, recorded so they cannot get lost)

- **Signed + notarized macOS releases**: @rainmana holds a paid Apple Developer
  membership; wire Developer ID Application signing (`codesign`) and notarization
  (`notarytool` submit + staple) into the release workflow via GitHub secrets so
  downloads run without Gatekeeper exceptions. Parked by request — the unsigned
  binary is field-verified working on macOS as of v0.1.0-alpha.3.
- **Brand & visual identity**: exploratory brand/theme work for graffy already exists from
  side efforts (logo routes, palettes, light/dark mode studies). Not integrated anywhere yet —
  when the theme engine lands, that work is the natural seed for graffy's default identity
  and first-party theme.
- **Theme engine**: first-class TUI themes — Catppuccin, Dracula, and Nord out of the box,
  plus user-defined theme files (a `themes/*.toml` format in the same durable spirit as graph
  specs); optional Unicode / Nerd-Font icon sets with a plain-ASCII fallback that is always
  available.
- **Accessibility as a design principle** (neurodivergence-aware and beyond): already in place —
  no color-only signaling (every node state renders a glyph AND a word), minimal motion, and a
  plain-language novice strip. Tracked follow-ups: honoring `NO_COLOR`, high-contrast and
  reduced-motion modes, configurable tick rates, keyboard-only operation audits, predictable
  layout (no reflow surprises), and plain-output parity for every TUI view so screen readers
  and pipes get the same information (`replay` without `--tui` already is that parity for
  inspection).

Out of scope (explicitly, per ADR-0002): GUI/web/Tauri clients. graffy is terminal-native.
