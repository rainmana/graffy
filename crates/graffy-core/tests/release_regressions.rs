use graffy_core::{
    control::{Cancellation, RemainingBudget},
    error::ModelError,
    exec::{
        ApprovalHandler, ApprovalOutcome, AutoApprove, Executor, ModelInvoker, ModelRequest,
        ModelResponse, OfflineEcho, RunInput,
    },
    journal::{JournalReader, JournalWriter, wire},
    spec::{EdgeSpec, GraphSpec, NodeSpec},
};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use wire::run_event::Event;

fn path() -> PathBuf {
    std::env::temp_dir().join(format!("graffy-release-{}.journal", ulid::Ulid::generate()))
}
fn linear(kinds: &[&str]) -> GraphSpec {
    let mut s = GraphSpec::from_toml_str(include_str!("../../../graphs/conversation.default.toml"))
        .unwrap();
    s.nodes = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| NodeSpec {
            id: format!("n{i}"),
            kind: (*k).into(),
            description: String::new(),
            model_tier: None,
            params: Default::default(),
        })
        .collect();
    s.edges = (1..kinds.len())
        .map(|i| EdgeSpec {
            from: format!("n{}", i - 1),
            to: format!("n{i}"),
            when: None,
        })
        .collect();
    s
}
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<ModelRequest>>,
    delay: bool,
    reject: bool,
}
#[async_trait::async_trait]
impl ModelInvoker for Fake {
    async fn complete(&self, r: &ModelRequest) -> Result<ModelResponse, ModelError> {
        self.calls.lock().unwrap().push(r.clone());
        if self.delay {
            std::future::pending::<()>().await;
        }
        Ok(ModelResponse {
            provider: "fake".into(),
            model: "local".into(),
            text: if r.purpose == "verify" {
                if self.reject {
                    "REVISE\nMODE: drift\nfix the claim"
                } else {
                    "PASS"
                }
            } else if r.system == "unused" {
                "UNCONSUMED_MARKER"
            } else if r.system == "source" {
                "CONSUMED_MARKER"
            } else {
                "draft answer"
            }
            .into(),
            input_tokens: 2,
            output_tokens: 3,
            cost_usd: 0.0,
            latency_ms: 0,
        })
    }
}
async fn execute(
    s: &GraphSpec,
    e: Executor,
    f: &dyn ModelInvoker,
    a: &dyn ApprovalHandler,
) -> (graffy_core::exec::RunOutcome, Vec<wire::RunEvent>) {
    let p = path();
    let out = e
        .run(
            s,
            &s.to_toml_string().unwrap(),
            RunInput {
                prompt: "ORIGINAL_REQUEST".into(),
                ..Default::default()
            },
            &p,
            f,
            a,
        )
        .await
        .unwrap();
    let events = JournalReader::read_all(&p).unwrap();
    (out, events)
}
#[tokio::test]
async fn deadline_interrupts_a_stalled_model_and_records_terminal_status() {
    let f = Fake {
        delay: true,
        ..Default::default()
    };
    let e = Executor {
        remaining_budget: RemainingBudget {
            duration: Some(Duration::from_millis(30)),
            ..Default::default()
        },
        ..Default::default()
    };
    let (out, events) = tokio::time::timeout(
        Duration::from_secs(2),
        execute(
            &linear(&["intake", "model", "respond"]),
            e,
            &f,
            &AutoApprove,
        ),
    )
    .await
    .unwrap();
    assert_eq!(out.status, wire::RunStatus::BudgetExhausted);
    assert!(out.final_text.is_none());
    assert!(events.iter().any(|e| matches!(&e.event,Some(Event::RunFinished(f)) if f.status == wire::RunStatus::BudgetExhausted as i32)));
    std::fs::remove_file(out.journal_path).unwrap();
}
#[tokio::test]
async fn cancellation_interrupts_a_stalled_model() {
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    let f = Fake {
        delay: true,
        ..Default::default()
    };
    let e = Executor {
        cancellation: cancel,
        ..Default::default()
    };
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        trigger.cancel();
    });
    let (out, _) = tokio::time::timeout(
        Duration::from_secs(2),
        execute(
            &linear(&["intake", "model", "respond"]),
            e,
            &f,
            &AutoApprove,
        ),
    )
    .await
    .unwrap();
    cancel_task.await.unwrap();
    assert_eq!(out.status, wire::RunStatus::Cancelled);
    assert!(out.final_text.is_none());
    std::fs::remove_file(out.journal_path).unwrap();
}
#[tokio::test]
async fn token_ceiling_reaches_each_model_and_remaining_budget_stops_dispatch() {
    let f = Fake::default();
    let mut s = linear(&["intake", "model", "verify", "respond"]);
    s.policy.budget.max_tokens = Some(20);
    let (out, _) = execute(&s, Executor::default(), &f, &AutoApprove).await;
    assert_eq!(out.status, wire::RunStatus::Succeeded);
    let caps: Vec<_> = f
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.max_tokens)
        .collect();
    assert_eq!(caps, vec![Some(20), Some(15)]);
    std::fs::remove_file(out.journal_path).unwrap();
    let f = Fake::default();
    let e = Executor {
        remaining_budget: RemainingBudget {
            tokens: Some(0),
            ..Default::default()
        },
        ..Default::default()
    };
    let (out, _) = execute(&s, e, &f, &AutoApprove).await;
    assert_eq!(out.status, wire::RunStatus::BudgetExhausted);
    assert!(f.calls.lock().unwrap().is_empty());
    std::fs::remove_file(out.journal_path).unwrap();
}
#[tokio::test]
async fn quality_halt_does_not_escalate_or_release() {
    let f = Fake {
        reject: true,
        ..Default::default()
    };
    let mut s = GraphSpec::from_toml_str(include_str!("../../../graphs/conversation.default.toml"))
        .unwrap();
    s.policy.routing.on_quality_fail = Some("halt".into());
    let (out, events) = execute(&s, Executor::default(), &f, &AutoApprove).await;
    assert_eq!(out.status, wire::RunStatus::Failed);
    assert!(out.final_text.is_none());
    assert_eq!(f.calls.lock().unwrap().len(), 2);
    assert!(
        !events
            .iter()
            .any(|e| matches!(&e.event,Some(Event::RoutingDecision(r)) if r.escalation_level>0))
    );
    std::fs::remove_file(out.journal_path).unwrap();
}
#[tokio::test]
async fn judge_context_is_limited_to_original_request_and_consumed_sources() {
    let f = Fake::default();
    let mut s = linear(&["intake", "model", "model", "model", "verify", "respond"]);
    s.nodes[1].params.insert("iu_role".into(), "unused".into());
    s.nodes[1].params.insert("system".into(), "unused".into());
    s.nodes[2].params.insert("iu_role".into(), "source".into());
    s.nodes[2].params.insert("system".into(), "source".into());
    s.nodes[3]
        .params
        .insert("context_roles".into(), "source".into());
    let (out, _) = execute(&s, Executor::default(), &f, &AutoApprove).await;
    let calls = f.calls.lock().unwrap();
    let judge = calls.iter().find(|r| r.purpose == "verify").unwrap();
    assert!(judge.prompt.contains("ORIGINAL_REQUEST"));
    assert!(judge.prompt.contains("CONSUMED_MARKER"));
    assert!(!judge.prompt.contains("UNCONSUMED_MARKER"));
    std::fs::remove_file(out.journal_path).unwrap();
}
struct BrokenOutput;
impl io::Write for BrokenOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
}
#[tokio::test]
async fn output_receipt_requires_successful_flush_and_is_idempotent() {
    let (out, events) = execute(
        &linear(&["intake", "model", "verify", "respond"]),
        Executor::default(),
        &Fake::default(),
        &AutoApprove,
    )
    .await;
    assert_eq!(graffy_core::boundary::project(&[events]).exchanges.len(), 0);
    assert!(
        JournalWriter::deliver_response(&out.journal_path, &out.run_id, &mut BrokenOutput).is_err()
    );
    assert_eq!(
        graffy_core::boundary::project(&[JournalReader::read_all(&out.journal_path).unwrap()])
            .exchanges
            .len(),
        0
    );
    let mut printed = Vec::new();
    JournalWriter::deliver_response(&out.journal_path, &out.run_id, &mut printed).unwrap();
    assert!(
        String::from_utf8(printed)
            .unwrap()
            .contains("reviewed response")
    );
    let mut duplicate = Vec::new();
    JournalWriter::deliver_response(&out.journal_path, &out.run_id, &mut duplicate).unwrap();
    assert!(duplicate.is_empty());
    assert_eq!(
        graffy_core::boundary::project(&[JournalReader::read_all(&out.journal_path).unwrap()])
            .exchanges
            .len(),
        1
    );
    std::fs::remove_file(out.journal_path).unwrap();
}
#[tokio::test]
async fn an_unreviewed_or_changed_draft_is_not_labeled_reviewed() {
    for kinds in [
        &["intake", "model", "respond"][..],
        &["intake", "model", "verify", "model", "respond"][..],
    ] {
        let (out, events) = execute(
            &linear(kinds),
            Executor::default(),
            &Fake::default(),
            &AutoApprove,
        )
        .await;
        assert!(events.iter().any(|e| matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.visibility=="released" && b.role=="response")));
        std::fs::remove_file(out.journal_path).unwrap();
    }
}
struct Human {
    outcome: ApprovalOutcome,
    questions: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl ApprovalHandler for Human {
    fn describe(&self) -> &'static str {
        "human-fixture"
    }
    fn is_human(&self) -> bool {
        true
    }
    async fn resolve(&self, _: &str, question: &str) -> ApprovalOutcome {
        self.questions.lock().unwrap().push(question.into());
        self.outcome.clone()
    }
}
#[tokio::test]
async fn approval_exposes_payload_and_distinguishes_rejection_from_abandonment() {
    for answer in [
        ApprovalOutcome::Rejected,
        ApprovalOutcome::Abandoned,
        ApprovalOutcome::Edited("replacement".into()),
    ] {
        let h = Human {
            outcome: answer.clone(),
            questions: Mutex::new(Vec::new()),
        };
        let (out, events) = execute(
            &linear(&["intake", "model", "approval", "respond"]),
            Executor::default(),
            &Fake::default(),
            &h,
        )
        .await;
        assert_eq!(out.status, wire::RunStatus::Cancelled);
        assert!(out.final_text.is_none());
        let question = h.questions.lock().unwrap()[0].clone();
        assert!(question.contains("draft answer"));
        assert!(question.contains("sha256"));
        let decided=events.iter().any(|e|matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.actor=="human" && b.role=="approval_decision"));
        assert_eq!(decided, answer != ApprovalOutcome::Abandoned);
        std::fs::remove_file(out.journal_path).unwrap();
    }
}
#[tokio::test]
async fn automatic_approval_is_not_a_human_exchange() {
    let (out, events) = execute(
        &linear(&["intake", "model", "approval", "respond"]),
        Executor::default(),
        &OfflineEcho,
        &AutoApprove,
    )
    .await;
    assert!(!events.iter().any(
        |e| matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.role.starts_with("approval"))
    ));
    std::fs::remove_file(out.journal_path).unwrap();
}
#[test]
fn journals_preserve_existing_bytes_and_reject_broken_identity_or_sequence() {
    use graffy_proto::prost::Message;
    let p = path();
    std::fs::write(&p, b"keep me").unwrap();
    assert!(JournalWriter::create(&p, "x").is_err());
    assert_eq!(std::fs::read(&p).unwrap(), b"keep me");
    let one = wire::RunEvent {
        run_id: "a".into(),
        seq: 1,
        ..Default::default()
    };
    for two in [
        wire::RunEvent {
            run_id: "b".into(),
            seq: 2,
            ..Default::default()
        },
        wire::RunEvent {
            run_id: "a".into(),
            seq: 3,
            ..Default::default()
        },
    ] {
        let mut bytes = one.encode_length_delimited_to_vec();
        bytes.extend(two.encode_length_delimited_to_vec());
        std::fs::write(&p, bytes).unwrap();
        assert!(JournalReader::read_all(&p).is_err());
    }
    std::fs::remove_file(p).unwrap();
}

#[tokio::test]
async fn visit_cap_after_a_response_still_fails_the_whole_run() {
    let mut s = linear(&["intake", "model", "respond"]);
    s.edges.push(EdgeSpec {
        from: "n2".into(),
        to: "n1".into(),
        when: Some("done".into()),
    });
    let (out, events) = execute(&s, Executor::default(), &Fake::default(), &AutoApprove).await;
    assert_eq!(out.status, wire::RunStatus::Failed);
    assert!(out.final_text.is_none());
    assert!(
        !events
            .iter()
            .any(|e| matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.visibility=="released"))
    );
    std::fs::remove_file(out.journal_path).unwrap();
}
#[test]
fn legacy_success_does_not_prove_output_delivery() {
    use graffy_proto::mcw::v1 as mcw;
    let p = path();
    let mut w = JournalWriter::create(&p, "legacy").unwrap();
    w.append(Event::IuRecorded(mcw::InformationUnit {
        kind: mcw::IuKind::Goal as i32,
        source: Some(mcw::ActorRef {
            kind: mcw::actor_ref::ActorKind::Human as i32,
            ..Default::default()
        }),
        ..Default::default()
    }))
    .unwrap();
    w.append(Event::IuRecorded(mcw::InformationUnit {
        attributes: std::collections::HashMap::from([("role".into(), "response".into())]),
        payload_text: "legacy answer".into(),
        ..Default::default()
    }))
    .unwrap();
    w.append(Event::RunFinished(wire::RunFinished {
        status: wire::RunStatus::Succeeded as i32,
        ..Default::default()
    }))
    .unwrap();
    drop(w);
    let projection = graffy_core::boundary::project(&[JournalReader::read_all(&p).unwrap()]);
    assert!(projection.exchanges.is_empty());
    assert!(
        !graffy_core::boundary::hydration_from(&projection)
            .iter()
            .any(|(_, t)| t == "legacy answer")
    );
    std::fs::remove_file(p).unwrap();
}

struct StalledTool;
#[async_trait::async_trait]
impl graffy_core::exec::ToolInvoker for StalledTool {
    async fn invoke(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<graffy_core::exec::ToolResponse, graffy_core::error::ToolError> {
        std::future::pending().await
    }
}
struct StalledApproval;
#[async_trait::async_trait]
impl ApprovalHandler for StalledApproval {
    fn describe(&self) -> &'static str {
        "stalled-human"
    }
    fn is_human(&self) -> bool {
        true
    }
    async fn resolve(&self, _: &str, _: &str) -> ApprovalOutcome {
        std::future::pending().await
    }
}
#[tokio::test]
async fn deadline_interrupts_tools_and_waiting_approvals_without_inventing_decisions() {
    for gate in [false, true] {
        let mut s = linear(if gate {
            &["intake", "model", "approval", "respond"]
        } else {
            &["intake", "tool.invoke", "model", "respond"]
        });
        if !gate {
            s.nodes[1].params.insert("server".into(), "fixture".into());
            s.nodes[1].params.insert("tool".into(), "stall".into());
        }
        let e = Executor {
            tool_invoker: Some(Arc::new(StalledTool)),
            remaining_budget: RemainingBudget {
                duration: Some(Duration::from_millis(30)),
                ..Default::default()
            },
            ..Default::default()
        };
        let (out, events) = tokio::time::timeout(
            Duration::from_secs(2),
            execute(&s, e, &Fake::default(), &StalledApproval),
        )
        .await
        .unwrap();
        assert_eq!(out.status, wire::RunStatus::BudgetExhausted);
        assert!(!events.iter().any(
            |e| matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.role=="approval_decision")
        ));
        std::fs::remove_file(out.journal_path).unwrap();
    }
}
