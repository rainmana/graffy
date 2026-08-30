//! Compiled graphs — petgraph-backed topology the executor walks.
//!
//! Compilation enforces the structural half of graffy's law: a cycle is
//! lawful only when every back-edge carries a `when` guard (the runtime half
//! — budgets — is enforced by the executor).

use std::collections::HashMap;

use petgraph::graph::DiGraph;

use crate::error::CompileError;
use crate::spec::GraphSpec;

/// Node payload of the compiled topology (Phase 1 M2 attaches the resolved
/// node implementation and routing bindings here).
#[derive(Debug, Clone)]
pub struct CompiledNode {
    pub id: String,
    pub kind: String,
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
}

impl CompiledGraph {
    /// Compile a TOML spec into an executable topology.
    pub fn compile(spec: &GraphSpec) -> Result<Self, CompileError> {
        let mut topology = DiGraph::new();
        let mut index = HashMap::new();

        for node in &spec.nodes {
            if index.contains_key(&node.id) {
                return Err(CompileError::DuplicateNode(node.id.clone()));
            }
            let ix = topology.add_node(CompiledNode {
                id: node.id.clone(),
                kind: node.kind.clone(),
            });
            index.insert(node.id.clone(), ix);
        }

        for edge in &spec.edges {
            let from = index
                .get(&edge.from)
                .ok_or_else(|| CompileError::UnknownNode(edge.from.clone()))?;
            let to = index
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

        Ok(Self { topology })
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
    fn guarded_cycle_compiles() {
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
        assert!(CompiledGraph::compile(&s).is_ok());
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
}
