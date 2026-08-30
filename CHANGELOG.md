# Changelog

## v0.1.0-alpha.3 — the release that ships binaries (2026-08-30)

Supersedes the alpha.2 tag, which was pushed from a clone carrying a stale
local tag and therefore pointed at a pre-fix tree whose release workflow
requested a retired runner — no binaries ever attached. Fresh tag name, no
deletions required anywhere. Contents: everything below (alpha.2 + alpha.1)
plus graphify v1 — `graffy graphify <SKILL.md|prompt>` compiles skills and
raw prompts into durable graphs on the verified conversation floor
(auto-adopt mode; guided/collaborative gate until the TUI flows land).
Targets: Linux x86_64 + aarch64, macOS Apple Silicon.

## v0.1.0-alpha.2 — first version-correct release (2026-08-29)

Version strings match the tag; CHANGELOG and install docs land in-tree; the
Phase 2 MCP design doc arrives (facade subgraphs, MCW-aligned node taxonomy,
failure-mode-aware usage interview). The final tag additionally carries the
first two Phase 2 slices: the engine tool plane (ToolInvoker, tool.invoke
nodes, MCP evidence artifacts) and the rmcp client with `graffy mcp add`
(annotation-seeded roles, skill-fronted facade generation, hermetic
fixture-server protocol tests). Release targets: Linux x86_64 + aarch64,
macOS Apple Silicon (Intel Mac dropped by decision — retired runners,
retiring architecture).

## v0.1.0-alpha.1 — Phase 1 complete (2026-08-29)

*(Tagged at `a7525c1`, one commit before the version-bump landed — its
binaries report `0.1.0-alpha.0`. Everything below is in that tag.)*

First tagged release. Everything below shipped in one day, every push CI-green
on Linux + macOS, 25 tests.

### The invariant, working

No prompt, skill, or chat turn executes outside a graph. Even plain chat runs
`intake → ground → draft → verify → respond`, and the field test proved the
point: asked about a term the model couldn't ground (no research tools yet),
the verify gate issued REVISE three times, the visit cap tripped, and the run
**failed honestly instead of hallucinating** — with every receipt in the
journal.

### Engine (graffy-core)

- TOML graph specs: human-readable, git-friendly, shareable durable objects
- Cycle-guard compiler: unguarded cycles are unlawful; guarded back-edges +
  per-node visit caps + token/USD/time budgets make loops safe
- tokio executor with guard grammar (`==`, `!=`, truthy, `&&`), routing
  escalation ladders (quality failures route UP, never bounce back), and
  `NodeExecutionResult::{Continue, Escalate, PauseForDisambiguation}`
- Append-only run journal: length-delimited protobuf frames, flushed per
  event, replayable; live tap mirrors exactly what is written

### MCW Framework, first-class ([mcw-framework](https://github.com/rainmana/mcw-framework))

- `graffy.mcw.v1` protobuf schema: Information Units with the five-stage
  lifecycle, the six failure modes, the five repair operations as node kinds,
  H/R/D/M observables, evidence levels L0–L3
- Every journal is an MCW dataset; claims carry hash-addressed evidence
  artifacts with explicit epistemic levels

### Providers (rig-core)

- Anthropic, OpenAI, Ollama, Venice, OpenRouter behind capability tiers
  (`GRAFFY_MODEL_<TIER>=provider:model`) — no hardcoded model names, ever
- The only path to a model is an executor-scheduled graph node

### TUI (Ratatui)

- Live run view: pipeline states (glyph AND word — never color-only), live
  journal feed, MCW counter strip, plain-language novice line
- Step inspector: per-node IUs, evidence, model calls, routing decisions
- Interactive approvals: freeze-frame modal (approve / reject / edit);
  quitting mid-approval rejects — silence never releases
- CLI parity for everything, including stdin approvals where EOF rejects

### Persistence (libSQL, embedded)

- Graph registry (validated at the door; built-ins seed automatically),
  run history, queryable journal mirror, sessions — one `graffy.db` file

### Built-in graphs (forkable TOML like everything else)

- `graffy.builtin.conversation` — the floor every chat runs on
- `graffy.builtin.conversation.gated` — human release gate before respond
- `graffy.builtin.reasoning.sequential-thinking` — plan → think → critique
  (guarded revise loop) → synthesize (Clear-Thought lineage)
- `graffy.builtin.reasoning.decision-framework` — frame → evaluate →
  challenge → decide (Clear-Thought lineage)

### Coming in Phase 2

MCP servers as first-class citizens (tools become graph nodes, results become
evidence — see `docs/design/phase-2-mcp.md`), skill/prompt → graph
conversion with three involvement modes, and the reasoning-template catalog.
