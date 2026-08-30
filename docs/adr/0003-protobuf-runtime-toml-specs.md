# ADR-0003: Protobuf for runtime payloads and journal; TOML for graph specs

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

The Phase 1 lock-down prescribes Protocol Buffers (via prost) for all internal message passing,
node execution payloads, and the append-only run journal — and human-readable TOML for graph
configurations so specs diff cleanly and live in git.

## Decision

- `src/protos/` (repo root) is the single source of truth: `mcw.proto` (coordination types) and
  `journal.proto` (run events). Packages are versioned (`graffy.mcw.v1`) — breaking schema
  changes mint `v2`, never mutate `v1`.
- Codegen in `graffy-proto` via **prost-build fed by protox** — a pure-Rust protobuf compiler,
  so neither contributors nor CI ever install system `protoc`.
- The journal on disk = length-delimited `RunEvent` frames; replay = fold(events).
- Graph specs are TOML (`graphs/*.toml`): `[graph]` metadata, `[[node]]`/`[[edge]]` arrays,
  `[policy.*]` tables. Specs are data; the compiler in `graffy-core` validates them.

## Consequences

- Runtime records are compact, typed, and evolvable; specs are human-legible and reviewable in
  PRs — each format where it is strongest.
- Two serialization systems to maintain; the seam is explicit (spec SHA-256 pinned inside every
  journal manifest, so a bundle proves which TOML produced it).
