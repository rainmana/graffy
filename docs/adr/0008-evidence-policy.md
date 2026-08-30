# ADR-0008: Evidence-backed execution and the epistemic floor

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

graffy agents default to never executing on inference ("guessing") alone: statements must be
backed by research artifacts. In non-research contexts (a DnD campaign), artifacts may stay out
of the interface — but they must exist and be referenced during execution. This mirrors the MCW
Constitution's epistemic floor: no claim above the evidence layer that supports it; summaries
must preserve falsification conditions.

## Decision

- Claims (IUs) link `EvidenceArtifact`s — hash-addressed, journaled, levelled L0–L3.
- Verify nodes enforce `policy.evidence.min_level`; model inference alone is L0 and can never
  satisfy an L1+ floor by itself.
- `policy.evidence.mode`: `strict` surfaces receipts in the UI; `trace-only` keeps them
  journal-only. There is no "off".
- Compaction runs as a graph whose output must preserve IUs and falsification conditions;
  detected loss raises an `Overcompression` failure signal with receipts.

## Consequences

- "Trust me" is not a state the system can represent — the honest cost is more tool calls and
  slightly slower answers, which the TUI explains in novice mode.
- Entertainment graphs stay immersive without sacrificing auditability.
