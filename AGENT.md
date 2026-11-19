# AGENT.md — working on graffy

This file is for any AI coding agent (and any human) picking up this project.
It captures what the code cannot say about itself: the invariant, the
discipline, the seams, the owner's working style, and the lessons already
paid for. Read it before changing anything. `CLAUDE.md` points here.

## What graffy is

A terminal-native agentic harness (Rust, Apache-2.0) with one
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
   (Apache-2.0-compatible only; no copyleft code, ADR-0013), deferred ideas in ROADMAP "Tabled" (they must
   not evaporate — write them down with design breadcrumbs), field
   observations in `docs/KNOWN-ISSUES.md`.
5. **Generated things must be lawful**: specs produced by graphify or facade
   generation are parsed + compiled before registration; generated TOML is
   built via `GraphSpec` structs + serializer, never string templates.
6. **Tests tell the story**: every failure mode demonstrated (EOF rejects,
   no-plane fails loudly, unguarded cycle rejected, visit cap halts,
   rejection releases nothing). An end-to-end offline run of the default
   conversation graph is the acceptance floor.

## Reviewing and merging PRs (Codex auto-review)

A PR is not done when CI is green. **Codex reviews every PR automatically.**

- **Open the PR, then wait.** Do not merge on green CI alone. Wait for Codex's
  review comments and an explicit approval (or no open findings).
- **Reply to Codex yourself.** Reply to every review comment as the agent, and
  make it unmistakable that it is not @rainmana even though the PR sits on his
  account. Sign each reply, e.g. a final line:
  `— Claude (agent), on @rainmana's behalf`.
- **Resolve every thread, one of two ways:**
  1. **Fix it:** make the change, commit it, say which commit addresses it,
     re-run the gates (`cargo fmt --all --check`,
     `cargo clippy --workspace --all-targets -- -D warnings`,
     `cargo test --workspace`), and push.
  2. **Argue it:** if the finding does not apply, explain the code/spec/ADR
     reason to Codex and resolve the thread. "Won't fix" without a reason is not
     enough.
- **Merge only after** Codex has no open threads and has given a thumbs up.
- Prefer added commits over rewrites while review is open, so Codex keeps its
  context.

## What to work on next (priority order)

Canonical detail lives in GitHub Issues. The order below is the tie-breaker when
two issues compete; it keeps parallel agents from all chasing features while the
core live path is still rough. Distribution is deliberately last.

- **P0 — make the live path trustworthy.** #30 context policy for long
  conversations; #32 live-provider end-to-end smoke; #35 abandoned-approval
  stdin swallow; #34 quiet plain-CLI logging.
- **P1 — make it do real work.** #31 give `research` a real behavior (grounding /
  MCP evidence reachable from conversation); #33 token streaming / visible
  progress.
- **P2 — robustness and honesty.** #36 store WAL/busy_timeout; #38 decider
  timeout; #37 refresh ARCHITECTURE.md; #3 C4 Lessons + `graffy lessons`.
- **P3 — distribution (tabled by the owner 2026-11-19; not blockers).** #28
  Homebrew tap, then #12 / PR #29 signing, then #25 docs site.
- **P4 — the long tail the owner keeps adding.** #17 routing follow-ons; #7/#8;
  #9/#11; #10; #1–#6; #13/#14.

**Definition of "solid enough to hand to people":** a stranger can install, bind
a model, hold a long multi-turn conversation that uses a tool, cancel and resume
without a panic, and see honest failures — with the gates green and Codex review
clean. Signed/notarized binaries and a Homebrew tap are explicitly not on that
critical path.

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

## Where the project is (alpha.7, 2026-11-16)

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
- Chat (ADR-0010): steps 1–3 are in — `src/turn.rs` is the shared turn runner
  every frontend drives (never add a second execution path), `graffy chat` is the
  plain-terminal loop, and `graffy chat --tui` is the Ratatui conversation view.
  The view records an output receipt only after drawing the answer, and chat
  approvals need Ctrl chords (typing must never decide a gate) — keep both.
  Routing (ADR-0012): `chat --route` runs `graffy.builtin.router` as its own
  run (kind ROUTING — never a turn; keep it out of every history, exchange and
  execution-metric path, like rating runs). Graphs opt in with `route_on`;
  absent means explicit-only. The `router` node is registered by the frontend
  with the catalog, not in the default registry. `--route=decider` asks a
  decision model through the `graffy_core::route::Decider` seam (HTTP adapter:
  `graffy-providers/src/decider.rs`; Jev or a local Strands Decider). It has no
  defaults by design — endpoint, model and `GRAFFY_ROUTER_MIN_CONFIDENCE` are
  all the human's to set — and it fails loudly rather than degrade. Still open
  under issue #17: graphify-on-the-fly, embeddings, mis-routes as Lessons. The TUI chat has been exercised in a
  pseudo-terminal and by render tests, not yet by a human on real terminals.
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
cargo run -- chat --offline                  # terminal chat; every line is a graph turn (src/turn.rs)
cargo run -- chat --offline --tui            # Ratatui conversation view (crates/graffy-tui/src/ui/chat.rs)
cargo run -- chat --offline --route          # route messages onto opted-in graphs (crates/graffy-core/src/route.rs)
cargo run -- chat --route=decider            # same, chosen by a decision model (needs GRAFFY_DECIDER_URL + GRAFFY_ROUTER_MIN_CONFIDENCE)
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
