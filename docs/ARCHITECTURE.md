# graffy architecture

Current implementation: alpha.6 (2026-10-17). Decisions live in [ADRs](adr/README.md).

## Runtime and ownership

Nothing executes outside a graph. The Rust binary hosts CLI, Ratatui, Tokio
execution, model/tool adapters, and embedded libSQL.

```text
CLI / Ratatui → TOML compiler → executor → models / tools / approvals
                                  ↓
                           protobuf journal → replay / metrics / boundary projection
                                  ↓
                             libSQL indexes
```

| Crate | Implemented responsibility |
| --- | --- |
| graffy-proto | Canonical protobuf schemas compiled by protox; no system protoc |
| graffy-core | Specs, compiler, execution traits, cancellation, journals, projection, metrics |
| graffy-providers | rig-backed tier bindings; no pricing tables |
| graffy-mcp | rmcp stdio discovery/interview, facade generation and tool receipts |
| graffy-graphs | Four built-ins and fixed-floor graphification |
| graffy-memory | Graph/MCP registries, session/run indexes, journal mirror |
| graffy-tui | Live/replay inspection, graphify review, exact-payload approvals |
| graffy-mcw | Type exports and repair names, no executable repair registry |
| graffy-agents / graffy-evals | Scaffolds, no active persona/evaluation runner |
| graffy root | CLI orchestration, retries, continuation, delivery receipts, human ratings |

Core traits define the execution doorways; adapters depend on core. The executor
is serial, not a general parallel branch/join scheduler. Replay inspects records;
it does not resume or re-execute. Chat will use this path per turn
([ADR-0010](adr/0010-chat-through-graphs.md)); desktop/web is tabled indefinitely.

## Execution and release

Specs are TOML; journals contain length-delimited protobuf frames with run and
sequence identity. Journal creation is exclusive. Manifests pin the spec hash;
they do not embed the full TOML. Portable research bundles remain future work.

Only successful runs release an answer. The frontend writes and flushes output
before appending its delivery receipt. Projection merges that receipt with the
released candidate, counts confirmed exchanges, and separates internal retries.
Legacy unconfirmed output stays unknown. Delivery does not prove attention or
understanding. See [ADR-0011](adr/0011-release-and-boundary-receipts.md).

Approval exposes the exact payload and hash. Edits require rejection and a new
correction turn. Human decisions differ from abandonment and automatic approval.
A review pass applies only to the exact draft IU. New MCP facades run intake →
prepare → [approve] → invoke → digest → verify → respond. Revisions repeat
digest/review, not the tool effect. Stored facades are not silently upgraded.

## Policies and limits

Routing tiers resolve through environment bindings. `escalate` advances eligible
successors after review failure; `halt` stops. Unsupported policies fail compilation.
Different tiers can still resolve to the same provider/model.

Deadlines interrupt model, tool and approval waits. Retries share remaining tokens,
recorded USD cost and time. Providers apply a separate per-call output cap (4,096
by default, configurable per tier) bounded by the remaining aggregate allowance
after a conservative input reserve. No model-limit discovery or exact tokenizer
is implemented; reported usage can still overshoot the estimate. Live prices
remain unavailable, so USD ceilings cannot enforce actual spend. Cancellation
cannot roll back external effects. See ADR-0011 for the reserve policy.

Operational `SupportLevel` is separate from study-level `McwEvidenceLayer`.
Verification first checks receipts and support, then asks the configured reviewer
about the original request, draft and consumed source IUs. That provider may differ
from the drafter. This is lineage plus judgment, not deterministic claim entailment.
The generic research slot is a no-op. Intake records one whole-prompt IU.

## MCW measurement boundary

Failure labels are uncalibrated hypotheses with source artifacts. Assigned salience
is labeled; unobserved five-stage fidelity is absent. Internal repairs record cost
and run-passed outcomes, not demonstrated target resolution. Human HRDM collection
uses confirmed boundary units with frozen proposals and separately recorded
corrections. The [v2 adaptation](mcw/alpha6-adaptation.md) remains draft/calibration only.

Learned lessons, model raters, executable repair graphs, fine-grained IU/claim
grounding, FTS/vectors/temporal recall, compaction, pricing and research bundles
remain future work.
