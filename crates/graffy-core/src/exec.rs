//! The executor — Phase 1 milestone M2 (see docs/ROADMAP.md).
//!
//! Design (ADR-0001, ADR-0005): a single-process, tokio-driven step loop
//! patterned after graph-flow's typed-task execution — implemented natively
//! so that:
//! * every node payload is a protobuf message (graffy-proto),
//! * every observable lands in the append-only journal as it happens,
//! * guarded back-edges consume budget on every traversal,
//! * routing decisions (including quality-gate escalations) are journaled,
//! * the no-raw-execution invariant is enforced at the only place model
//!   calls can originate.

/// Node execution states surfaced to the TUI while a run is live.
/// Wire form: `graffy.journal.v1.NodeState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Skipped,
    AwaitingApproval,
    Cancelled,
}
