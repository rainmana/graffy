# graffy architecture

> Locked for Phase 1 on 2026-08-29. Every decision here is traceable to an ADR in `docs/adr/`.

## The one invariant

**Nothing executes outside a graph.** A user prompt, an imported skill, a memory recall, a
summarization pass, a benchmark run, a DnD turn — each is compiled into a typed, budgeted,
journaled graph before any model sees it. There is no code path from user input to a model call
that bypasses the executor, and the provider layer deliberately exposes no "just complete this"
function (ADR-0005).

## Runtime shape

Single process, single binary (ADR-0001, ADR-0002). The TUI, executor, memory store, and
providers all live in one tokio runtime — no daemons, no sidecars, no IPC. "Distribution" means
copying one file (GitHub Actions builds macOS + Linux artifacts per tag).

```
┌──────────────────────────── graffy (one process) ────────────────────────────┐
│                                                                              │
│  graffy-tui (ratatui)                                                        │
│    live run view ◄── folds ── journal stream (protobuf RunEvent frames)      │
│    step inspector ◄─ reads ── journal + evidence store                       │
│         │ user intents (run graph, approve node, answer survey)              │
│         ▼                                                                    │
│  graffy-core executor  ── petgraph topology from TOML spec                   │
│    │ schedules nodes; enforces guards, budgets, evidence policy              │
│    ├─► graffy-providers (rig) ── Anthropic / OpenAI(+compat) / Ollama        │
│    ├─► graffy-agents (rigs) ──── multi-persona conversation nodes            │
│    ├─► graffy-mcp (rmcp, P2) ─── MCP tools as nodes                          │
│    ├─► graffy-mcw ────────────── IU ledger, detectors, repair nodes, H/R/D/M │
│    └─► graffy-graphs ─────────── built-in templates (also plain TOML)        │
│         │                                                                    │
│         ▼                                                                    │
│  graffy-memory (libSQL, embedded) ── episodic log • vectors • temporal KG    │
│                                      sessions • journal mirror • registry    │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Serialization split (ADR-0003)

| Concern | Format | Why |
| --- | --- | --- |
| Graph specs (durable, shared, versioned) | **TOML** | Human-readable, diff-able, lives happily in git. `graphs/*.toml` is the interchange format. |
| Node payloads, internal messages, run journal | **Protobuf** (prost, compiled by pure-Rust protox) | Typed, compact, evolvable, replayable. `src/protos/` is the single source of truth. |

A shared graph is a TOML file (+ optional provenance); a shared *run* or *benchmark result* is
a journal bundle of protobuf frames whose manifest pins the spec's SHA-256 — process and data
travel together (ADR-0007).

## MCW is in the type system (ADR-0004)

The [Meta-Context Window Framework](https://github.com/rainmana/mcw-framework) supplies the
coordination vocabulary, encoded in `graffy.mcw.v1`:

| MCW construct | graffy representation |
| --- | --- |
| Information Unit (+ 5-stage lifecycle) | `InformationUnit` / `IuStageRecord` — the payload currency of edges |
| 6 failure modes | `FailureMode` + `FailureSignal` (+ per-run `FailureModeVector` slots) |
| 5 repair operations | `RepairOperation` + `RepairAction`, executable as `repair.*` node kinds |
| H/R/D/M observables | `HrdmSample` / `HrdmSeries` — heuristic, model-judge, or human-survey sourced |
| Epistemic floor | `EvidenceLevel` (L0–L3) on every claim; verify nodes enforce `policy.evidence.min_level` |
| Constitution's "summaries preserve falsification conditions" | compaction runs as a graph whose output IUs must cover the input's guarded distinctions, or an `Overcompression` signal is raised |

## Crate map

| Crate | Owns | Phase |
| --- | --- | --- |
| `graffy-proto` | Generated protobuf types (MCW + journal) | 0 ✅ |
| `graffy-core` | Spec parsing, cycle-guard compiler, executor, journal | 0 partial → 1 |
| `graffy-providers` | rig-backed model access + capability/cost registry for routing | 1 |
| `graffy-agents` | rigs-backed multi-persona conversation nodes | 1–2 |
| `graffy-mcw` | IU ledger, failure detectors, repair nodes, H/R/D/M sampling | 1 schema → 3 live |
| `graffy-memory` | Embedded libSQL: episodic log, vectors, temporal KG, sessions, registry | 1 schema → 3 |
| `graffy-graphs` | Built-in graph templates (TOML, unprivileged) | 1 seed → 2 catalog |
| `graffy-mcp` | rmcp client; MCP tools as nodes; later graffy-as-MCP-server | 2 |
| `graffy-evals` | Benchmarks, surveys, portable eval bundles | 4 |
| `graffy-tui` | Live run view, step inspector, novice mode | 1 |
| `graffy` (root) | The single binary: CLI + TUI entry | 0 ✅ |

## Smart routing (ADR-0005)

Specs request capability *tiers* (`model_tier = "balanced"`); the provider registry resolves
tiers to concrete models with cost metadata. `policy.routing.ladder` orders tiers weakest-first;
when a verify/review node rejects content, the executor consults `on_quality_fail`:

- `escalate` — re-dispatch to the next tier up (or a designated local fine-tune), never
  silently back to the producer;
- `reroute` — same tier, different provider;
- `halt` — stop and ask the human.

Every decision is journaled as a `RoutingDecision` — routing is inspectable, not folklore.

## Evidence policy (ADR-0008)

Claims carry evidence artifacts (hash-addressed) with `EvidenceLevel` tags. Verify nodes block
propagation of claims below the graph's `min_level`. `mode = "strict"` surfaces receipts in the
UI; `mode = "trace-only"` keeps them journal-only (the DnD-campaign case) — but they must exist
either way. Model inference alone is L0 and can never satisfy an L1+ floor.
