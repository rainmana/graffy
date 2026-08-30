# ADR-0004: The MCW Framework as first-class protobuf core types

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

graffy heavily integrates the Meta-Context Window (MCW) Framework (W. Alec Akin, Apache-2.0):
coordination failures are named, detectable, repairable, and measurable. Bolted-on telemetry
always lies; if MCW is real in graffy, it must be in the type system from day zero.

## Decision

`graffy.mcw.v1` (in `src/protos/mcw.proto`) models, faithfully to canon:

- `InformationUnit` — payload text, timestamp, source actor, salience, evidence links, and the
  five-stage lifecycle (Selection, Encoding, Transmission, Decoding, Integration) as
  `IuStageRecord`s.
- The six failure modes — Drift, Asymmetric State Advancement, False Alignment,
  Overcompression, Constraint Opacity, Repair Suppression — as `FailureMode`/`FailureSignal`,
  plus `FailureModeVector` slots per run/session.
- The five repair operations — Re-grounding, Decompression, Re-weighting, Disambiguation,
  Synchronization — as `RepairOperation`/`RepairAction`, executable via `repair.*` node kinds.
- H/R/D/M observables as `HrdmSample`/`HrdmSeries` (anchored ordinal scores, versioned rubrics,
  heuristic / model-judge / human-survey sources).
- The epistemic floor as `EvidenceLevel` (L0 definitional → L3 validated) on IUs, artifacts,
  and claims.

Divergences from canon are labeled "graffy extension" in the schema (e.g. per-stage
`fidelity_estimate`, graph-level actors). The framework is early-stage and falsifiable; the
schema records observations without over-claiming solved theory (IU individuation stays open —
`IU_KIND_OTHER` exists on purpose).

## Consequences

- Every journal is an MCW dataset: H/R/D/M over time, failure→repair chains, IU lineage across
  compactions. Phase 4 surveys and benchmarks read the same types.
- graffy becomes a practical test bed for the framework's experiments — and its data could
  falsify parts of the framework. That is a feature, per the MCW Constitution.
