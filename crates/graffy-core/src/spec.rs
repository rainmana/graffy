//! Graph specifications — the human-readable, git-friendly TOML form of a
//! durable graph object (ADR-0003, ADR-0007).
//!
//! A spec is data, not code: nodes declare *kinds* ("intake", "model",
//! "verify", "repair.regrounding", …) that the executor resolves against the
//! built-in and installed node registries at compile time.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::SpecError;

/// A complete graph specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSpec {
    pub graph: GraphMeta,
    #[serde(rename = "node", default)]
    pub nodes: Vec<NodeSpec>,
    #[serde(rename = "edge", default)]
    pub edges: Vec<EdgeSpec>,
    #[serde(default)]
    pub policy: PolicySpec,
}

/// Identity, provenance, and sharing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// One node declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    /// Node kind: `intake` | `research` | `model` | `verify` | `respond` |
    /// `router` | `approval` | `compaction` | `tool` | `agent.conversation` |
    /// `repair.*` | `custom.*`.
    pub kind: String,
    #[serde(default)]
    pub description: String,
    /// Capability tier requested from the routing ladder (e.g. "balanced").
    #[serde(default)]
    pub model_tier: Option<String>,
    /// Kind-specific parameters, passed through to the node implementation.
    #[serde(default)]
    pub params: toml::Table,
}

/// One directed edge. Guarded edges (`when`) are how cycles stay lawful.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
    /// Optional guard expression evaluated against the upstream node's
    /// output; back-edges MUST carry one (see `graph::CompiledGraph`).
    #[serde(default)]
    pub when: Option<String>,
}

/// Per-graph policies: evidence floor, budgets, routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicySpec {
    #[serde(default)]
    pub evidence: EvidencePolicy,
    #[serde(default)]
    pub budget: BudgetPolicy,
    #[serde(default)]
    pub routing: RoutingPolicy,
}

/// The epistemic floor (ADR-0008).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePolicy {
    /// "strict" — artifacts surfaced in the UI; "trace-only" — recorded but
    /// hidden (e.g. a DnD campaign still keeps receipts).
    #[serde(default = "default_evidence_mode")]
    pub mode: String,
    /// Minimum MCW evidence level (L0–L3) a claim needs to pass verify nodes.
    #[serde(default = "default_min_level")]
    pub min_level: String,
}

impl Default for EvidencePolicy {
    fn default() -> Self {
        Self {
            mode: default_evidence_mode(),
            min_level: default_min_level(),
        }
    }
}

fn default_evidence_mode() -> String {
    "strict".to_owned()
}

fn default_min_level() -> String {
    "L1".to_owned()
}

/// Hard resource ceilings; the executor halts the run when exceeded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetPolicy {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_usd: Option<f64>,
    #[serde(default)]
    pub max_seconds: Option<u64>,
}

/// Smart model routing (ADR-0005): quality failures escalate, never bounce
/// silently back to the producer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingPolicy {
    /// Escalation ladder of capability tiers, weakest first.
    #[serde(default)]
    pub ladder: Vec<String>,
    /// "escalate" | "reroute" | "halt".
    #[serde(default)]
    pub on_quality_fail: Option<String>,
}

impl PolicySpec {
    /// The standard policy for generated graphs (facades, graphified skills
    /// and prompts): the full routing ladder with escalate-on-quality-fail,
    /// and sane budgets. Without a ladder, escalation has nowhere to climb —
    /// found the hard way in a field-run TOML with `ladder = []`.
    pub fn standard() -> Self {
        Self {
            evidence: EvidencePolicy::default(),
            budget: BudgetPolicy {
                max_tokens: Some(200_000),
                max_usd: Some(1.0),
                max_seconds: Some(300),
            },
            routing: RoutingPolicy {
                ladder: vec![
                    "fast".to_owned(),
                    "balanced".to_owned(),
                    "frontier".to_owned(),
                ],
                on_quality_fail: Some("escalate".to_owned()),
            },
        }
    }
}

impl GraphSpec {
    /// Parse a spec from TOML text.
    pub fn from_toml_str(input: &str) -> Result<Self, SpecError> {
        Ok(toml::from_str(input)?)
    }

    /// Parse a spec from a TOML file.
    pub fn from_toml_path(path: &Path) -> Result<Self, SpecError> {
        let raw = std::fs::read_to_string(path)?;
        Self::from_toml_str(&raw)
    }

    /// Serialize back to shareable TOML.
    pub fn to_toml_string(&self) -> Result<String, SpecError> {
        Ok(toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::GraphSpec;

    #[test]
    fn minimal_spec_parses_with_policy_defaults() {
        let spec = GraphSpec::from_toml_str(
            r#"
            [graph]
            id = "t.min"
            name = "Minimal"
            version = "0.0.1"

            [[node]]
            id = "a"
            kind = "intake"

            [[node]]
            id = "b"
            kind = "respond"

            [[edge]]
            from = "a"
            to = "b"
            "#,
        )
        .expect("minimal spec should parse");
        assert_eq!(spec.nodes.len(), 2);
        assert_eq!(spec.policy.evidence.mode, "strict");
        assert_eq!(spec.policy.evidence.min_level, "L1");
    }
}
