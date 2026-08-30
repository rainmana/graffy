//! Multi-agent conversation nodes (ADR-0005).
//!
//! A conversation node (`kind = "agent.conversation"`) hosts several model
//! personas — a peer-review panel, a red team, a rules-lawyer table — whose
//! transcript is journaled turn by turn and whose conclusions must attach
//! evidence artifacts before anything leaves the node.
//!
//! Orchestration rides on the `rigs` crate (MIT, built atop rig). Version
//! 0.0.8 is early; the integration surface is kept thin so it can be
//! replaced without touching node semantics (see ADR-0005 "Risks").
//!
//! Phase 1 milestone M2/M3 implements the node; this crate currently pins
//! the dependency.

pub use rigs;
