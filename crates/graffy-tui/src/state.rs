//! Pure fold from journal frames into renderable TUI state.
//!
//! Deliberately terminal-free: the same fold powers the live view (frames
//! arriving over the journal tap) and post-hoc inspection (frames read back
//! from disk), and it is unit-testable without a TTY.

use std::collections::BTreeMap;

use graffy_core::journal::wire;
use graffy_core::spec::GraphSpec;
use graffy_proto::journal::v1::run_event::Event;
use graffy_proto::mcw::v1 as mcw;

/// One row in the node pipeline panel.
#[derive(Debug, Clone)]
pub struct NodeRow {
    pub id: String,
    /// Node kind when known (seeded from the spec in live mode).
    pub kind: Option<String>,
    pub state: wire::NodeState,
    pub last_note: String,
    pub visits: u32,
}

/// One line in the activity feed.
#[derive(Debug, Clone)]
pub struct FeedLine {
    pub seq: u64,
    pub text: String,
}

/// One expandable entry in the step inspector.
#[derive(Debug, Clone)]
pub struct InspectorEntry {
    pub seq: u64,
    pub title: String,
    pub body: Vec<String>,
}

/// Everything the TUI renders, folded from `RunEvent` frames.
#[derive(Debug, Default)]
pub struct AppState {
    pub graph_name: String,
    pub graph_version: String,
    pub run_id: String,
    pub spec_sha8: String,
    pub evidence_mode: String,
    pub status: Option<wire::RunStatus>,
    pub finished_summary: String,

    pub nodes: Vec<NodeRow>,
    node_index: BTreeMap<String, usize>,
    /// Node currently (or most recently) running — IU/evidence attribution.
    current_node: Option<String>,

    pub feed: Vec<FeedLine>,
    pub novice_line: String,

    pub iu_count: usize,
    pub evidence_count: usize,
    pub failure_count: usize,
    pub repair_count: usize,
    pub model_calls: usize,
    pub routing_decisions: usize,
    pub max_escalation: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_usd: f64,
    pub event_count: usize,

    /// Per-node inspector entries, in arrival order.
    pub inspector: BTreeMap<String, Vec<InspectorEntry>>,
}

impl AppState {
    /// Pre-populate the pipeline from the spec so every node is visible
    /// (pending) before its first event arrives. Live mode only.
    pub fn seed_from_spec(&mut self, spec: &GraphSpec) {
        self.graph_name = spec.graph.name.clone();
        self.graph_version = spec.graph.version.clone();
        for node in &spec.nodes {
            self.row_mut(&node.id).kind = Some(node.kind.clone());
        }
    }

    fn row_mut(&mut self, node_id: &str) -> &mut NodeRow {
        if let Some(&ix) = self.node_index.get(node_id) {
            return &mut self.nodes[ix];
        }
        self.nodes.push(NodeRow {
            id: node_id.to_owned(),
            kind: None,
            state: wire::NodeState::Unspecified,
            last_note: String::new(),
            visits: 0,
        });
        let ix = self.nodes.len() - 1;
        self.node_index.insert(node_id.to_owned(), ix);
        &mut self.nodes[ix]
    }

    fn inspect(&mut self, node_id: &str, seq: u64, title: String, body: Vec<String>) {
        self.inspector
            .entry(node_id.to_owned())
            .or_default()
            .push(InspectorEntry { seq, title, body });
    }

    fn feed(&mut self, seq: u64, text: String) {
        self.feed.push(FeedLine { seq, text });
    }

    /// Fold one frame.
    pub fn apply(&mut self, frame: &wire::RunEvent) {
        self.event_count += 1;
        if self.run_id.is_empty() {
            self.run_id = frame.run_id.clone();
        }
        let seq = frame.seq;
        match &frame.event {
            Some(Event::RunStarted(m)) => {
                self.graph_name = m.graph_name.clone();
                self.graph_version = m.graph_version.clone();
                self.spec_sha8 = m.spec_sha256.chars().take(8).collect();
                self.evidence_mode = m.evidence_mode.clone();
                self.feed(seq, format!("run started — graph '{}'", m.graph_name));
                self.novice_line = "starting the graph…".to_owned();
            }
            Some(Event::NodeTransition(t)) => {
                let to = wire::NodeState::try_from(t.to).unwrap_or_default();
                let kind = {
                    let row = self.row_mut(&t.node_id);
                    row.state = to;
                    row.last_note = t.note.clone();
                    if to == wire::NodeState::Running {
                        row.visits += 1;
                    }
                    row.kind.clone()
                };
                if to == wire::NodeState::Running {
                    self.current_node = Some(t.node_id.clone());
                    self.novice_line = novice_phrase(kind.as_deref(), &t.node_id);
                }
                self.feed(seq, format!("{} → {}", t.node_id, state_word(to)));
                if !t.note.is_empty() {
                    self.inspect(
                        &t.node_id.clone(),
                        seq,
                        format!("transition → {}", state_word(to)),
                        vec![t.note.clone()],
                    );
                }
            }
            Some(Event::ModelCall(c)) => {
                self.model_calls += 1;
                self.input_tokens += c.input_tokens;
                self.output_tokens += c.output_tokens;
                self.total_usd += c.cost_usd;
                self.feed(
                    seq,
                    format!(
                        "{}: {} call — {}:{} ({}→{} tok)",
                        c.node_id, c.purpose, c.provider, c.model, c.input_tokens, c.output_tokens
                    ),
                );
                self.inspect(
                    &c.node_id.clone(),
                    seq,
                    format!("model call — {}", c.purpose),
                    vec![
                        format!("target   : {}:{}", c.provider, c.model),
                        format!("tokens   : {} in / {} out", c.input_tokens, c.output_tokens),
                        format!("latency  : {}ms   cost: ${:.4}", c.latency_ms, c.cost_usd),
                        format!(
                            "req sha  : {}",
                            &c.request_sha256.chars().take(12).collect::<String>()
                        ),
                    ],
                );
            }
            Some(Event::RoutingDecision(d)) => {
                self.routing_decisions += 1;
                self.max_escalation = self.max_escalation.max(d.escalation_level);
                if d.escalation_level > 0 {
                    self.feed(
                        seq,
                        format!(
                            "{}: ESCALATED to {}:{} (level {})",
                            d.node_id, d.chosen_provider, d.chosen_model, d.escalation_level
                        ),
                    );
                }
                self.inspect(
                    &d.node_id.clone(),
                    seq,
                    format!("routing — escalation level {}", d.escalation_level),
                    vec![
                        format!("requested: tier '{}'", d.requested_tier),
                        format!("chosen   : {}:{}", d.chosen_provider, d.chosen_model),
                        format!("reason   : {}", d.reason),
                        format!("considered: {}", d.considered.join(", ")),
                    ],
                );
            }
            Some(Event::IuRecorded(iu)) => {
                self.iu_count += 1;
                let node = self.attribution(&iu.source);
                let kind = iu_kind_word(iu.kind);
                self.feed(seq, format!("{node}: IU recorded ({kind})"));
                let mut body = vec![
                    format!(
                        "kind     : {kind}   salience: {}",
                        iu.salience
                            .map(|s| format!("{s:.2} ({})", iu.salience_source))
                            .unwrap_or_else(|| "unmeasured".to_owned())
                    ),
                    format!("support  : {}", evidence_level_word(iu.support_ceiling)),
                ];
                body.extend(wrap_payload("payload  : ", &iu.payload_text));
                if !iu.evidence_ids.is_empty() {
                    body.push(format!("evidence : {}", iu.evidence_ids.join(", ")));
                }
                if !iu.supersedes_iu_ids.is_empty() {
                    body.push(format!("supersedes: {}", iu.supersedes_iu_ids.join(", ")));
                }
                if let Some(role) = iu.attributes.get("role") {
                    body.push(format!("role     : {role}"));
                }
                self.inspect(&node, seq, format!("IU — {kind}"), body);
            }
            Some(Event::EvidenceRecorded(ev)) => {
                self.evidence_count += 1;
                let node = self.attribution(&ev.producer);
                let kind = evidence_kind_word(ev.kind);
                self.feed(seq, format!("{node}: evidence ({kind})"));
                self.inspect(
                    &node,
                    seq,
                    format!("evidence — {kind}"),
                    vec![
                        format!("level    : {}", evidence_level_word(ev.level)),
                        format!("uri      : {}", ev.uri),
                        format!(
                            "sha256   : {}",
                            ev.content_hash.chars().take(12).collect::<String>()
                        ),
                        format!("summary  : {}", ev.summary),
                        format!(
                            "visible  : {}",
                            if ev.user_visible {
                                "yes (strict mode)"
                            } else {
                                "trace-only"
                            }
                        ),
                    ],
                );
            }
            Some(Event::FailureRaised(f)) => {
                self.failure_count += 1;
                let node = f
                    .detector
                    .as_ref()
                    .map(|a| a.id.clone())
                    .unwrap_or_default();
                self.feed(
                    seq,
                    format!("⚠ failure signal: {}", failure_mode_word(f.mode)),
                );
                self.inspect(
                    &node,
                    seq,
                    format!("failure — {}", failure_mode_word(f.mode)),
                    vec![
                        format!(
                            "confidence: {}",
                            f.confidence
                                .map(|v| format!("{v:.2}"))
                                .unwrap_or_else(|| "uncalibrated hypothesis".to_owned())
                        ),
                        format!("signal    : {}", f.early_signal),
                    ],
                );
            }
            Some(Event::RepairExecuted(r)) => {
                self.repair_count += 1;
                let node = r
                    .executor
                    .as_ref()
                    .map(|a| a.id.clone())
                    .unwrap_or_default();
                self.feed(
                    seq,
                    format!("repair executed: {}", repair_word(r.operation)),
                );
                self.inspect(
                    &node,
                    seq,
                    format!("repair — {}", repair_word(r.operation)),
                    vec![format!("summary: {}", r.summary)],
                );
            }
            Some(Event::Approval(a)) => {
                let decision = wire::ApprovalDecision::try_from(a.decision).unwrap_or_default();
                self.feed(
                    seq,
                    format!("{}: approval {}", a.node_id, approval_word(decision)),
                );
                self.inspect(
                    &a.node_id.clone(),
                    seq,
                    format!("approval — {}", approval_word(decision)),
                    vec![
                        format!("question: {}", a.prompt),
                        format!("note: {}", a.note),
                    ],
                );
                if decision == wire::ApprovalDecision::Pending {
                    self.novice_line = "waiting for a human decision…".to_owned();
                }
            }
            Some(Event::Budget(b)) => {
                self.feed(
                    seq,
                    format!(
                        "budget checkpoint: {} tok / ${:.4} / {:.1}s",
                        b.tokens_spent, b.usd_spent, b.seconds_spent
                    ),
                );
            }
            Some(Event::RunFinished(f)) => {
                self.status = Some(wire::RunStatus::try_from(f.status).unwrap_or_default());
                self.finished_summary = f.summary.clone();
                self.feed(seq, format!("run finished — {}", status_word(self.status)));
                self.novice_line = match self.status {
                    Some(wire::RunStatus::Succeeded) => "done — answer verified and delivered.",
                    Some(wire::RunStatus::Failed) => {
                        "stopped — the graph could not verify an answer."
                    }
                    Some(wire::RunStatus::Cancelled) => "cancelled by a human decision.",
                    Some(wire::RunStatus::BudgetExhausted) => {
                        "stopped — the run budget was used up."
                    }
                    _ => "finished.",
                }
                .to_owned();
            }
            Some(Event::ToolCall(t)) => {
                self.feed(seq, format!("{}: tool call {}", t.node_id, t.tool_name));
                self.inspect(
                    &t.node_id.clone(),
                    seq,
                    format!("tool call — {}", t.tool_name),
                    vec![
                        format!("success: {}   latency: {}ms", t.success, t.latency_ms),
                        format!("error  : {}", t.error),
                    ],
                );
            }
            Some(Event::HrdmSampled(h)) => {
                self.feed(seq, format!("H/R/D/M sample recorded (scope {})", h.scope));
            }
            Some(Event::McwSnapshot(_)) => {
                self.feed(seq, "MCW state snapshot captured".to_owned());
            }
            None => {}
        }
    }

    /// Attribute an actor to a pipeline node for inspector grouping.
    fn attribution(&mut self, actor: &Option<mcw::ActorRef>) -> String {
        if let Some(actor) = actor {
            let is_node = actor.kind == mcw::actor_ref::ActorKind::AgentNode as i32
                && self.node_index.contains_key(&actor.id);
            if is_node {
                return actor.id.clone();
            }
        }
        self.current_node
            .clone()
            .unwrap_or_else(|| "run".to_owned())
    }
}

fn wrap_payload(prefix: &str, payload: &str) -> Vec<String> {
    const WIDTH: usize = 76;
    const MAX_LINES: usize = 6;
    let mut lines = Vec::new();
    let clean = payload.replace('\n', " ⏎ ");
    let chars: Vec<char> = clean.chars().collect();
    for (i, chunk) in chars.chunks(WIDTH).enumerate() {
        if i >= MAX_LINES {
            lines.push("  … (truncated — full text in the journal)".to_owned());
            break;
        }
        let head = if i == 0 { prefix } else { "           " };
        lines.push(format!("{head}{}", chunk.iter().collect::<String>()));
    }
    if lines.is_empty() {
        lines.push(format!("{prefix}(empty)"));
    }
    lines
}

pub fn state_word(state: wire::NodeState) -> &'static str {
    match state {
        wire::NodeState::Queued => "queued",
        wire::NodeState::Running => "running",
        wire::NodeState::Succeeded => "succeeded",
        wire::NodeState::Failed => "failed",
        wire::NodeState::Skipped => "skipped",
        wire::NodeState::AwaitingApproval => "awaiting approval",
        wire::NodeState::Cancelled => "cancelled",
        wire::NodeState::Unspecified => "pending",
    }
}

pub fn state_glyph(state: wire::NodeState) -> &'static str {
    match state {
        wire::NodeState::Unspecified => "·",
        wire::NodeState::Queued => "○",
        wire::NodeState::Running => "◐",
        wire::NodeState::Succeeded => "●",
        wire::NodeState::Failed => "✖",
        wire::NodeState::Skipped => "⊘",
        wire::NodeState::AwaitingApproval => "◔",
        wire::NodeState::Cancelled => "◌",
    }
}

pub fn status_word(status: Option<wire::RunStatus>) -> &'static str {
    match status {
        Some(wire::RunStatus::Running) => "running",
        Some(wire::RunStatus::Succeeded) => "SUCCEEDED",
        Some(wire::RunStatus::Failed) => "FAILED",
        Some(wire::RunStatus::Cancelled) => "CANCELLED",
        Some(wire::RunStatus::BudgetExhausted) => "BUDGET EXHAUSTED",
        _ => "in flight",
    }
}

fn novice_phrase(kind: Option<&str>, node_id: &str) -> String {
    match kind {
        Some("intake") => "understanding your request…".to_owned(),
        Some("research") => "gathering evidence before answering…".to_owned(),
        Some("model") => "drafting a response from the gathered facts…".to_owned(),
        Some("verify") => "double-checking the draft before it reaches you…".to_owned(),
        Some("respond") => "delivering the verified answer…".to_owned(),
        Some("approval") => "waiting for a human decision…".to_owned(),
        Some(k) if k.starts_with("repair.") => "repairing shared understanding…".to_owned(),
        _ => format!("working on step '{node_id}'…"),
    }
}

fn iu_kind_word(kind: i32) -> &'static str {
    match mcw::IuKind::try_from(kind).unwrap_or_default() {
        mcw::IuKind::Assumption => "assumption",
        mcw::IuKind::Goal => "goal",
        mcw::IuKind::Constraint => "constraint",
        mcw::IuKind::Correction => "correction",
        mcw::IuKind::Distinction => "distinction",
        mcw::IuKind::PriorityChange => "priority change",
        mcw::IuKind::Other => "other",
        mcw::IuKind::Unspecified => "unspecified",
    }
}

fn evidence_kind_word(kind: i32) -> &'static str {
    match mcw::EvidenceKind::try_from(kind).unwrap_or_default() {
        mcw::EvidenceKind::ToolResult => "tool result",
        mcw::EvidenceKind::McpResult => "MCP result",
        mcw::EvidenceKind::WebSource => "web source",
        mcw::EvidenceKind::MemoryRecall => "memory recall",
        mcw::EvidenceKind::Document => "document",
        mcw::EvidenceKind::Dataset => "dataset",
        mcw::EvidenceKind::ModelInference => "model inference",
        mcw::EvidenceKind::HumanInput => "human input",
        mcw::EvidenceKind::BenchmarkResult => "benchmark result",
        mcw::EvidenceKind::Unspecified => "unspecified",
    }
}

fn evidence_level_word(level: i32) -> &'static str {
    match mcw::SupportLevel::try_from(level).unwrap_or_default() {
        mcw::SupportLevel::Definitional => "definitional",
        mcw::SupportLevel::Observational => "observational",
        mcw::SupportLevel::Empirical => "empirical",
        mcw::SupportLevel::Validated => "validated",
        mcw::SupportLevel::Unspecified => "unleveled",
    }
}

fn failure_mode_word(mode: i32) -> &'static str {
    match mcw::FailureMode::try_from(mode).unwrap_or_default() {
        mcw::FailureMode::Drift => "drift",
        mcw::FailureMode::AsymmetricStateAdvancement => "asymmetric state advancement",
        mcw::FailureMode::FalseAlignment => "false alignment",
        mcw::FailureMode::Overcompression => "overcompression",
        mcw::FailureMode::ConstraintOpacity => "constraint opacity",
        mcw::FailureMode::RepairSuppression => "repair suppression",
        mcw::FailureMode::Unspecified => "unspecified",
    }
}

fn repair_word(op: i32) -> &'static str {
    match mcw::RepairOperation::try_from(op).unwrap_or_default() {
        mcw::RepairOperation::Regrounding => "re-grounding",
        mcw::RepairOperation::Decompression => "decompression",
        mcw::RepairOperation::Reweighting => "re-weighting",
        mcw::RepairOperation::Disambiguation => "disambiguation",
        mcw::RepairOperation::Synchronization => "synchronization",
        mcw::RepairOperation::Unspecified => "unspecified",
    }
}

fn approval_word(decision: wire::ApprovalDecision) -> &'static str {
    match decision {
        wire::ApprovalDecision::Pending => "pending",
        wire::ApprovalDecision::Approved => "approved",
        wire::ApprovalDecision::Rejected => "rejected",
        wire::ApprovalDecision::Edited => "approved with edits",
        wire::ApprovalDecision::Unspecified => "unspecified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graffy_core::exec::{AutoApprove, Executor, OfflineEcho, RunInput};

    const DEFAULT_CONVERSATION: &str = include_str!("../../../graphs/conversation.default.toml");

    #[tokio::test]
    async fn fold_of_a_real_offline_run_matches_reference_summary() {
        let spec = GraphSpec::from_toml_str(DEFAULT_CONVERSATION).unwrap();
        let path = std::env::temp_dir().join(format!("graffy-tui-state-test-{}", ulid_stamp()));
        let outcome = Executor::default()
            .run(
                &spec,
                DEFAULT_CONVERSATION,
                RunInput {
                    prompt: "state fold test".into(),
                    session_id: None,
                    feedback: Vec::new(),
                },
                &path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await
            .unwrap();

        let events = graffy_core::journal::JournalReader::read_all(&path).unwrap();
        let reference = graffy_core::journal::summarize(&events);

        let mut app = AppState::default();
        app.seed_from_spec(&spec);
        for frame in &events {
            app.apply(frame);
        }

        assert_eq!(app.run_id, outcome.run_id);
        assert_eq!(app.status, reference.status);
        assert_eq!(app.iu_count, reference.iu_count);
        assert_eq!(app.evidence_count, reference.evidence_count);
        assert_eq!(app.model_calls, reference.model_calls);
        assert_eq!(app.routing_decisions, reference.routing_decisions);
        assert_eq!(app.input_tokens, reference.total_input_tokens);
        assert_eq!(app.event_count, events.len());

        // Every pipeline node ended Succeeded and has inspector entries for
        // the interesting ones.
        for row in &app.nodes {
            assert_eq!(row.state, wire::NodeState::Succeeded, "node {}", row.id);
        }
        assert!(app.inspector.get("draft").is_some_and(|e| !e.is_empty()));
        assert!(app.inspector.get("verify").is_some_and(|e| !e.is_empty()));
        assert!(!app.novice_line.is_empty());
        std::fs::remove_file(&path).ok();
    }

    fn ulid_stamp() -> String {
        graffy_core::id::RunId::generate().to_string()
    }
}
