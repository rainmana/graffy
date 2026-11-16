//! Prompt-to-graph routing (ADR-0012): the deterministic trigger-phrase
//! matcher and the `router` node that journals its choice.
//!
//! MCW reading: routing is a Selection-stage act on the user's intent. The
//! decision is recorded with its exact reason so a mis-route is auditable and
//! the constraint stays legible to the human (anti-Constraint-Opacity).
//!
//! A graph is eligible only if its spec declares `route_on` phrases.
//!
//! * **Phrases** (default): no score and no threshold. Exactly one matching
//!   graph is selected; anything else falls back to the floor graph.
//! * **Decider** (optional): a small decision model picks among the same
//!   opted-in graphs. [`Decider`] is the only doorway to such a model. What it
//!   reports — choice, confidence, probabilities — is journaled as its own
//!   report, and a pick below the human's configured threshold is not followed.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::ExecError;
use crate::exec::{NodeBehavior, NodeCtx, NodeExecutionResult, NodeOutput, sha256_hex};
use crate::journal::wire;
use crate::journal::wire::run_event::Event;

/// A registered graph that opted in to automatic routing.
#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub graph_id: String,
    pub name: String,
    pub route_on: Vec<String>,
    /// When this graph is the right choice, in one sentence — the criterion a
    /// decision model is given for it.
    pub route_when: String,
}

/// What a decision model answered.
#[derive(Debug, Clone)]
pub struct DeciderAnswer {
    pub choice: String,
    /// The model's own confidence in `choice`, 0–1.
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    /// The concrete model that answered, as the service reported it.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
}

/// A decision model could not be consulted, or answered unusably.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct DeciderError(pub String);

/// The only doorway to a decision model (a "system one" model that picks
/// among supplied choices instead of generating text). Implemented outside
/// core, like [`crate::exec::ModelInvoker`].
#[async_trait::async_trait]
pub trait Decider: Send + Sync {
    /// Where content is sent to be decided (`host[:port]`) — disclosed to the
    /// human and journaled, because the prompt leaves this process.
    fn endpoint(&self) -> &str;

    /// Choose one of `choices` (name → when it applies) for `content`.
    async fn choose(
        &self,
        content: &str,
        instructions: &str,
        choices: &BTreeMap<String, String>,
    ) -> Result<DeciderAnswer, DeciderError>;
}

/// Decision-model routing: the decider and the human's acceptance threshold.
#[derive(Clone)]
pub struct DeciderRouting {
    pub decider: Arc<dyn Decider>,
    /// A pick whose reported confidence is below this is not followed. graffy
    /// ships no default: the human sets it.
    pub min_confidence: f64,
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
    /// `Some`: a decision model chooses. `None`: trigger phrases decide.
    pub decider: Option<DeciderRouting>,
}

const DECIDER_INSTRUCTIONS: &str = "Which workflow best fits this request? Choose the conversation option unless another clearly fits.";

impl RouterNode {
    /// Ask the decision model. Its pick is followed only when it names an
    /// opted-in graph with at least the configured confidence.
    async fn decide(
        &self,
        ctx: &mut NodeCtx<'_>,
        routing: &DeciderRouting,
        decision: &mut wire::GraphRoutingDecision,
    ) -> Result<RouteCandidate, ExecError> {
        let endpoint = routing.decider.endpoint().to_owned();
        decision.router = "decider".into();
        decision.min_confidence = Some(routing.min_confidence);
        decision.decided_at_endpoint = endpoint.clone();
        if self.candidates.is_empty() {
            // Nothing to choose between: do not send the prompt anywhere.
            decision.fallback = true;
            decision.reason = format!(
                "running '{}': no registered graph has opted in to routing, so the decision model was not consulted",
                self.floor.name
            );
            return Ok(self.floor.clone());
        }
        let mut choices: BTreeMap<String, String> = self
            .candidates
            .iter()
            .map(|c| (c.graph_id.clone(), c.route_when.clone()))
            .collect();
        choices.insert(self.floor.graph_id.clone(), self.floor.route_when.clone());
        let failed = |reason: String| ExecError::NodeFailed(ctx.node.id.clone(), reason);
        let answer = routing
            .decider
            .choose(ctx.prompt, DECIDER_INSTRUCTIONS, &choices)
            .await
            .map_err(|err| failed(format!("decision model at {endpoint}: {err}")))?;
        // The call is a model call: journaled, and counted against the budget.
        ctx.records.push(Event::ModelCall(wire::ModelCallRecord {
            node_id: ctx.node.id.clone(),
            provider: endpoint.clone(),
            model: answer.model.clone(),
            purpose: "route".into(),
            input_tokens: answer.input_tokens,
            output_tokens: answer.output_tokens,
            cost_usd: 0.0,
            latency_ms: answer.latency_ms,
            streamed: false,
            request_sha256: sha256_hex(
                format!(
                    "{DECIDER_INSTRUCTIONS}\n---\n{}\n---\n{choices:?}",
                    ctx.prompt
                )
                .as_bytes(),
            ),
        }));
        if !(0.0..=1.0).contains(&answer.confidence) {
            return Err(failed(format!(
                "decision model at {endpoint} reported confidence {} outside 0–1",
                answer.confidence
            )));
        }
        decision.confidence = Some(answer.confidence);
        decision.probabilities = answer.probabilities.clone().into_iter().collect();
        decision.decided_by_model = answer.model.clone();
        let who = format!("decision model {} at {endpoint}", answer.model);
        if answer.choice == self.floor.graph_id {
            decision.fallback = true;
            decision.reason = format!(
                "running '{}': {who} chose plain conversation (confidence {:.2})",
                self.floor.name, answer.confidence
            );
            return Ok(self.floor.clone());
        }
        let Some(picked) = self.candidates.iter().find(|c| c.graph_id == answer.choice) else {
            return Err(failed(format!(
                "{who} answered '{}', which is not one of the offered graphs",
                answer.choice
            )));
        };
        decision.matched = vec![picked.graph_id.clone()];
        if answer.confidence < routing.min_confidence {
            decision.fallback = true;
            decision.reason = format!(
                "running '{}': {who} leaned to '{}' with confidence {:.2}, below your threshold {:.2}",
                self.floor.name, picked.name, answer.confidence, routing.min_confidence
            );
            return Ok(self.floor.clone());
        }
        decision.reason = format!(
            "running '{}': {who} chose it with confidence {:.2} (your threshold {:.2})",
            picked.name, answer.confidence, routing.min_confidence
        );
        Ok(picked.clone())
    }

    /// Deterministic trigger-phrase routing.
    fn match_phrases(
        &self,
        prompt: &str,
        decision: &mut wire::GraphRoutingDecision,
    ) -> RouteCandidate {
        decision.router = "phrases".into();
        match select(prompt, &self.candidates) {
            Selection::Graph { index, phrase } => {
                let chosen = &self.candidates[index];
                decision.reason = format!(
                    "running '{}' because your message says \"{phrase}\"",
                    chosen.name
                );
                decision.matched = vec![chosen.graph_id.clone()];
                decision.matched_phrase = phrase;
                chosen.clone()
            }
            Selection::NoMatch => {
                decision.fallback = true;
                decision.reason = format!(
                    "running '{}': no trigger phrase of a routable graph appears in your message",
                    self.floor.name
                );
                self.floor.clone()
            }
            Selection::Ambiguous(indexes) => {
                decision.fallback = true;
                decision.matched = indexes
                    .iter()
                    .map(|i| self.candidates[*i].graph_id.clone())
                    .collect();
                decision.reason = format!(
                    "running '{}': your message matches more than one graph ({}), so none was chosen",
                    self.floor.name,
                    decision.matched.join(", ")
                );
                self.floor.clone()
            }
        }
    }
}

#[async_trait::async_trait]
impl NodeBehavior for RouterNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let mut decision = wire::GraphRoutingDecision {
            node_id: ctx.node.id.clone(),
            considered: self.candidates.iter().map(|c| c.graph_id.clone()).collect(),
            ..Default::default()
        };
        let chosen = match &self.decider {
            Some(routing) => self.decide(ctx, routing, &mut decision).await?,
            None => self.match_phrases(ctx.prompt, &mut decision),
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
                route_when: String::new(),
            },
            RouteCandidate {
                graph_id: "t.steps".into(),
                name: "Steps".into(),
                route_on: vec!["step by step".into()],
                route_when: String::new(),
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
