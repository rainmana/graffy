//! Append-only run journal (ADR-0003).
//!
//! On disk: length-delimited `graffy.journal.v1.RunEvent` protobuf frames —
//! a self-contained, portable record of everything a run did. Mirrored into
//! libSQL (graffy-memory) for query. The TUI folds the stream for live
//! rendering and post-hoc replay; other graphs may read it as evidence.
//!
//! Phase 1 milestone M2 implements the writer/reader pair.

pub use graffy_proto::journal::v1 as wire;
