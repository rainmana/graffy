//! Built-in graph library.
//!
//! Ships as TOML specs (same format users share) so built-ins are not
//! privileged: you can export, diff, and fork them like any other graph.
//!
//! Phase 1 seeds: the default conversation floor graph plus two
//! Clear-Thought-derived reasoning templates (sequential thinking, decision
//! framework). Phase 2 expands the catalog (mental models, OODA, Ulysses,
//! collaborative reasoning, …) and adds the skill-adoption flows
//! (auto / guided / collaborative).

/// The default conversation graph — the floor every plain chat runs on.
/// Also shipped at `graphs/conversation.default.toml` in the repo.
pub const DEFAULT_CONVERSATION_TOML: &str =
    include_str!("../../../graphs/conversation.default.toml");

#[cfg(test)]
mod tests {
    #[test]
    fn default_conversation_spec_parses_and_compiles() {
        let spec = graffy_core::spec::GraphSpec::from_toml_str(super::DEFAULT_CONVERSATION_TOML)
            .expect("default conversation TOML must parse");
        assert_eq!(spec.graph.id, "graffy.builtin.conversation");
        assert!(spec.nodes.len() >= 5, "floor graph has at least 5 stages");
        assert_eq!(spec.policy.routing.on_quality_fail.as_deref(), Some("escalate"));

        let compiled = graffy_core::graph::CompiledGraph::compile(&spec)
            .expect("default conversation graph must compile (its cycle is guarded)");
        assert_eq!(compiled.topology.node_count(), spec.nodes.len());
    }
}
