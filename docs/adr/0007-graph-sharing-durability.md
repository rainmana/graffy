# ADR-0007: Durable sharing — TOML specs in git, hash-pinned journal bundles

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

Graphs, benchmarks, and runs must be durable, portable artifacts users can share — including
with people who share nothing else (no server, no account).

## Decision

- **A graph is a TOML file.** `graffy graph export` writes the spec (+ provenance block);
  import validates and registers it. Git is the natural collaboration medium — diff, review,
  fork.
- **A run/benchmark result is a journal bundle**: protobuf RunEvent frames whose manifest pins
  the producing spec's SHA-256, the graph version, evidence policy, and harness version. Data
  and the process that produced it travel together and are verifiable.
- Built-in graphs ship in the same TOML format (unprivileged — exportable and forkable).
- Registry, signing, and discovery UX build on these files in Phase 5; the file formats are the
  contract, not the registry.

## Consequences

- Sharing works at Phase 1 with zero infrastructure (a file is the unit of exchange).
- Reproducibility claims are checkable: same spec hash + same models ⇒ comparable journals.
