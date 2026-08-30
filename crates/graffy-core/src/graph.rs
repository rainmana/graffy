//! Compiled graphs — the petgraph-backed topology the executor walks.
//!
//! Compilation enforces the structural half of graffy's law: a cycle is
//! lawful only when every back-edge carries a `when` guard (the runtime half
//! — budgets and per-node visit caps — is enforced by the executor).

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::error::CompileError;
use crate::spec::GraphSpec;

/// Node payload of the compiled topology: everything the executor needs to
/// dispatch the node, carried over from the TOML spec.
#[derive(Debug, Clone)]
pub struct CompiledNode {
    pub id: String,
    pub kind: String,
    pub description: String,
    /// Capability tier requested from the routing ladder, if any.
    pub model_tier: Option<String>,
    /// Kind-specific parameters, passed through to the node behavior.
    pub params: toml::Table,
}

/// Edge payload: the optional guard expression.
#[derive(Debug, Clone)]
pub struct CompiledEdge {
    pub guard: Option<String>,
}

/// A validated, executable topology.
#[derive(Debug)]
pub struct CompiledGraph {
    pub topology: DiGraph<CompiledNode, CompiledEdge>,
    index_by_id: HashMap<String, NodeIndex>,
}

impl CompiledGraph {
    /// Compile a TOML spec into an executable topology.
    pub fn compile(spec: &GraphSpec) -> Result<Self, CompileError> {
        let mut topology = DiGraph::new();
        let mut index_by_id = HashMap::new();

        for node in &spec.nodes {
            if index_by_id.contains_key(&node.id) {
                return Err(CompileError::DuplicateNode(node.id.clone()));
            }
            let ix = topology.add_node(CompiledNode {
                id: node.id.clone(),
                kind: node.kind.clone(),
                description: node.description.clone(),
                model_tier: node.model_tier.clone(),
                params: node.params.clone(),
            });
            index_by_id.insert(node.id.clone(), ix);
        }

        for edge in &spec.edges {
            let from = index_by_id
                .get(&edge.from)
                .ok_or_else(|| CompileError::UnknownNode(edge.from.clone()))?;
            let to = index_by_id
                .get(&edge.to)
                .ok_or_else(|| CompileError::UnknownNode(edge.to.clone()))?;
            topology.add_edge(
                *from,
                *to,
                CompiledEdge {
                    guard: edge.when.clone(),
                },
            );
        }

        // Structural law: drop guarded edges; whatever cycles remain are
        // unguarded and therefore unlawful.
        let unguarded: DiGraph<(), ()> = topology.filter_map(
            |_, _| Some(()),
            |_, edge| if edge.guard.is_none() { Some(()) } else { None },
        );
        if petgraph::algo::is_cyclic_directed(&unguarded) {
            return Err(CompileError::UnguardedCycle);
        }

        let compiled = Self {
            topology,
            index_by_id,
        };
        if compiled.topology.node_count() > 0 && compiled.entry_nodes().is_empty() {
            return Err(CompileError::NoEntryNode);
        }
        Ok(compiled)
    }

    /// Nodes where execution begins. Strictly: nodes with no incoming edges
    /// at all. Only when that set is empty (pure-cycle graphs) does the
    /// fallback apply: nodes whose incoming edges are all guarded — those
    /// guards are re-entry paths (e.g. a revise loop), not initial
    /// dependencies. A guarded *forward* edge (verify → respond) must never
    /// make its target an entry, or it would run before its inputs exist.
    pub fn entry_nodes(&self) -> Vec<NodeIndex> {
        let strict: Vec<NodeIndex> = self
            .topology
            .node_indices()
            .filter(|ix| {
                self.topology
                    .edges_directed(*ix, Direction::Incoming)
                    .next()
                    .is_none()
            })
            .collect();
        if !strict.is_empty() {
            return strict;
        }
        self.topology
            .node_indices()
            .filter(|ix| {
                !self
                    .topology
                    .edges_directed(*ix, Direction::Incoming)
                    .any(|e| e.weight().guard.is_none())
            })
            .collect()
    }

    /// Outgoing edges of a node as `(target, guard)` pairs, in spec order.
    pub fn successors(&self, ix: NodeIndex) -> Vec<(NodeIndex, Option<String>)> {
        let mut out: Vec<(NodeIndex, Option<String>)> = self
            .topology
            .edges_directed(ix, Direction::Outgoing)
            .map(|e| (e.target(), e.weight().guard.clone()))
            .collect();
        // petgraph iterates most-recent-first; restore spec order.
        out.reverse();
        out
    }

    pub fn node(&self, ix: NodeIndex) -> &CompiledNode {
        &self.topology[ix]
    }

    pub fn find(&self, node_id: &str) -> Option<NodeIndex> {
        self.index_by_id.get(node_id).copied()
    }

    pub fn node_count(&self) -> usize {
        self.topology.node_count()
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::CompiledGraph;
    use crate::spec::GraphSpec;

    fn spec(toml_body: &str) -> GraphSpec {
        GraphSpec::from_toml_str(toml_body).expect("test spec should parse")
    }

    #[test]
    fn guarded_cycle_compiles_and_guarded_reentry_keeps_entry_status() {
        let s = spec(
            r#"
            [graph]
            id = "t.cycle"
            name = "Guarded cycle"
            version = "0.0.1"

            [[node]]
            id = "draft"
            kind = "model"

            [[node]]
            id = "verify"
            kind = "verify"

            [[edge]]
            from = "draft"
            to = "verify"

            [[edge]]
            from = "verify"
            to = "draft"
            when = "verdict == 'revise'"
            "#,
        );
        let g = CompiledGraph::compile(&s).expect("guarded cycle is lawful");
        let entries = g.entry_nodes();
        assert_eq!(entries.len(), 1);
        assert_eq!(g.node(entries[0]).id, "draft");
    }

    #[test]
    fn unguarded_cycle_is_rejected() {
        let s = spec(
            r#"
            [graph]
            id = "t.badcycle"
            name = "Unguarded cycle"
            version = "0.0.1"

            [[node]]
            id = "a"
            kind = "model"

            [[node]]
            id = "b"
            kind = "model"

            [[edge]]
            from = "a"
            to = "b"

            [[edge]]
            from = "b"
            to = "a"
            "#,
        );
        assert!(matches!(
            CompiledGraph::compile(&s),
            Err(crate::error::CompileError::UnguardedCycle)
        ));
    }

    #[test]
    fn unknown_edge_target_is_rejected() {
        let s = spec(
            r#"
            [graph]
            id = "t.unknown"
            name = "Unknown node"
            version = "0.0.1"

            [[node]]
            id = "a"
            kind = "model"

            [[edge]]
            from = "a"
            to = "ghost"
            "#,
        );
        assert!(matches!(
            CompiledGraph::compile(&s),
            Err(crate::error::CompileError::UnknownNode(name)) if name == "ghost"
        ));
    }

    #[test]
    fn successors_preserve_spec_order_and_guards() {
        let s = spec(
            r#"
            [graph]
            id = "t.succ"
            name = "Successors"
            version = "0.0.1"

            [[node]]
            id = "verify"
            kind = "verify"

            [[node]]
            id = "respond"
            kind = "respond"

            [[node]]
            id = "draft"
            kind = "model"

            [[edge]]
            from = "verify"
            to = "respond"
            when = "verdict == 'pass'"

            [[edge]]
            from = "verify"
            to = "draft"
            when = "verdict == 'revise'"
            "#,
        );
        let g = CompiledGraph::compile(&s).unwrap();
        let verify = g.find("verify").unwrap();
        let succ = g.successors(verify);
        assert_eq!(succ.len(), 2);
        assert_eq!(g.node(succ[0].0).id, "respond");
        assert_eq!(succ[0].1.as_deref(), Some("verdict == 'pass'"));
        assert_eq!(g.node(succ[1].0).id, "draft");
    }
}
