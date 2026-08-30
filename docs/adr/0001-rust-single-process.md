# ADR-0001: Rust, single-process runtime, single binary

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

graffy must be fast, shippable as a single binary, and terminal-native. The graph semantics
(durable shareable graph objects, graphs inspecting graphs, MCW-instrumented edges) are custom
enough that no existing orchestration framework (LangGraph et al.) covers them — the engine is
bespoke in any language, which removes the main argument for Python/TypeScript. A prior
clarifying round confirmed Rust; the Phase 1 lock-down added: optimize all architecture for a
single-process runtime.

## Decision

- Rust, stable channel, edition 2024, one cargo workspace, one released binary (`graffy`).
- **Single process**: TUI, executor, memory, and providers share one tokio runtime. No daemons,
  no sidecar services, no IPC boundaries inside the harness.
- Async via tokio, initialized at the binary entry point (`#[tokio::main]`) from day one: the
  Ratatui event loop must stay non-blocking while the executor concurrently streams model
  tokens and processes Information Units — so the root package always carries tokio with
  `macros` + `rt-multi-thread` enabled.
- Graph topology via petgraph; identifiers are ULIDs (time-sortable).

## Consequences

- Distribution is one file per platform; `cargo install` also works.
- Concurrency is task-level inside the executor, not process-level — simpler failure domains,
  and the journal can be a plain append-only file without cross-process coordination.
- A future multi-process mode (if ever) must arrive through a new ADR, not accretion.
