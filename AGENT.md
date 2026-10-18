# AGENT.md — working on graffy

This file is for any AI coding agent (and any human) picking up this project.
It captures what the code cannot say about itself: the invariant, the
discipline, the seams, the owner's working style, and the lessons already
paid for. Read it before changing anything. `CLAUDE.md` points here.

## What graffy is

A terminal-native agentic harness (Rust, GPL-3.0-or-later) with one
non-negotiable invariant: **nothing executes as a raw prompt or skill.**
Every prompt, imported skill, MCP tool call, compaction step, and chat turn
is compiled into a typed, budgeted, journaled **graph** that persists as a
durable, shareable TOML artifact. Even plain chat runs
`intake → ground → draft → verify → respond`.

The second pillar: the **Meta-Context Window (MCW) Framework** by the
project owner, W. Alec Akin (@rainmana; Apache-2.0;
github.com/rainmana/mcw-framework). MCW is not decoration — it is in the
type system. Information Units (with their five transfer stages), the six
coordination failure modes (Drift, Asymmetric State Advancement, False
Alignment, Overcompression, Constraint Opacity, Repair Suppression), the
five repair operations, H/R/D/M observables, the operational support ladder,
and the distinct MCW Article IV evidence layers L0–L4 are represented by
protobuf types in `src/protos/mcw.proto`. Runtime observations are journaled;
the Article IV ladder is reserved for empirical claims about MCW. When you
design a feature, name the MCW construct it implements (see
`docs/design/phase-2-mcp.md` §7 for the pattern).

## Architecture in one screen

Cargo workspace, single `graffy` binary, single tokio process (ADR-0001/2).

- `src/protos/` — canonical schemas: `mcw.proto` (MCW types), `journal.proto`
  (append-only RunEvent stream). Compiled by `graffy-proto` via **protox**
  (pure Rust — never introduce a system `protoc` dependency).
- `graffy-core` — TOML specs (`spec.rs`), cycle-guard compiler (`graph.rs`:
  unguarded cycles are unlawful; entries = no incoming edges, guarded-reentry
  fallback for pure cycles), executor (`exec.rs`: guard grammar `==`/`!=`/
  truthy/`&&`, per-node visit caps, budgets, escalation ladders,
  `NodeExecutionResult::{Continue, Escalate, PauseForDisambiguation}`),
  journal writer/reader (`journal.rs`, length-delimited protobuf frames,
  flushed per event, optional live tap for the TUI).
- **The seams** (dependency direction is law): `graffy-core` defines traits;
  implementations plug in from outside. `ModelInvoker` (only doorway to
  models — there is deliberately no free "complete this prompt" function),
  `ToolInvoker` (only doorway to tools), `ApprovalHandler` (humans;
  `describe()` lands in the journal — artifacts must not lie about who
  decided). `OfflineEcho` is the deterministic invoker for tests/demos.
- `graffy-providers` — rig-core 0.42 (lib name is `rig_core`, NOT `rig`).
  Tiers, not vendor names: `GRAFFY_MODEL_<TIER>=provider:model` env bindings
  (anthropic | openai | openrouter | venice | ollama). **Never hardcode
  model names** — vendors rename faster than releases ship.
- `graffy-mcp` — rmcp 3.1 stdio client, discovery with annotation-seeded
  roles (`readOnlyHint→evidence`, `destructiveHint→effector`+approval gate,
  unknown→effector: conservative always), prompts-primitive import as usage
  knowledge, facade generation (`prepare → [approve] → invoke → digest → verify → respond`),
  the usage interview logic (`interview.rs`, pure + unit-tested).
  Test fixture: `tests/fixture/mini_server.py` — a genuine MCP stdio server
  in Python; the full client round-trip runs hermetically in CI.
- `graffy-memory` — embedded libSQL (feature `core` only — default features
  drag a network stack). Graph registry (validates before storing — the
  registry can never hold an unlawful graph), run history, queryable journal
  mirror, MCP server registry (transport lives HERE, never in specs — shared
  graphs stay portable). Migrations: idempotent CREATE + column-probe ALTER.
- `graffy-graphs` — built-in graphs as TOML (unprivileged: exportable,
  forkable) + `graphify.rs` (skill/prompt → graph; three involvement tiers:
  auto / guided / collaborative; artifacts record their tier in `authors`).
- `graffy-tui` — ratatui 0.30 (use `ratatui::crossterm`, one blessed
  version). Live run view folds the journal tap; step inspector; review
  surface (`preview.rs`) with help overlay; approval modal. Accessibility is
  a requirement: never color-only signaling (glyph AND word), minimal
  motion, plain-language novice strip, plain-terminal parity for every TUI
  capability (piped stdin: EOF always REJECTS — silence never registers or
  releases anything).

## Non-negotiable discipline

1. **Gates before any push**: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`. CI runs exactly these on Linux + macOS.
2. **Verify dependency APIs against real source** — download the published
   crate from `static.crates.io/crates/<name>/<name>-<ver>.crate` and read
   it. Multiple real bugs were avoided/caught this way (rig's lib name,
   rmcp's non-exhaustive params and response enums, ulid 3.0's
   `generate()`, libsql feature bloat). Never code against a remembered API.
3. **Honest values only.** `cost_usd` is 0.0 on purpose (no pricing tables
   yet — a guessed price would poison budgets and benchmarks). Unverifiable
   numbers, invented model names, silently-degraded fallbacks: all
   forbidden. When something can't be done, say so loudly (see
   `ToolError::Unavailable`, the collaborative non-TTY bail).
4. **Decisions live in ADRs** (`docs/adr/`), inflows in `NOTICE.md`
   (GPLv3-compatible only), deferred ideas in ROADMAP "Tabled" (they must
   not evaporate — write them down with design breadcrumbs), field
   observations in `docs/KNOWN-ISSUES.md`.
5. **Generated things must be lawful**: specs produced by graphify or facade
   generation are parsed + compiled before registration; generated TOML is
   built via `GraphSpec` structs + serializer, never string templates.
6. **Tests tell the story**: every failure mode demonstrated (EOF rejects,
   no-plane fails loudly, unguarded cycle rejected, visit cap halts,
   rejection releases nothing). An end-to-end offline run of the default
   conversation graph is the acceptance floor.

## Working with Alec (@rainmana)

- Self-taught, information-theory day job. **Teach as you work**: what, why,
  and how it could succeed, in plain language. No unexplained steps.
- Credit his ideas explicitly (facades fronting endpoints, the usage
  interview, graffy-administers-graffy, LangGraph export are his).
- He runs Windows PowerShell (no `&&` — separate lines or `;`), WSL, and
  macOS. He tests binaries on real hardware and reports honestly.
- He performs the steps agents can't in some setups: workflow-file edits,
  tag pushes, release management. Ask explicitly; give exact commands.
- Tag hygiene lesson (paid for twice): `git push --delete origin <tag>` does
  NOT delete local tags; a stale local tag re-pushes the old commit, and
  tag-triggered workflows execute the workflow file FROM THE TAGGED COMMIT.
  When in doubt, use a fresh tag name and bump the version to match.
- Releases: Linux = musl static (no glibc floor), macOS = Apple Silicon
  only (Intel dropped deliberately). A post-build `--version` smoke step is
  mandatory. Apple signing/notarization is tabled (he has a paid Apple
  Developer membership) — see ROADMAP.

## Where the project is (alpha.6, 2026-10-17)

- Runtime, journals, provider adapters, MCP facades, graphification, store and
  Ratatui inspection work. The graph loop is serial; general branch/join scheduling
  is not implemented.
- Release fixes: exclusive journal creation, policy validation, nonzero failure
  exits, exact-payload approvals, cancellation/deadlines, whole-turn retry budgets,
  output receipts, exact-draft review identity, and strict session-history loading.
- MCW: typed IUs/support artifacts, judge hypotheses, internal retry repair
  records, human corrections, confirmed boundary exchanges, and human HRDM
  calibration with frozen proposals and reviewed segmentation. Read
  `docs/mcw/alpha6-adaptation.md`; the instrument remains draft. Preserve pinned
  anchors and the historical author review. Do not equate run success, same-mode
  non-recurrence, or a delivery receipt with target repair or human understanding.
- The generic research node is a no-op. Intake uses a whole-prompt IU.
  `graffy-mcw` exports vocabulary; `graffy-agents` and `graffy-evals` are scaffolds.
  FTS/vectors/temporal recall, learned lessons, model raters, fine-grained claim
  grounding, executable repair graphs, pricing and bundles remain future work.
- Next: chat in Ratatui and CLI, every prompt still a graph (ADR-0010).
  Desktop/web is tabled indefinitely. The optional graph inspection view is not
  a prerequisite for chat.
- After chat: native Windows x64 CI/release builds are a low-priority portability
  follow-up requested by the owner; see ROADMAP. Windows is not yet a supported
  release target, and this work does not gate alpha.6 or chat.
- Open TODOs live in GitHub Issues. `docs/RELEASE-READINESS.md` distinguishes the
  original audit from implemented fixes and final verification. Historical design
  docs must not be treated as claims of shipped capability.

## Quick commands

```
cargo test --workspace                       # offline suite; live MCP test is explicitly ignored
cargo run -- run graffy.builtin.conversation --prompt "hi" --offline --tui
cargo run -- run <graph> --prompt "..." --retry auto   # C2 repair-feedback retries
cargo run -- rate <session-id> [--blinded]   # boundary-transcript HRDM rating (calibration)
cargo run -- run <graph> --prompt "..." --session <id>  # continue a session (adds an exchange)
cargo run -- graphify SKILL.md --mode collaborative
cargo run -- mcp add fixture --stdio "python3 crates/graffy-mcp/tests/fixture/mini_server.py"
cargo run -- replay graffy-runs/<id>.journal --tui
cargo run -- init                            # create ~/.graffy, seed built-ins
cargo run -- metrics --json                  # fold journals into research metrics
cargo run -- doctor                          # bindings, store, credentials presence
GRAFFY_DATA_DIR=<dir>                        # override store location (tests/demos)
cargo test -p graffy-mcp -- --ignored        # real npx MCP server round-trip
```

Live runs: set `GRAFFY_MODEL_FAST/_BALANCED/_FRONTIER=provider:model` plus
the provider's key env (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
`OPENROUTER_API_KEY`, `VENICE_API_KEY`; Ollama needs none, honors
`OLLAMA_API_BASE_URL`).
