//! graffy-core — the graph substrate.
//!
//! * [`spec`] — human-readable TOML graph specifications (git-friendly,
//!   shareable durable objects).
//! * [`graph`] — compilation into a petgraph topology with cycle-guard
//!   validation (unguarded cycles are unlawful).
//! * [`exec`] — the single-process executor (Phase 1 M2): tokio-driven,
//!   graph-flow-patterned step loop, protobuf payloads, journaled everything.
//! * [`journal`] — append-only run journal (length-delimited
//!   `graffy.journal.v1.RunEvent` frames).
//! * [`id`] — ULID-backed identifiers for every durable object.

pub mod error;
pub mod exec;
pub mod graph;
pub mod id;
pub mod journal;
pub mod spec;

/// Core library version (workspace-synced).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
