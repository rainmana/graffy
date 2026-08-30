//! Provider layer (ADR-0005).
//!
//! rig-core carries the actual clients: Anthropic, OpenAI, Ollama — plus any
//! OpenAI-compatible endpoint (Venice.AI, OpenRouter, LM Studio) via custom
//! base URLs. graffy wraps rig behind a registry that attaches capability
//! tiers and cost metadata, because routing ladders speak in tiers
//! ("fast" / "balanced" / "frontier"), not vendor names.
//!
//! Invariant enforcement point: nothing in this crate is reachable except
//! through an executor-scheduled graph node. There is deliberately no
//! "just complete this prompt" convenience function.
//!
//! Phase 1 milestone M2 wires completion + streaming; this crate currently
//! pins the dependency and the routing vocabulary.

pub use rig;

/// A concrete, routable model target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    /// Provider key: "anthropic" | "openai" | "ollama" | "openai-compat:<name>".
    pub provider: String,
    /// Provider-native model name.
    pub model: String,
    /// Capability tier this model serves in routing ladders.
    pub tier: String,
}
