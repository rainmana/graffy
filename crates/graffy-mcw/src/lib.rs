//! MCW runtime (ADR-0004).
//!
//! The types live in graffy-proto (`graffy.mcw.v1`); this crate gives them
//! behavior: the per-session IU ledger, failure-mode detectors that watch the
//! journal stream, the five repair operations as executable graph node kinds,
//! and H/R/D/M sampling.
//!
//! Canon: Meta-Context Window (MCW) Framework, W. Alec Akin, Apache-2.0 —
//! <https://github.com/rainmana/mcw-framework> (attribution in NOTICE.md).
//! Detectors implement the taxonomy's early signals; Phase 3 makes them live.

pub use graffy_proto::mcw::v1 as wire;

/// The five repair operations as built-in graph node kinds — usable in any
/// TOML spec, required by some built-in graphs.
pub const REPAIR_NODE_KINDS: &[&str] = &[
    "repair.regrounding",
    "repair.decompression",
    "repair.reweighting",
    "repair.disambiguation",
    "repair.synchronization",
];

#[cfg(test)]
mod tests {
    #[test]
    fn repair_node_kinds_cover_all_five_canonical_operations() {
        assert_eq!(super::REPAIR_NODE_KINDS.len(), 5);
        for kind in super::REPAIR_NODE_KINDS {
            assert!(kind.starts_with("repair."));
        }
    }
}
