//! Multi-agent conversation nodes (ADR-0005).
//!
//! A conversation node (`kind = "agent.conversation"`) hosts several model
//! personas — a peer-review panel, a red team, a rules-lawyer table — whose
//! transcript is journaled turn by turn and whose conclusions must attach
//! evidence artifacts before anything leaves the node. Lands in Phase 1
//! M2/M3 follow-up.
//!
//! ## The rigs gate
//!
//! Orchestration was slated to ride the `rigs` crate. As of 0.0.8 it pins
//! rig-core **0.11** (native-tls era) while graffy runs rig-core **0.42** —
//! two API-incompatible rigs in one binary, plus an openssl-sys build
//! dependency. Until upstream catches up, the dependency is pinned but
//! feature-gated (`--features graffy-agents/rigs-backend`), and node
//! semantics stay independent of the backend so it can slot in later
//! without touching graph specs (exactly the "thin seam" ADR-0005 planned).

#[cfg(feature = "rigs-backend")]
pub use rigs;
