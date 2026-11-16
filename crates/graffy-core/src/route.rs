//! Prompt-to-graph routing (ADR-0012): the deterministic trigger-phrase
//! matcher and the `router` node that journals its choice.
//!
//! MCW reading: routing is a Selection-stage act on the user's intent. The
//! decision is recorded with its exact reason so a mis-route is auditable and
//! the constraint stays legible to the human (anti-Constraint-Opacity).
//!
//! There is no score and no threshold. A graph is eligible only if its spec
//! declares `route_on` phrases; exactly one matching graph is selected;
//! anything else falls back to the floor graph.

use crate::error::ExecError;
use crate::exec::{NodeBehavior, NodeCtx, NodeExecutionResult, NodeOutput};
use crate::journal::wire;
use crate::journal::wire::run_event::Event;

/// A registered graph that opted in to automatic routing.
#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub graph_id: String,
    pub name: String,
    pub route_on: Vec<String>,
}

/// Lowercased words joined by single spaces; punctuation is a word break.
pub fn normalize(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the phrase matcher concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Exactly one candidate matched, on this declared phrase.
    Graph {
        index: usize,
        phrase: String,
    },
    NoMatch,
    /// Several candidates matched; choosing between them would be a guess.
    Ambiguous(Vec<usize>),
}

/// Match whole words, case-insensitively: "decide" does not fire on "undecided".
pub fn select(prompt: &str, candidates: &[RouteCandidate]) -> Selection {
    let haystack = format!(" {} ", normalize(prompt));
    let hits: Vec<(usize, &String)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            candidate
                .route_on
                .iter()
                .find(|phrase| {
                    let needle = normalize(phrase);
                    !needle.is_empty() && haystack.contains(&format!(" {needle} "))
                })
                .map(|phrase| (index, phrase))
        })
        .collect();
    match hits.as_slice() {
        [] => Selection::NoMatch,
        [(index, phrase)] => Selection::Graph {
            index: *index,
            phrase: (*phrase).clone(),
        },
        many => Selection::Ambiguous(many.iter().map(|(index, _)| *index).collect()),
    }
}

/// The `router` node kind. Not in the default registry: a frontend registers
/// it with the catalog of opted-in graphs, so a graph with a `router` node
/// fails loudly where no catalog was supplied.
pub struct RouterNode {
    pub candidates: Vec<RouteCandidate>,
    /// The graph that runs when nothing is selected (the conversation floor).
    pub floor: RouteCandidate,
}

#[async_trait::async_trait]
impl NodeBehavior for RouterNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let ids = |indexes: &[usize]| -> Vec<String> {
            indexes
                .iter()
                .map(|i| self.candidates[*i].graph_id.clone())
                .collect()
        };
        let mut decision = wire::GraphRoutingDecision {
            node_id: ctx.node.id.clone(),
            router: "phrases".into(),
            considered: self.candidates.iter().map(|c| c.graph_id.clone()).collect(),
            ..Default::default()
        };
        let chosen = match select(ctx.prompt, &self.candidates) {
            Selection::Graph { index, phrase } => {
                let chosen = &self.candidates[index];
                decision.reason = format!(
                    "running '{}' because your message says \"{phrase}\"",
                    chosen.name
                );
                decision.matched = vec![chosen.graph_id.clone()];
                decision.matched_phrase = phrase;
                chosen
            }
            Selection::NoMatch => {
                decision.fallback = true;
                decision.reason = format!(
                    "running '{}': no trigger phrase of a routable graph appears in your message",
                    self.floor.name
                );
                &self.floor
            }
            Selection::Ambiguous(indexes) => {
                decision.fallback = true;
                decision.matched = ids(&indexes);
                decision.reason = format!(
                    "running '{}': your message matches more than one graph ({}), so none was chosen",
                    self.floor.name,
                    decision.matched.join(", ")
                );
                &self.floor
            }
        };
        decision.chosen_graph_id = chosen.graph_id.clone();
        decision.chosen_graph_name = chosen.name.clone();
        let mut out = NodeOutput::default();
        out.guard_facts
            .insert("routed_graph".to_owned(), chosen.graph_id.clone());
        ctx.records.push(Event::GraphRouting(decision));
        Ok(NodeExecutionResult::Continue(out))
    }
}

/// The routing decision a finished routing run recorded.
pub fn decision_from(events: &[wire::RunEvent]) -> Option<wire::GraphRoutingDecision> {
    events.iter().find_map(|e| match &e.event {
        Some(Event::GraphRouting(decision)) => Some(decision.clone()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<RouteCandidate> {
        vec![
            RouteCandidate {
                graph_id: "t.decide".into(),
                name: "Decide".into(),
                route_on: vec!["pros and cons".into(), "decide".into()],
            },
            RouteCandidate {
                graph_id: "t.steps".into(),
                name: "Steps".into(),
                route_on: vec!["step by step".into()],
            },
        ]
    }

    #[test]
    fn exactly_one_match_selects_and_names_the_phrase() {
        assert_eq!(
            select("What are the PROS, and cons?", &candidates()),
            Selection::Graph {
                index: 0,
                phrase: "pros and cons".into()
            }
        );
    }

    #[test]
    fn whole_words_only_and_no_match_is_not_a_guess() {
        assert_eq!(
            select("I am undecided today", &candidates()),
            Selection::NoMatch
        );
        assert_eq!(select("", &candidates()), Selection::NoMatch);
        assert_eq!(select("hello", &[]), Selection::NoMatch);
    }

    #[test]
    fn matching_two_graphs_is_ambiguous_never_first_wins() {
        assert_eq!(
            select("Help me decide, step by step", &candidates()),
            Selection::Ambiguous(vec![0, 1])
        );
    }

    #[test]
    fn a_phrase_with_no_words_is_rejected_at_compile_time() {
        let mut spec = crate::spec::GraphSpec::from_toml_str(
            r#"
            [graph]
            id = "t.route"
            name = "Route"
            version = "0.0.1"
            route_on = ["pros and cons"]

            [[node]]
            id = "a"
            kind = "intake"
            "#,
        )
        .unwrap();
        assert!(crate::graph::CompiledGraph::compile(&spec).is_ok());
        // "?!" normalizes to nothing and would match every prompt.
        spec.graph.route_on.push(" ?! ".into());
        assert!(crate::graph::CompiledGraph::compile(&spec).is_err());
        // Absent phrases stay out of the serialized spec (shared specs are stable).
        spec.graph.route_on.clear();
        assert!(!spec.to_toml_string().unwrap().contains("route_on"));
    }
}
