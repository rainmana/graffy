//! Built-in graph library.
//!
//! Ships as TOML specs (the same format users share) so built-ins are not
//! privileged: export, diff, and fork them like any other graph. All are
//! also present under `graphs/` in the repo.
//!
//! Reasoning templates follow the Clear-Thought MCP server lineage (MIT —
//! see NOTICE.md), re-expressed as graffy graphs with critique gates and
//! guarded revision loops. Phase 2 expands the catalog (mental models, OODA,
//! Ulysses protocol, collaborative reasoning, …) and adds the skill-adoption
//! flows (auto / guided / collaborative).

pub mod graphify;

/// The default conversation floor graph — what plain chat runs on.
pub const DEFAULT_CONVERSATION_TOML: &str =
    include_str!("../../../graphs/conversation.default.toml");

/// Conversation floor + human release gate (PauseForDisambiguation demo).
pub const GATED_CONVERSATION_TOML: &str = include_str!("../../../graphs/conversation.gated.toml");

/// Sequential Thinking: plan → think → critique (guarded revise loop) → synthesize.
pub const SEQUENTIAL_THINKING_TOML: &str =
    include_str!("../../../graphs/reasoning.sequential-thinking.toml");

/// Decision Framework: frame → evaluate → challenge (guarded loop) → decide.
pub const DECISION_FRAMEWORK_TOML: &str =
    include_str!("../../../graphs/reasoning.decision-framework.toml");

/// The prompt router (ADR-0012): `intake → route`. Its `router` node needs a
/// catalog of opted-in graphs, supplied by the frontend that runs it.
pub const ROUTER_TOML: &str = include_str!("../../../graphs/routing.router.toml");

/// Every built-in spec shipped with this build, `(id, toml)`.
pub fn builtin_specs() -> [(&'static str, &'static str); 5] {
    [
        ("graffy.builtin.router", ROUTER_TOML),
        ("graffy.builtin.conversation", DEFAULT_CONVERSATION_TOML),
        ("graffy.builtin.conversation.gated", GATED_CONVERSATION_TOML),
        (
            "graffy.builtin.reasoning.sequential-thinking",
            SEQUENTIAL_THINKING_TOML,
        ),
        (
            "graffy.builtin.reasoning.decision-framework",
            DECISION_FRAMEWORK_TOML,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use graffy_core::exec::{
        ApprovalHandler, ApprovalOutcome, AutoApprove, Executor, OfflineEcho, RunInput,
    };
    use graffy_core::graph::CompiledGraph;
    use graffy_core::journal::{JournalReader, summarize, wire};
    use graffy_core::spec::GraphSpec;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "graffy-graphs-test-{tag}-{}",
            graffy_core::id::RunId::generate()
        ))
    }

    #[test]
    fn every_builtin_parses_compiles_and_matches_its_declared_id() {
        for (id, toml) in super::builtin_specs() {
            let spec = GraphSpec::from_toml_str(toml)
                .unwrap_or_else(|e| panic!("builtin '{id}' must parse: {e}"));
            assert_eq!(spec.graph.id, id);
            let compiled = CompiledGraph::compile(&spec)
                .unwrap_or_else(|e| panic!("builtin '{id}' must compile: {e}"));
            assert_eq!(compiled.node_count(), spec.nodes.len());
            assert_eq!(
                spec.policy.routing.on_quality_fail.as_deref(),
                Some("escalate"),
                "builtin '{id}' must escalate on quality failure"
            );
        }
    }

    #[tokio::test]
    async fn sequential_thinking_executes_offline_end_to_end() {
        let spec = GraphSpec::from_toml_str(super::SEQUENTIAL_THINKING_TOML).unwrap();
        let path = temp_path("seq");
        let outcome = Executor::default()
            .run(
                &spec,
                super::SEQUENTIAL_THINKING_TOML,
                RunInput {
                    prompt: "How should I structure a falsifiable experiment?".into(),
                    session_id: None,
                    feedback: Vec::new(),
                    ..Default::default()
                },
                &path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await
            .expect("sequential thinking must run offline");
        assert_eq!(outcome.status, wire::RunStatus::Succeeded);
        let summary = summarize(&JournalReader::read_all(&path).unwrap());
        assert!(summary.model_calls >= 4, "got {}", summary.model_calls);
        assert!(summary.iu_count >= 5);
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn decision_framework_executes_offline_end_to_end() {
        let spec = GraphSpec::from_toml_str(super::DECISION_FRAMEWORK_TOML).unwrap();
        let path = temp_path("dec");
        let outcome = Executor::default()
            .run(
                &spec,
                super::DECISION_FRAMEWORK_TOML,
                RunInput {
                    prompt: "Pick a serialization format for run journals.".into(),
                    session_id: None,
                    feedback: Vec::new(),
                    ..Default::default()
                },
                &path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await
            .expect("decision framework must run offline");
        assert_eq!(outcome.status, wire::RunStatus::Succeeded);
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn gated_conversation_releases_on_approval() {
        let spec = GraphSpec::from_toml_str(super::GATED_CONVERSATION_TOML).unwrap();
        let path = temp_path("gate-approve");
        let outcome = Executor::default()
            .run(
                &spec,
                super::GATED_CONVERSATION_TOML,
                RunInput {
                    prompt: "gate me".into(),
                    session_id: None,
                    feedback: Vec::new(),
                    ..Default::default()
                },
                &path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await
            .expect("gated run with approval must succeed");
        assert_eq!(outcome.status, wire::RunStatus::Succeeded);
        assert!(outcome.final_text.is_some());

        let events = JournalReader::read_all(&path).unwrap();
        let mut decisions = Vec::new();
        for frame in &events {
            if let Some(wire::run_event::Event::Approval(a)) = &frame.event {
                decisions.push((a.decision, a.decided_by.clone()));
            }
        }
        assert_eq!(decisions.len(), 2, "pending then resolved");
        assert_eq!(decisions[0].0, wire::ApprovalDecision::Pending as i32);
        assert_eq!(decisions[1].0, wire::ApprovalDecision::Approved as i32);
        assert_eq!(decisions[1].1, "auto-approve", "honest attribution");
        std::fs::remove_file(&path).ok();
    }

    struct RejectAll;

    #[async_trait::async_trait]
    impl ApprovalHandler for RejectAll {
        fn describe(&self) -> &'static str {
            "test-reject-all"
        }
        async fn resolve(&self, _node_id: &str, _question: &str) -> ApprovalOutcome {
            ApprovalOutcome::Rejected
        }
    }

    #[tokio::test]
    async fn gated_conversation_cancels_on_rejection() {
        let spec = GraphSpec::from_toml_str(super::GATED_CONVERSATION_TOML).unwrap();
        let path = temp_path("gate-reject");
        let outcome = Executor::default()
            .run(
                &spec,
                super::GATED_CONVERSATION_TOML,
                RunInput {
                    prompt: "gate me".into(),
                    session_id: None,
                    feedback: Vec::new(),
                    ..Default::default()
                },
                &path,
                &OfflineEcho,
                &RejectAll,
            )
            .await
            .expect("rejection is a clean cancellation, not an error");
        assert_eq!(outcome.status, wire::RunStatus::Cancelled);
        assert!(
            outcome.final_text.is_none(),
            "nothing released past the gate"
        );

        let events = JournalReader::read_all(&path).unwrap();
        let rejected = events.iter().any(|f| {
            matches!(
                &f.event,
                Some(wire::run_event::Event::Approval(a))
                    if a.decision == wire::ApprovalDecision::Rejected as i32
                        && a.decided_by == "test-reject-all"
            )
        });
        assert!(
            rejected,
            "journal must carry the rejection with attribution"
        );
        std::fs::remove_file(&path).ok();
    }

    fn router_executor(candidates: Vec<graffy_core::route::RouteCandidate>) -> Executor {
        let mut registry = graffy_core::exec::NodeRegistry::with_defaults();
        registry.register(
            "router",
            std::sync::Arc::new(graffy_core::route::RouterNode {
                candidates,
                floor: graffy_core::route::RouteCandidate {
                    graph_id: "graffy.builtin.conversation".into(),
                    name: "Default Conversation".into(),
                    route_on: Vec::new(),
                },
            }),
        );
        Executor {
            registry,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn router_graph_journals_its_choice_and_is_not_a_conversation_turn() {
        use wire::run_event::Event;
        let spec = GraphSpec::from_toml_str(super::ROUTER_TOML).unwrap();
        // Candidates are exactly the built-ins that opted in.
        let candidates: Vec<_> = super::builtin_specs()
            .iter()
            .map(|(_, toml)| GraphSpec::from_toml_str(toml).unwrap().graph)
            .filter(|meta| !meta.route_on.is_empty())
            .map(|meta| graffy_core::route::RouteCandidate {
                graph_id: meta.id,
                name: meta.name,
                route_on: meta.route_on,
            })
            .collect();
        assert!(
            candidates
                .iter()
                .all(|c| c.graph_id != "graffy.builtin.router"
                    && !c.graph_id.starts_with("graffy.builtin.conversation")),
            "the router and the conversation floors stay explicit-only"
        );
        let path = temp_path("router");
        let outcome = router_executor(candidates)
            .run(
                &spec,
                super::ROUTER_TOML,
                RunInput {
                    prompt: "What are the pros and cons of renting?".into(),
                    run_kind: wire::RunKind::Routing as i32,
                    ..Default::default()
                },
                &path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await
            .unwrap();
        assert_eq!(outcome.status, wire::RunStatus::Succeeded);
        assert!(
            outcome.final_text.is_none(),
            "a routing run answers nothing"
        );
        let events = JournalReader::read_all(&path).unwrap();
        let decision = graffy_core::route::decision_from(&events).unwrap();
        assert_eq!(
            decision.chosen_graph_id,
            "graffy.builtin.reasoning.decision-framework"
        );
        assert_eq!(decision.matched_phrase, "pros and cons");
        assert!(!decision.fallback);
        assert!(decision.reason.contains("pros and cons"));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e.event, Some(Event::BoundaryEvent(_)))),
            "no boundary contribution is recorded for a routing run"
        );
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn router_graph_fails_loudly_without_a_catalog() {
        // The default registry has no `router` kind: nothing pretends to route.
        let spec = GraphSpec::from_toml_str(super::ROUTER_TOML).unwrap();
        let path = temp_path("router-bare");
        let result = Executor::default()
            .run(
                &spec,
                super::ROUTER_TOML,
                RunInput {
                    prompt: "pros and cons".into(),
                    run_kind: wire::RunKind::Routing as i32,
                    ..Default::default()
                },
                &path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await;
        let failed = match result {
            Err(_) => true,
            Ok(outcome) => outcome.status != wire::RunStatus::Succeeded,
        };
        assert!(failed);
        std::fs::remove_file(&path).ok();
    }
}
