//! The executor — Phase 1 milestone M2 (ADR-0001, ADR-0005).
//!
//! A single-process, tokio-driven step loop patterned after graph-flow's
//! typed-task execution, implemented natively so that:
//!
//! * every node payload is a protobuf message (graffy-proto),
//! * every observable is committed to the append-only journal as it happens,
//! * TOML guard conditions decide which edges fire,
//! * guarded back-edges consume per-node visit budget on every traversal,
//! * routing decisions — including quality-gate escalations — are journaled,
//! * the no-raw-execution invariant holds: the ONLY path to a model is a
//!   scheduled node calling [`NodeCtx::call_model`].
//!
//! Node behaviors return [`NodeExecutionResult`]:
//! * `Continue` — merge output, fire matching guarded edges;
//! * `Escalate` — quality threshold failed: matching targets get their
//!   routing tier bumped up the ladder (never a silent bounce-back);
//! * `PauseForDisambiguation` — human-in-the-loop checkpoint; the
//!   [`ApprovalHandler`] resolves it and the decision is journaled.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use petgraph::graph::NodeIndex;
use sha2::{Digest, Sha256};

use graffy_proto::mcw::v1 as mcw;
use graffy_proto::prost_types;

use crate::error::{ExecError, ModelError};
use crate::graph::{CompiledGraph, CompiledNode};
use crate::id::{EvidenceId, IuId, RunId, SessionId};
use crate::journal::{JournalWriter, wire};
use crate::spec::{GraphSpec, PolicySpec};

use crate::journal::wire::run_event::Event;

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

/// Current wall-clock time as a protobuf timestamp.
pub fn now_ts() -> prost_types::Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    prost_types::Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

/// SHA-256 of arbitrary bytes, lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// The human participant (HCW side).
pub fn human_actor() -> mcw::ActorRef {
    mcw::ActorRef {
        id: "human".to_owned(),
        kind: mcw::actor_ref::ActorKind::Human as i32,
        display_name: "Human".to_owned(),
    }
}

/// A graph node acting with model backing (the only lawful model actor).
pub fn node_actor(node_id: &str) -> mcw::ActorRef {
    mcw::ActorRef {
        id: node_id.to_owned(),
        kind: mcw::actor_ref::ActorKind::AgentNode as i32,
        display_name: node_id.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Guard expressions
// ---------------------------------------------------------------------------
// M2 grammar (documented in ADR-0003):
//   expr   := clause ( '&&' clause )*
//   clause := path '==' lit | path '!=' lit | path        (bare = truthy)
//   path   := [A-Za-z0-9_.-]+          lit := '...' | "..."

/// Evaluate a guard expression against the source node's output facts.
/// Missing facts make equality clauses false and bare clauses false.
pub fn eval_guard(expr: &str, facts: &BTreeMap<String, String>) -> Result<bool, ExecError> {
    for clause in expr.split("&&") {
        let clause = clause.trim();
        if clause.is_empty() {
            return Err(ExecError::BadGuard {
                expr: expr.to_owned(),
                reason: "empty clause".to_owned(),
            });
        }
        if !eval_clause(expr, clause, facts)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eval_clause(
    expr: &str,
    clause: &str,
    facts: &BTreeMap<String, String>,
) -> Result<bool, ExecError> {
    if let Some((path, lit)) = split_binary(clause, "==") {
        let lit = parse_literal(expr, lit)?;
        let path = parse_path(expr, path)?;
        return Ok(facts.get(&path).map(String::as_str) == Some(lit.as_str()));
    }
    if let Some((path, lit)) = split_binary(clause, "!=") {
        let lit = parse_literal(expr, lit)?;
        let path = parse_path(expr, path)?;
        return Ok(facts.get(&path).map(String::as_str) != Some(lit.as_str()));
    }
    let path = parse_path(expr, clause)?;
    Ok(facts.get(&path).map(String::as_str) == Some("true"))
}

fn split_binary<'a>(clause: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    clause.split_once(op)
}

fn parse_path(expr: &str, raw: &str) -> Result<String, ExecError> {
    let path = raw.trim();
    let valid = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if valid {
        Ok(path.to_owned())
    } else {
        Err(ExecError::BadGuard {
            expr: expr.to_owned(),
            reason: format!("invalid fact path '{raw}'"),
        })
    }
}

fn parse_literal(expr: &str, raw: &str) -> Result<String, ExecError> {
    let lit = raw.trim();
    let stripped = lit
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| lit.strip_prefix('"').and_then(|s| s.strip_suffix('"')));
    match stripped {
        Some(inner) => Ok(inner.to_owned()),
        None => Err(ExecError::BadGuard {
            expr: expr.to_owned(),
            reason: format!("literal must be quoted: {raw}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// The model plane (trait only — implementations live in graffy-providers)
// ---------------------------------------------------------------------------

/// A request the executor makes on behalf of a node. Tiers, not vendor names:
/// the invoker resolves "fast" / "balanced" / "frontier" to concrete models.
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub tier: String,
    /// "draft" | "verify" | "detect" | "repair" | custom.
    pub purpose: String,
    pub system: String,
    pub prompt: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
}

/// What came back, with enough metadata to journal honestly.
#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub provider: String,
    pub model: String,
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms: u64,
}

/// The only doorway to a model. There is deliberately no free function that
/// completes a prompt — invokers are handed to the executor, and the executor
/// hands them to scheduled nodes (ADR-0005).
#[async_trait::async_trait]
pub trait ModelInvoker: Send + Sync {
    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError>;

    /// Candidate model identifiers for a tier (journaled in RoutingDecision).
    fn tier_candidates(&self, _tier: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Deterministic offline invoker for tests and `graffy run --offline`.
/// Verify purposes always PASS; everything else echoes the prompt. Clearly
/// labeled in every journal record — never mistakable for a real model.
pub struct OfflineEcho;

#[async_trait::async_trait]
impl ModelInvoker for OfflineEcho {
    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let text = if request.purpose == "verify" {
            "PASS — offline echo judge (no real model was consulted)".to_owned()
        } else {
            format!(
                "[offline echo — no real model was consulted]\n{}",
                request.prompt
            )
        };
        Ok(ModelResponse {
            provider: "offline".to_owned(),
            model: "echo".to_owned(),
            input_tokens: (request.prompt.len() / 4) as u64,
            output_tokens: (text.len() / 4) as u64,
            cost_usd: 0.0,
            latency_ms: 0,
            text,
        })
    }

    fn tier_candidates(&self, _tier: &str) -> Vec<String> {
        vec!["offline:echo".to_owned()]
    }
}

// ---------------------------------------------------------------------------
// Human-in-the-loop
// ---------------------------------------------------------------------------

/// Outcome of a disambiguation / approval checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Approved,
    Rejected,
    /// Approved with a human edit (the edit becomes a correction IU upstream).
    Edited(String),
}

/// Resolves [`NodeExecutionResult::PauseForDisambiguation`] checkpoints.
/// The TUI supplies an interactive handler in M3; tests use [`AutoApprove`].
#[async_trait::async_trait]
pub trait ApprovalHandler: Send + Sync {
    /// Journaled as `ApprovalRecord.decided_by` — say who really decided
    /// ("auto-approve", "human-tui", "human-cli", …).
    fn describe(&self) -> &'static str {
        "handler"
    }

    async fn resolve(&self, node_id: &str, question: &str) -> ApprovalOutcome;
}

/// Approves everything (headless runs, tests). Every use is journaled as an
/// ApprovalRecord with `decided_by = "auto-approve"`, so nothing is hidden.
pub struct AutoApprove;

#[async_trait::async_trait]
impl ApprovalHandler for AutoApprove {
    fn describe(&self) -> &'static str {
        "auto-approve"
    }

    async fn resolve(&self, _node_id: &str, _question: &str) -> ApprovalOutcome {
        ApprovalOutcome::Approved
    }
}

// ---------------------------------------------------------------------------
// Node behaviors
// ---------------------------------------------------------------------------

/// Everything a node's output can contribute back to the run.
#[derive(Debug, Default, Clone)]
pub struct NodeOutput {
    /// Facts guard expressions evaluate against (e.g. `verdict = "pass"`).
    pub guard_facts: BTreeMap<String, String>,
    /// Information Units to merge into the shared ledger (journaled).
    pub ius: Vec<mcw::InformationUnit>,
    /// Evidence artifacts backing this node's claims (journaled).
    pub evidence: Vec<mcw::EvidenceArtifact>,
    /// Free text (the respond node's text becomes the run's final answer).
    pub text: Option<String>,
}

/// Type-safe state variants the execution loop processes (locked in the
/// Phase 1 M2 requirements).
#[derive(Debug)]
pub enum NodeExecutionResult {
    /// Proceed along whichever outgoing edges' guards pass.
    Continue(NodeOutput),
    /// Quality threshold failed: matching targets escalate up the routing
    /// ladder instead of silently re-running on the same model.
    Escalate { reason: String, output: NodeOutput },
    /// Human-in-the-loop checkpoint before anything else happens.
    PauseForDisambiguation {
        question: String,
        output: NodeOutput,
    },
}

/// Per-dispatch context handed to a node behavior.
pub struct NodeCtx<'a> {
    pub run_id: &'a str,
    pub session_id: &'a str,
    pub node: &'a CompiledNode,
    pub prompt: &'a str,
    /// The shared IU ledger accumulated so far (read-only in behaviors).
    pub ledger: &'a [mcw::InformationUnit],
    /// Facts produced by the node that enqueued this one.
    pub facts_in: &'a BTreeMap<String, String>,
    /// Tier after applying escalations to the node's requested tier.
    pub effective_tier: String,
    pub escalation_level: u32,
    pub policy: &'a PolicySpec,
    pub invoker: &'a dyn ModelInvoker,
    /// Evidence policy: whether artifacts surface in the UI (strict) or stay
    /// journal-only (trace-only).
    pub evidence_visible: bool,
    /// Journal events the behavior wants recorded (model calls, routing…).
    pub records: Vec<Event>,
}

impl NodeCtx<'_> {
    /// The one lawful path to a model. Journals a RoutingDecision and a
    /// ModelCallRecord for every invocation.
    pub async fn call_model(
        &mut self,
        purpose: &str,
        system: String,
        prompt: String,
    ) -> Result<ModelResponse, ExecError> {
        let request_sha256 = sha256_hex(format!("{system}\n---\n{prompt}").as_bytes());
        let request = ModelRequest {
            tier: self.effective_tier.clone(),
            purpose: purpose.to_owned(),
            system,
            prompt,
            temperature: None,
            max_tokens: None,
        };
        let started = Instant::now();
        let response = self.invoker.complete(&request).await?;
        let latency_ms = response
            .latency_ms
            .max(started.elapsed().as_millis() as u64);

        self.records
            .push(Event::RoutingDecision(wire::RoutingDecision {
                node_id: self.node.id.clone(),
                requested_tier: self
                    .node
                    .model_tier
                    .clone()
                    .unwrap_or_else(|| "balanced".to_owned()),
                chosen_provider: response.provider.clone(),
                chosen_model: response.model.clone(),
                escalation_level: self.escalation_level,
                reason: if self.escalation_level == 0 {
                    "policy".to_owned()
                } else {
                    "quality-gate-escalation".to_owned()
                },
                considered: self.invoker.tier_candidates(&self.effective_tier),
            }));
        self.records.push(Event::ModelCall(wire::ModelCallRecord {
            node_id: self.node.id.clone(),
            provider: response.provider.clone(),
            model: response.model.clone(),
            purpose: purpose.to_owned(),
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            cost_usd: response.cost_usd,
            latency_ms,
            streamed: false,
            request_sha256,
        }));
        Ok(response)
    }

    fn param_str(&self, key: &str) -> Option<String> {
        self.node
            .params
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    }
}

/// A node implementation, dispatched by `kind`.
#[async_trait::async_trait]
pub trait NodeBehavior: Send + Sync {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError>;
}

/// Maps node kinds to behaviors. Unknown kinds fail the run loudly — graffy
/// never silently skips a declared step.
pub struct NodeRegistry {
    map: HashMap<String, Arc<dyn NodeBehavior>>,
}

impl NodeRegistry {
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// The built-in kinds every graffy installation understands.
    pub fn with_defaults() -> Self {
        let mut registry = Self::empty();
        registry.register("intake", Arc::new(IntakeNode));
        registry.register("research", Arc::new(ResearchNode));
        registry.register("model", Arc::new(ModelNode));
        registry.register("verify", Arc::new(VerifyNode));
        registry.register("respond", Arc::new(RespondNode));
        registry.register("approval", Arc::new(ApprovalNode));
        registry
    }

    pub fn register(&mut self, kind: &str, behavior: Arc<dyn NodeBehavior>) {
        self.map.insert(kind.to_owned(), behavior);
    }

    pub fn get(&self, kind: &str) -> Option<Arc<dyn NodeBehavior>> {
        self.map.get(kind).cloned()
    }
}

fn five_stage_records(
    origin: mcw::ActorRef,
    integrator: mcw::ActorRef,
    representation: &str,
) -> Vec<mcw::IuStageRecord> {
    use mcw::IuStage;
    let mk = |stage: IuStage, actor: &mcw::ActorRef| mcw::IuStageRecord {
        stage: stage as i32,
        occurred_at: Some(now_ts()),
        actor: Some(actor.clone()),
        representation: representation.to_owned(),
        fidelity_estimate: 1.0,
        note: String::new(),
    };
    vec![
        mk(IuStage::Selection, &origin),
        mk(IuStage::Encoding, &origin),
        mk(IuStage::Transmission, &origin),
        mk(IuStage::Decoding, &integrator),
        mk(IuStage::Integration, &integrator),
    ]
}

/// `intake` — decomposes the user turn into Information Units and records the
/// verbatim prompt as human-input evidence (L1).
struct IntakeNode;

#[async_trait::async_trait]
impl NodeBehavior for IntakeNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let mut out = NodeOutput::default();
        let evidence_id = EvidenceId::generate().to_string();
        out.evidence.push(mcw::EvidenceArtifact {
            id: evidence_id.clone(),
            kind: mcw::EvidenceKind::HumanInput as i32,
            uri: format!("graffy://run/{}/prompt", ctx.run_id),
            content_hash: sha256_hex(ctx.prompt.as_bytes()),
            collected_at: Some(now_ts()),
            producer: Some(human_actor()),
            run_id: ctx.run_id.to_owned(),
            summary: "verbatim user prompt".to_owned(),
            level: mcw::EvidenceLevel::L1Observational as i32,
            user_visible: ctx.evidence_visible,
        });
        out.ius.push(mcw::InformationUnit {
            id: IuId::generate().to_string(),
            kind: mcw::IuKind::Goal as i32,
            payload_text: ctx.prompt.to_owned(),
            created_at: Some(now_ts()),
            source: Some(human_actor()),
            session_id: ctx.session_id.to_owned(),
            run_id: ctx.run_id.to_owned(),
            stages: five_stage_records(human_actor(), node_actor(&ctx.node.id), "text"),
            salience: 1.0,
            evidence_ids: vec![evidence_id],
            evidence_level: mcw::EvidenceLevel::L1Observational as i32,
            ..Default::default()
        });
        out.guard_facts
            .insert("intake".to_owned(), "true".to_owned());
        Ok(NodeExecutionResult::Continue(out))
    }
}

/// `research` — the grounding slot in every pipeline. Phase 2 attaches MCP
/// tools and memory recall here; M2 keeps the slot structurally present
/// without fabricating evidence it did not collect.
struct ResearchNode;

#[async_trait::async_trait]
impl NodeBehavior for ResearchNode {
    async fn execute(&self, _ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let mut out = NodeOutput::default();
        out.guard_facts
            .insert("grounded".to_owned(), "true".to_owned());
        Ok(NodeExecutionResult::Continue(out))
    }
}

/// `model` — produces a draft from grounded IUs only, wrapping the completion
/// into an Information Unit backed by a MODEL_INFERENCE evidence artifact
/// (L0 — model output alone never rises above the definitional layer).
struct ModelNode;

#[async_trait::async_trait]
impl NodeBehavior for ModelNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let goal = ctx
            .ledger
            .iter()
            .rev()
            .find(|iu| iu.kind == mcw::IuKind::Goal as i32)
            .map(|iu| iu.payload_text.clone())
            .unwrap_or_else(|| ctx.prompt.to_owned());
        let feedback: Vec<String> = ctx
            .ledger
            .iter()
            .filter(|iu| {
                iu.attributes
                    .get("role")
                    .is_some_and(|r| r == "review-feedback")
            })
            .map(|iu| iu.payload_text.clone())
            .collect();

        let system = ctx.param_str("system").unwrap_or_else(|| {
            "You are a careful drafting node inside a graffy graph. Ground every claim; \
             do not invent facts. Where you are uncertain, say so explicitly."
                .to_owned()
        });
        let prompt = if feedback.is_empty() {
            goal
        } else {
            format!(
                "{goal}\n\nRevise your previous draft, addressing this review feedback:\n{}",
                feedback.join("\n---\n")
            )
        };

        let response = ctx.call_model("draft", system, prompt).await?;

        let mut out = NodeOutput::default();
        let evidence_id = EvidenceId::generate().to_string();
        out.evidence.push(mcw::EvidenceArtifact {
            id: evidence_id.clone(),
            kind: mcw::EvidenceKind::ModelInference as i32,
            uri: format!("graffy://run/{}/node/{}/draft", ctx.run_id, ctx.node.id),
            content_hash: sha256_hex(response.text.as_bytes()),
            collected_at: Some(now_ts()),
            producer: Some(node_actor(&ctx.node.id)),
            run_id: ctx.run_id.to_owned(),
            summary: format!(
                "draft completion from {}:{}",
                response.provider, response.model
            ),
            level: mcw::EvidenceLevel::L0Definitional as i32,
            user_visible: ctx.evidence_visible,
        });
        let mut attributes = HashMap::new();
        attributes.insert("role".to_owned(), "draft".to_owned());
        attributes.insert(
            "model".to_owned(),
            format!("{}:{}", response.provider, response.model),
        );
        out.ius.push(mcw::InformationUnit {
            id: IuId::generate().to_string(),
            kind: mcw::IuKind::Other as i32,
            payload_text: response.text.clone(),
            created_at: Some(now_ts()),
            source: Some(node_actor(&ctx.node.id)),
            session_id: ctx.session_id.to_owned(),
            run_id: ctx.run_id.to_owned(),
            stages: five_stage_records(node_actor(&ctx.node.id), node_actor(&ctx.node.id), "text"),
            salience: 0.8,
            evidence_ids: vec![evidence_id],
            evidence_level: mcw::EvidenceLevel::L0Definitional as i32,
            attributes,
            ..Default::default()
        });
        out.guard_facts
            .insert("drafted".to_owned(), "true".to_owned());
        out.text = Some(response.text);
        Ok(NodeExecutionResult::Continue(out))
    }
}

/// `verify` — the peer-review quality gate. A judge model (routed on the same
/// ladder) returns PASS or REVISE; REVISE escalates instead of bouncing back
/// to the same producer configuration.
struct VerifyNode;

#[async_trait::async_trait]
impl NodeBehavior for VerifyNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let Some(draft) = ctx
            .ledger
            .iter()
            .rev()
            .find(|iu| iu.attributes.get("role").is_some_and(|r| r == "draft"))
        else {
            return Err(ExecError::NodeFailed(
                ctx.node.id.clone(),
                "no draft found in the ledger to verify".to_owned(),
            ));
        };

        let system = "You are a strict peer reviewer inside a graffy graph. Judge the draft \
                      for accuracy, grounding, and completeness. Your reply MUST begin with \
                      exactly one word: PASS or REVISE, followed by a short reason."
            .to_owned();
        let prompt = format!(
            "Minimum evidence level in force: {}.\n\nDRAFT UNDER REVIEW:\n{}",
            ctx.policy.evidence.min_level, draft.payload_text
        );
        let response = ctx.call_model("verify", system, prompt).await?;

        let first_word = response
            .text
            .split(|c: char| !c.is_ascii_alphabetic())
            .find(|w| !w.is_empty())
            .map(str::to_ascii_uppercase)
            .unwrap_or_default();
        let passed = first_word == "PASS";

        let mut out = NodeOutput::default();
        let mut attributes = HashMap::new();
        attributes.insert(
            "role".to_owned(),
            if passed {
                "review".to_owned()
            } else {
                "review-feedback".to_owned()
            },
        );
        out.ius.push(mcw::InformationUnit {
            id: IuId::generate().to_string(),
            kind: mcw::IuKind::Distinction as i32,
            payload_text: response.text.clone(),
            created_at: Some(now_ts()),
            source: Some(node_actor(&ctx.node.id)),
            session_id: ctx.session_id.to_owned(),
            run_id: ctx.run_id.to_owned(),
            stages: five_stage_records(node_actor(&ctx.node.id), node_actor(&ctx.node.id), "text"),
            salience: 0.9,
            evidence_level: mcw::EvidenceLevel::L0Definitional as i32,
            attributes,
            ..Default::default()
        });

        if passed {
            out.guard_facts
                .insert("verdict".to_owned(), "pass".to_owned());
            Ok(NodeExecutionResult::Continue(out))
        } else {
            out.guard_facts
                .insert("verdict".to_owned(), "revise".to_owned());
            Ok(NodeExecutionResult::Escalate {
                reason: response.text,
                output: out,
            })
        }
    }
}

/// `respond` — integrates the verified draft as the run's final answer.
struct RespondNode;

#[async_trait::async_trait]
impl NodeBehavior for RespondNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let Some(draft) = ctx
            .ledger
            .iter()
            .rev()
            .find(|iu| iu.attributes.get("role").is_some_and(|r| r == "draft"))
        else {
            return Err(ExecError::NodeFailed(
                ctx.node.id.clone(),
                "no draft found in the ledger to respond with".to_owned(),
            ));
        };

        let mut out = NodeOutput::default();
        let mut attributes = HashMap::new();
        attributes.insert("role".to_owned(), "response".to_owned());
        out.ius.push(mcw::InformationUnit {
            id: IuId::generate().to_string(),
            kind: mcw::IuKind::Other as i32,
            payload_text: draft.payload_text.clone(),
            created_at: Some(now_ts()),
            source: Some(node_actor(&ctx.node.id)),
            session_id: ctx.session_id.to_owned(),
            run_id: ctx.run_id.to_owned(),
            stages: five_stage_records(node_actor(&ctx.node.id), human_actor(), "text"),
            salience: 1.0,
            evidence_ids: draft.evidence_ids.clone(),
            evidence_level: draft.evidence_level,
            attributes,
            ..Default::default()
        });
        out.guard_facts.insert("done".to_owned(), "true".to_owned());
        out.text = Some(draft.payload_text.clone());
        Ok(NodeExecutionResult::Continue(out))
    }
}

/// `approval` — an explicit human-in-the-loop checkpoint node.
struct ApprovalNode;

#[async_trait::async_trait]
impl NodeBehavior for ApprovalNode {
    async fn execute(&self, ctx: &mut NodeCtx<'_>) -> Result<NodeExecutionResult, ExecError> {
        let question = ctx
            .param_str("question")
            .unwrap_or_else(|| "Approve this step to continue the run?".to_owned());
        Ok(NodeExecutionResult::PauseForDisambiguation {
            question,
            output: NodeOutput::default(),
        })
    }
}

// ---------------------------------------------------------------------------
// The executor
// ---------------------------------------------------------------------------

/// Input for one run.
#[derive(Debug, Clone)]
pub struct RunInput {
    pub prompt: String,
    /// Reuse an existing coordination session, or mint one.
    pub session_id: Option<String>,
}

/// What a finished run hands back (everything else is in the journal).
#[derive(Debug)]
pub struct RunOutcome {
    pub run_id: String,
    pub status: wire::RunStatus,
    pub final_text: Option<String>,
    pub journal_path: std::path::PathBuf,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_usd: f64,
    pub duration_ms: u64,
    pub notes: Vec<String>,
}

/// The tokio-driven graph execution loop.
pub struct Executor {
    pub registry: NodeRegistry,
    /// Runtime half of the cycle law: how many times a single node may run.
    pub max_node_visits: u32,
    /// Optional live mirror of committed journal frames (the TUI's feed).
    pub event_tap: Option<crate::journal::EventTap>,
}

impl Default for Executor {
    fn default() -> Self {
        Self {
            registry: NodeRegistry::with_defaults(),
            max_node_visits: 3,
            event_tap: None,
        }
    }
}

fn resolve_tier(ladder: &[String], base: &str, escalation: u32) -> String {
    if escalation == 0 || ladder.is_empty() {
        return base.to_owned();
    }
    match ladder.iter().position(|t| t == base) {
        Some(pos) => {
            let target = (pos + escalation as usize).min(ladder.len() - 1);
            ladder[target].clone()
        }
        None => base.to_owned(),
    }
}

impl Executor {
    /// Execute a spec end to end, committing every observable to `journal_path`.
    #[allow(clippy::too_many_lines)]
    pub async fn run(
        &self,
        spec: &GraphSpec,
        spec_toml: &str,
        input: RunInput,
        journal_path: &Path,
        invoker: &dyn ModelInvoker,
        approvals: &dyn ApprovalHandler,
    ) -> Result<RunOutcome, ExecError> {
        let started = Instant::now();
        let graph = CompiledGraph::compile(spec)?;
        let run_id = RunId::generate().to_string();
        let session_id = input
            .session_id
            .clone()
            .unwrap_or_else(|| SessionId::generate().to_string());
        let evidence_visible = spec.policy.evidence.mode != "trace-only";

        let mut journal =
            JournalWriter::create_with_tap(journal_path, &run_id, self.event_tap.clone())?;
        journal.append(Event::RunStarted(wire::RunManifest {
            run_id: run_id.clone(),
            graph_id: spec.graph.id.clone(),
            graph_name: spec.graph.name.clone(),
            graph_version: spec.graph.version.clone(),
            spec_sha256: sha256_hex(spec_toml.as_bytes()),
            session_id: session_id.clone(),
            started_at: Some(now_ts()),
            evidence_mode: spec.policy.evidence.mode.clone(),
            evidence_min_level: spec.policy.evidence.min_level.clone(),
            graffy_version: crate::VERSION.to_owned(),
        }))?;

        let mut ledger: Vec<mcw::InformationUnit> = Vec::new();
        let mut queue: VecDeque<(NodeIndex, BTreeMap<String, String>)> = graph
            .entry_nodes()
            .into_iter()
            .map(|ix| (ix, BTreeMap::new()))
            .collect();
        let mut queued: HashSet<NodeIndex> = queue.iter().map(|(ix, _)| *ix).collect();
        let mut visits: HashMap<NodeIndex, u32> = HashMap::new();
        let mut escalations: HashMap<String, u32> = HashMap::new();

        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut total_usd = 0f64;
        let mut failure_signals = 0u32;
        let mut repairs = 0u32;
        let mut final_text: Option<String> = None;
        let mut notes: Vec<String> = Vec::new();
        let mut forced_status: Option<wire::RunStatus> = None;

        while let Some((ix, facts_in)) = queue.pop_front() {
            queued.remove(&ix);
            let node = graph.node(ix);

            let visit = visits.entry(ix).or_insert(0);
            *visit += 1;
            if *visit > self.max_node_visits {
                notes.push(format!(
                    "node '{}' exceeded its visit cap ({}) — halting that path",
                    node.id, self.max_node_visits
                ));
                journal.append(Event::NodeTransition(wire::NodeTransition {
                    node_id: node.id.clone(),
                    from: wire::NodeState::Queued as i32,
                    to: wire::NodeState::Skipped as i32,
                    note: "visit cap exceeded".to_owned(),
                }))?;
                continue;
            }

            journal.append(Event::NodeTransition(wire::NodeTransition {
                node_id: node.id.clone(),
                from: wire::NodeState::Queued as i32,
                to: wire::NodeState::Running as i32,
                note: String::new(),
            }))?;
            tracing::info!(node = %node.id, kind = %node.kind, "node running");

            let Some(behavior) = self.registry.get(&node.kind) else {
                journal.append(Event::NodeTransition(wire::NodeTransition {
                    node_id: node.id.clone(),
                    from: wire::NodeState::Running as i32,
                    to: wire::NodeState::Failed as i32,
                    note: format!("unknown node kind '{}'", node.kind),
                }))?;
                journal.append(Event::RunFinished(wire::RunFinished {
                    status: wire::RunStatus::Failed as i32,
                    total_input_tokens: input_tokens,
                    total_output_tokens: output_tokens,
                    total_usd,
                    duration_ms: started.elapsed().as_millis() as u64,
                    failure_signal_count: failure_signals,
                    repair_count: repairs,
                    summary: format!("unknown node kind '{}'", node.kind),
                }))?;
                return Err(ExecError::UnknownNodeKind(node.kind.clone()));
            };

            let escalation_level = *escalations.get(&node.id).unwrap_or(&0);
            let base_tier = node
                .model_tier
                .clone()
                .unwrap_or_else(|| "balanced".to_owned());
            let effective_tier =
                resolve_tier(&spec.policy.routing.ladder, &base_tier, escalation_level);

            let mut ctx = NodeCtx {
                run_id: &run_id,
                session_id: &session_id,
                node,
                prompt: &input.prompt,
                ledger: &ledger,
                facts_in: &facts_in,
                effective_tier,
                escalation_level,
                policy: &spec.policy,
                invoker,
                evidence_visible,
                records: Vec::new(),
            };
            let result = behavior.execute(&mut ctx).await;
            let records = std::mem::take(&mut ctx.records);
            drop(ctx);

            for event in records {
                if let Event::ModelCall(call) = &event {
                    input_tokens += call.input_tokens;
                    output_tokens += call.output_tokens;
                    total_usd += call.cost_usd;
                }
                if matches!(event, Event::FailureRaised(_)) {
                    failure_signals += 1;
                }
                if matches!(event, Event::RepairExecuted(_)) {
                    repairs += 1;
                }
                journal.append(event)?;
            }

            let (output, escalated) = match result {
                Err(err) => {
                    journal.append(Event::NodeTransition(wire::NodeTransition {
                        node_id: node.id.clone(),
                        from: wire::NodeState::Running as i32,
                        to: wire::NodeState::Failed as i32,
                        note: err.to_string(),
                    }))?;
                    journal.append(Event::RunFinished(wire::RunFinished {
                        status: wire::RunStatus::Failed as i32,
                        total_input_tokens: input_tokens,
                        total_output_tokens: output_tokens,
                        total_usd,
                        duration_ms: started.elapsed().as_millis() as u64,
                        failure_signal_count: failure_signals,
                        repair_count: repairs,
                        summary: err.to_string(),
                    }))?;
                    return Err(err);
                }
                Ok(NodeExecutionResult::Continue(output)) => (output, false),
                Ok(NodeExecutionResult::Escalate { reason, output }) => {
                    notes.push(format!("quality gate at '{}': {}", node.id, reason));
                    (output, true)
                }
                Ok(NodeExecutionResult::PauseForDisambiguation { question, output }) => {
                    journal.append(Event::NodeTransition(wire::NodeTransition {
                        node_id: node.id.clone(),
                        from: wire::NodeState::Running as i32,
                        to: wire::NodeState::AwaitingApproval as i32,
                        note: question.clone(),
                    }))?;
                    journal.append(Event::Approval(wire::ApprovalRecord {
                        node_id: node.id.clone(),
                        prompt: question.clone(),
                        decision: wire::ApprovalDecision::Pending as i32,
                        decided_by: String::new(),
                        decided_at: None,
                        note: String::new(),
                    }))?;
                    let outcome = approvals.resolve(&node.id, &question).await;
                    let (decision, decided_by, note) = match &outcome {
                        ApprovalOutcome::Approved => (
                            wire::ApprovalDecision::Approved,
                            approvals.describe(),
                            String::new(),
                        ),
                        ApprovalOutcome::Rejected => (
                            wire::ApprovalDecision::Rejected,
                            approvals.describe(),
                            String::new(),
                        ),
                        ApprovalOutcome::Edited(edit) => (
                            wire::ApprovalDecision::Edited,
                            approvals.describe(),
                            edit.clone(),
                        ),
                    };
                    journal.append(Event::Approval(wire::ApprovalRecord {
                        node_id: node.id.clone(),
                        prompt: question,
                        decision: decision as i32,
                        decided_by: decided_by.to_owned(),
                        decided_at: Some(now_ts()),
                        note,
                    }))?;
                    if outcome == ApprovalOutcome::Rejected {
                        journal.append(Event::NodeTransition(wire::NodeTransition {
                            node_id: node.id.clone(),
                            from: wire::NodeState::AwaitingApproval as i32,
                            to: wire::NodeState::Cancelled as i32,
                            note: "rejected by human".to_owned(),
                        }))?;
                        forced_status = Some(wire::RunStatus::Cancelled);
                        notes.push(format!("run cancelled at approval node '{}'", node.id));
                        break;
                    }
                    let mut output = output;
                    output
                        .guard_facts
                        .insert("approval".to_owned(), "approved".to_owned());
                    (output, false)
                }
            };

            for artifact in &output.evidence {
                journal.append(Event::EvidenceRecorded(artifact.clone()))?;
            }
            for iu in &output.ius {
                journal.append(Event::IuRecorded(iu.clone()))?;
            }
            if node.kind == "respond"
                && let Some(text) = &output.text
            {
                final_text = Some(text.clone());
            }
            journal.append(Event::NodeTransition(wire::NodeTransition {
                node_id: node.id.clone(),
                from: wire::NodeState::Running as i32,
                to: wire::NodeState::Succeeded as i32,
                note: String::new(),
            }))?;
            ledger.extend(output.ius.iter().cloned());

            for (target, guard) in graph.successors(ix) {
                let passes = match &guard {
                    None => true,
                    Some(expr) => eval_guard(expr, &output.guard_facts)?,
                };
                if !passes {
                    continue;
                }
                if escalated {
                    *escalations
                        .entry(graph.node(target).id.clone())
                        .or_insert(0) += 1;
                }
                if !queued.contains(&target) {
                    queue.push_back((target, output.guard_facts.clone()));
                    queued.insert(target);
                }
            }

            // Budget enforcement (runtime half of the cycle law).
            let budget = &spec.policy.budget;
            let seconds = started.elapsed().as_secs_f64();
            let tokens = input_tokens + output_tokens;
            let exceeded = budget.max_tokens.is_some_and(|cap| tokens > cap)
                || budget.max_usd.is_some_and(|cap| total_usd > cap)
                || budget.max_seconds.is_some_and(|cap| seconds > cap as f64);
            if exceeded {
                journal.append(Event::Budget(wire::BudgetRecord {
                    scope: "run".to_owned(),
                    tokens_spent: tokens,
                    usd_spent: total_usd,
                    seconds_spent: seconds,
                    tokens_limit: budget.max_tokens.unwrap_or(0),
                    usd_limit: budget.max_usd.unwrap_or(0.0),
                    seconds_limit: budget.max_seconds.unwrap_or(0) as f64,
                }))?;
                forced_status = Some(wire::RunStatus::BudgetExhausted);
                notes.push("run budget exhausted".to_owned());
                break;
            }
        }

        let status = forced_status.unwrap_or(if final_text.is_some() {
            wire::RunStatus::Succeeded
        } else {
            wire::RunStatus::Failed
        });
        journal.append(Event::RunFinished(wire::RunFinished {
            status: status as i32,
            total_input_tokens: input_tokens,
            total_output_tokens: output_tokens,
            total_usd,
            duration_ms: started.elapsed().as_millis() as u64,
            failure_signal_count: failure_signals,
            repair_count: repairs,
            summary: notes.join("; "),
        }))?;

        Ok(RunOutcome {
            run_id,
            status,
            final_text,
            journal_path: journal_path.to_path_buf(),
            input_tokens,
            output_tokens,
            total_usd,
            duration_ms: started.elapsed().as_millis() as u64,
            notes,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{JournalReader, summarize};
    use crate::spec::GraphSpec;

    const DEFAULT_CONVERSATION: &str = include_str!("../../../graphs/conversation.default.toml");

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("graffy-exec-test-{tag}-{}", ulid::Ulid::generate()))
    }

    #[test]
    fn guards_evaluate_equality_inequality_and_truthiness() {
        let mut facts = BTreeMap::new();
        facts.insert("verdict".to_owned(), "pass".to_owned());
        facts.insert("route.escalated".to_owned(), "true".to_owned());

        assert!(eval_guard("verdict == 'pass'", &facts).unwrap());
        assert!(!eval_guard("verdict == 'revise'", &facts).unwrap());
        assert!(eval_guard("verdict != 'revise'", &facts).unwrap());
        assert!(eval_guard("route.escalated", &facts).unwrap());
        assert!(!eval_guard("missing.fact", &facts).unwrap());
        assert!(eval_guard("verdict == \"pass\" && route.escalated", &facts).unwrap());
        assert!(!eval_guard("verdict == 'pass' && missing", &facts).unwrap());
        assert!(eval_guard("verdict == 'no pass'", &facts).is_ok());
        assert!(eval_guard("verdict == pass", &facts).is_err());
        assert!(eval_guard("bad path! == 'x'", &facts).is_err());
    }

    #[test]
    fn tiers_escalate_up_the_ladder_and_clamp() {
        let ladder = vec![
            "fast".to_owned(),
            "balanced".to_owned(),
            "frontier".to_owned(),
        ];
        assert_eq!(resolve_tier(&ladder, "balanced", 0), "balanced");
        assert_eq!(resolve_tier(&ladder, "balanced", 1), "frontier");
        assert_eq!(resolve_tier(&ladder, "balanced", 9), "frontier");
        assert_eq!(resolve_tier(&ladder, "custom", 2), "custom");
        assert_eq!(resolve_tier(&[], "balanced", 2), "balanced");
    }

    #[tokio::test]
    async fn default_conversation_runs_offline_end_to_end() {
        let spec = GraphSpec::from_toml_str(DEFAULT_CONVERSATION).unwrap();
        let journal_path = temp_path("conversation");
        let outcome = Executor::default()
            .run(
                &spec,
                DEFAULT_CONVERSATION,
                RunInput {
                    prompt: "Explain what graffy is in one sentence.".to_owned(),
                    session_id: None,
                },
                &journal_path,
                &OfflineEcho,
                &AutoApprove,
            )
            .await
            .expect("offline conversation run must succeed");

        assert_eq!(outcome.status, wire::RunStatus::Succeeded);
        let final_text = outcome.final_text.expect("respond node must produce text");
        assert!(final_text.contains("offline echo"));

        let events = JournalReader::read_all(&journal_path).unwrap();
        let summary = summarize(&events);
        assert_eq!(summary.status, Some(wire::RunStatus::Succeeded));
        assert!(
            summary.iu_count >= 3,
            "goal + draft + review + response IUs"
        );
        assert!(summary.evidence_count >= 2, "human input + model inference");
        assert!(summary.model_calls >= 2, "draft + verify");
        assert_eq!(summary.routing_decisions, summary.model_calls);
        assert!(matches!(
            events.first().and_then(|e| e.event.as_ref()),
            Some(Event::RunStarted(_))
        ));
        assert!(matches!(
            events.last().and_then(|e| e.event.as_ref()),
            Some(Event::RunFinished(_))
        ));
        std::fs::remove_file(&journal_path).ok();
    }

    /// A judge that never passes: proves the revise loop escalates tiers and
    /// the visit cap halts the run instead of looping forever.
    struct AlwaysRevise;

    #[async_trait::async_trait]
    impl ModelInvoker for AlwaysRevise {
        async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
            let text = if request.purpose == "verify" {
                "REVISE — deterministic test judge rejects everything".to_owned()
            } else {
                format!("draft at tier {}", request.tier)
            };
            Ok(ModelResponse {
                provider: "test".to_owned(),
                model: format!("scripted-{}", request.tier),
                text,
                input_tokens: 1,
                output_tokens: 1,
                cost_usd: 0.0,
                latency_ms: 0,
            })
        }
    }

    #[tokio::test]
    async fn revise_loop_escalates_then_halts_at_visit_cap() {
        let spec = GraphSpec::from_toml_str(DEFAULT_CONVERSATION).unwrap();
        let journal_path = temp_path("revise");
        let outcome = Executor::default()
            .run(
                &spec,
                DEFAULT_CONVERSATION,
                RunInput {
                    prompt: "unpassable".to_owned(),
                    session_id: None,
                },
                &journal_path,
                &AlwaysRevise,
                &AutoApprove,
            )
            .await
            .expect("run completes (as failed) without erroring");

        assert_eq!(outcome.status, wire::RunStatus::Failed);
        assert!(outcome.final_text.is_none());
        assert!(
            outcome
                .notes
                .iter()
                .any(|n| n.contains("exceeded its visit cap")),
            "visit cap must be the halt reason: {:?}",
            outcome.notes
        );

        let events = JournalReader::read_all(&journal_path).unwrap();
        let escalated = events.iter().any(|e| {
            matches!(
                &e.event,
                Some(Event::RoutingDecision(d)) if d.escalation_level > 0
            )
        });
        assert!(escalated, "revise must bump the routing ladder");
        std::fs::remove_file(&journal_path).ok();
    }
}
