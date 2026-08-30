# ADR-0005: rig-core providers, rigs conversation nodes, custom graph-flow-patterned executor

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

The Phase 1 lock-down names the ecosystem: `rig-core` for model providers (Anthropic, OpenAI,
Ollama), `rigs` for multi-agent conversation nodes, and custom cyclic execution loops patterned
after `graph-flow` concepts — all in service of the invariant that no prompt ever executes as
raw text outside a graph. Verified 2026-08-29 on crates.io: rig-core 0.42.0 (MIT),
rigs 0.0.8 (MIT, M4n5ter — orchestration atop rig), graph-flow 0.6.0 (MIT).

## Decision

- **Providers**: `graffy-providers` wraps rig-core. Anthropic, OpenAI, and Ollama first-class;
  Venice.AI, OpenRouter, and LM Studio ride the OpenAI-compatible client with custom base URLs.
  A registry maps capability tiers ("fast"/"balanced"/"frontier"/custom) + cost metadata to
  concrete models; routing ladders speak tiers, not vendor names.
- **Multi-agent conversation nodes**: `graffy-agents` orchestrates persona panels via `rigs`.
  The integration surface stays thin (node semantics belong to graffy, transport belongs to
  rigs) so the dependency is swappable.
- **Executor**: custom, in `graffy-core`, patterned after graph-flow's typed-task step loop —
  NOT a dependency — because MCW instrumentation, evidence gating, budget-metered guarded
  cycles, and journaled routing must be native, not adapters.
- **The invariant**: the provider layer exposes no free-standing completion call. Model access
  exists only as node execution scheduled by the executor. Skills and prompts get *graphified*
  (Phase 2 flows: adopt / guided / collaborative) before first execution.
- **Smart routing**: `policy.routing` in specs (ladder + `on_quality_fail`: escalate | reroute
  | halt). Escalation targets the next tier up or a designated model (including local
  fine-tunes) — rejected work never silently returns to its producer. Every decision is
  journaled as a `RoutingDecision`.

## Risks

- `rigs` is 0.0.x — early. Mitigation: thin seam in `graffy-agents`; if it stalls, we implement
  conversation orchestration directly on rig-core behind the same node kind (would be recorded
  as a superseding ADR).
- rig major-version churn: pinned at the workspace root; upgrades are deliberate PRs.
