//! Research-grade metrics folded from run journals (Phase 3, C5 v1).
//!
//! A journal is already an MCW dataset; this module turns one (or many) into
//! the numbers a researcher needs: outcomes, failure-mode frequencies, repair
//! counts and costs, evidence-level distributions, and convergence behavior.
//! Everything here is *folded from recorded events* — nothing is estimated,
//! nothing is imputed. If a value was never journaled, it is absent, not
//! guessed (the honest-values rule applies to research output most of all).
//!
//! `graffy metrics` in the CLI scans a directory of journals, folds each with
//! [`RunMetrics::fold`], and aggregates with [`AggregateMetrics::from_rows`].
//! `--json` emits the same structures via serde for portable analysis.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::journal::wire;
use graffy_proto::mcw::v1 as mcw;
use wire::run_event::Event;

/// Strip a protobuf enum prefix and lowercase the remainder:
/// `FAILURE_MODE_DRIFT` → `drift`, `EVIDENCE_LEVEL_L2` → `l2`.
fn label(raw: &str, prefix: &str) -> String {
    raw.strip_prefix(prefix).unwrap_or(raw).to_ascii_lowercase()
}

fn failure_mode_label(mode: i32) -> String {
    let name = mcw::FailureMode::try_from(mode)
        .unwrap_or(mcw::FailureMode::Unspecified)
        .as_str_name();
    label(name, "FAILURE_MODE_")
}

fn repair_op_label(op: i32) -> String {
    let name = mcw::RepairOperation::try_from(op)
        .unwrap_or(mcw::RepairOperation::Unspecified)
        .as_str_name();
    label(name, "REPAIR_OPERATION_")
}

fn evidence_level_label(level: i32) -> String {
    let name = mcw::EvidenceLevel::try_from(level)
        .unwrap_or(mcw::EvidenceLevel::Unspecified)
        .as_str_name();
    label(name, "EVIDENCE_LEVEL_")
}

fn run_status_label(status: i32) -> String {
    let name = wire::RunStatus::try_from(status)
        .unwrap_or(wire::RunStatus::Unspecified)
        .as_str_name();
    label(name, "RUN_STATUS_")
}

/// Metrics for a single run, folded from its journal events.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RunMetrics {
    pub run_id: String,
    pub graph_id: String,
    pub graph_name: String,
    pub spec_sha256: String,
    pub graffy_version: String,
    /// Terminal status label (`succeeded`, `failed`, `budget_exhausted`, …);
    /// `unfinished` when the journal has no RunFinished frame.
    pub status: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub events: u64,
    pub model_calls: u64,
    pub tool_calls: u64,
    pub ius_recorded: u64,
    /// Total journaled failure signals, and their breakdown by MCW mode.
    pub failure_signals: u64,
    pub failures_by_mode: BTreeMap<String, u64>,
    /// Repairs executed, by MCW repair operation, plus observed repair cost.
    pub repairs: u64,
    pub repairs_by_op: BTreeMap<String, u64>,
    pub repair_cost_tokens: u64,
    /// Evidence artifacts recorded, by epistemic level (l0–l3).
    pub evidence_artifacts: u64,
    pub evidence_by_level: BTreeMap<String, u64>,
    /// Routing decisions with escalation_level > 0 — each one is a quality
    /// failure that routed UP the ladder instead of bouncing back.
    pub escalations: u64,
    pub max_escalation_level: u32,
    /// Nodes skipped because they exhausted their visit cap (convergence
    /// failure — the run stopped retrying rather than looping forever).
    pub visit_cap_hits: u64,
    /// Highest number of times any single node ran (1 = straight-through).
    pub max_node_visits: u64,
    pub approvals_approved: u64,
    pub approvals_rejected: u64,
    pub hrdm_samples: u64,
}

impl RunMetrics {
    /// Fold a run's journal events into metrics. Purely mechanical: every
    /// number is a count or sum over recorded frames.
    pub fn fold(events: &[wire::RunEvent]) -> Self {
        let mut m = RunMetrics {
            status: "unfinished".to_owned(),
            ..Default::default()
        };
        let mut visits: BTreeMap<String, u64> = BTreeMap::new();
        for frame in events {
            m.events += 1;
            if m.run_id.is_empty() && !frame.run_id.is_empty() {
                m.run_id = frame.run_id.clone();
            }
            let Some(event) = &frame.event else { continue };
            match event {
                Event::RunStarted(man) => {
                    m.graph_id = man.graph_id.clone();
                    m.graph_name = man.graph_name.clone();
                    m.spec_sha256 = man.spec_sha256.clone();
                    m.graffy_version = man.graffy_version.clone();
                }
                Event::RunFinished(fin) => {
                    m.status = run_status_label(fin.status);
                    m.input_tokens = fin.total_input_tokens;
                    m.output_tokens = fin.total_output_tokens;
                    m.cost_usd = fin.total_usd;
                    m.duration_ms = fin.duration_ms;
                }
                Event::ModelCall(_) => m.model_calls += 1,
                Event::ToolCall(_) => m.tool_calls += 1,
                Event::IuRecorded(_) => m.ius_recorded += 1,
                Event::FailureRaised(f) => {
                    m.failure_signals += 1;
                    *m.failures_by_mode
                        .entry(failure_mode_label(f.mode))
                        .or_default() += 1;
                }
                Event::RepairExecuted(r) => {
                    m.repairs += 1;
                    m.repair_cost_tokens += r.cost_tokens;
                    *m.repairs_by_op
                        .entry(repair_op_label(r.operation))
                        .or_default() += 1;
                }
                Event::EvidenceRecorded(a) => {
                    m.evidence_artifacts += 1;
                    *m.evidence_by_level
                        .entry(evidence_level_label(a.level))
                        .or_default() += 1;
                }
                Event::RoutingDecision(d) => {
                    if d.escalation_level > 0 {
                        m.escalations += 1;
                        m.max_escalation_level = m.max_escalation_level.max(d.escalation_level);
                    }
                }
                Event::NodeTransition(t) => {
                    if t.to == wire::NodeState::Running as i32 {
                        let v = visits.entry(t.node_id.clone()).or_default();
                        *v += 1;
                    }
                    if t.to == wire::NodeState::Skipped as i32 && t.note.contains("visit cap") {
                        m.visit_cap_hits += 1;
                    }
                }
                Event::Approval(a) => match wire::ApprovalDecision::try_from(a.decision) {
                    Ok(wire::ApprovalDecision::Approved) | Ok(wire::ApprovalDecision::Edited) => {
                        m.approvals_approved += 1;
                    }
                    Ok(wire::ApprovalDecision::Rejected) => {
                        m.approvals_rejected += 1;
                    }
                    _ => {}
                },
                Event::HrdmSampled(_) => m.hrdm_samples += 1,
                Event::Budget(_) | Event::McwSnapshot(_) => {}
            }
        }
        m.max_node_visits = visits.values().copied().max().unwrap_or(0);
        m
    }
}

/// Cross-run aggregate — the numbers a paper cites.
#[derive(Debug, Default, Serialize)]
pub struct AggregateMetrics {
    pub runs: u64,
    pub runs_by_status: BTreeMap<String, u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub model_calls: u64,
    pub tool_calls: u64,
    pub ius_recorded: u64,
    pub failure_signals: u64,
    pub failures_by_mode: BTreeMap<String, u64>,
    pub repairs: u64,
    pub repairs_by_op: BTreeMap<String, u64>,
    pub repair_cost_tokens: u64,
    pub evidence_artifacts: u64,
    pub evidence_by_level: BTreeMap<String, u64>,
    pub hrdm_samples: u64,
    /// Runs where at least one quality failure escalated the routing ladder.
    pub runs_with_escalation: u64,
    /// Of those, how many still finished `succeeded` — the ladder's win rate.
    /// `null` in JSON when no run escalated (never a fabricated 0 or 1).
    pub escalation_success_rate: Option<f64>,
    /// Success rate of runs that never needed escalation, for comparison.
    pub baseline_success_rate: Option<f64>,
    /// Runs where some node exhausted its visit cap (convergence exhaustion).
    pub runs_with_cap_hits: u64,
    pub mean_escalations_per_run: f64,
}

impl AggregateMetrics {
    pub fn from_rows(rows: &[RunMetrics]) -> Self {
        let mut a = AggregateMetrics::default();
        let mut esc_runs = 0u64;
        let mut esc_succeeded = 0u64;
        let mut base_runs = 0u64;
        let mut base_succeeded = 0u64;
        let mut escalations_total = 0u64;
        for r in rows {
            a.runs += 1;
            *a.runs_by_status.entry(r.status.clone()).or_default() += 1;
            a.input_tokens += r.input_tokens;
            a.output_tokens += r.output_tokens;
            a.cost_usd += r.cost_usd;
            a.model_calls += r.model_calls;
            a.tool_calls += r.tool_calls;
            a.ius_recorded += r.ius_recorded;
            a.failure_signals += r.failure_signals;
            a.repairs += r.repairs;
            a.repair_cost_tokens += r.repair_cost_tokens;
            a.evidence_artifacts += r.evidence_artifacts;
            a.hrdm_samples += r.hrdm_samples;
            for (k, v) in &r.failures_by_mode {
                *a.failures_by_mode.entry(k.clone()).or_default() += v;
            }
            for (k, v) in &r.repairs_by_op {
                *a.repairs_by_op.entry(k.clone()).or_default() += v;
            }
            for (k, v) in &r.evidence_by_level {
                *a.evidence_by_level.entry(k.clone()).or_default() += v;
            }
            escalations_total += r.escalations;
            let succeeded = r.status == "succeeded";
            if r.escalations > 0 {
                esc_runs += 1;
                if succeeded {
                    esc_succeeded += 1;
                }
            } else {
                base_runs += 1;
                if succeeded {
                    base_succeeded += 1;
                }
            }
            if r.visit_cap_hits > 0 {
                a.runs_with_cap_hits += 1;
            }
        }
        a.runs_with_escalation = esc_runs;
        a.escalation_success_rate = (esc_runs > 0).then(|| esc_succeeded as f64 / esc_runs as f64);
        a.baseline_success_rate = (base_runs > 0).then(|| base_succeeded as f64 / base_runs as f64);
        a.mean_escalations_per_run = if a.runs > 0 {
            escalations_total as f64 / a.runs as f64
        } else {
            0.0
        };
        a
    }
}

/// Everything `graffy metrics --json` emits: per-run rows + the aggregate.
#[derive(Debug, Serialize)]
pub struct MetricsReport {
    pub generated_by: String,
    pub runs: Vec<RunMetrics>,
    pub aggregate: AggregateMetrics,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, event: Event) -> wire::RunEvent {
        wire::RunEvent {
            run_id: "run_T".to_owned(),
            seq,
            at: None,
            event: Some(event),
        }
    }

    fn sample_events() -> Vec<wire::RunEvent> {
        vec![
            ev(
                1,
                Event::RunStarted(wire::RunManifest {
                    run_id: "run_T".to_owned(),
                    graph_id: "graffy.test".to_owned(),
                    graph_name: "Test Graph".to_owned(),
                    spec_sha256: "abc123".to_owned(),
                    graffy_version: "0.1.0-test".to_owned(),
                    ..Default::default()
                }),
            ),
            ev(
                2,
                Event::NodeTransition(wire::NodeTransition {
                    node_id: "draft".to_owned(),
                    from: wire::NodeState::Queued as i32,
                    to: wire::NodeState::Running as i32,
                    note: String::new(),
                }),
            ),
            ev(
                3,
                Event::NodeTransition(wire::NodeTransition {
                    node_id: "draft".to_owned(),
                    from: wire::NodeState::Queued as i32,
                    to: wire::NodeState::Running as i32,
                    note: String::new(),
                }),
            ),
            ev(4, Event::ModelCall(wire::ModelCallRecord::default())),
            ev(
                5,
                Event::RoutingDecision(wire::RoutingDecision {
                    node_id: "draft".to_owned(),
                    escalation_level: 1,
                    ..Default::default()
                }),
            ),
            ev(
                6,
                Event::FailureRaised(mcw::FailureSignal {
                    mode: mcw::FailureMode::FalseAlignment as i32,
                    ..Default::default()
                }),
            ),
            ev(
                7,
                Event::RepairExecuted(mcw::RepairAction {
                    operation: mcw::RepairOperation::Regrounding as i32,
                    cost_tokens: 120,
                    successful: true,
                    ..Default::default()
                }),
            ),
            ev(
                8,
                Event::EvidenceRecorded(mcw::EvidenceArtifact {
                    level: mcw::EvidenceLevel::L1Observational as i32,
                    ..Default::default()
                }),
            ),
            ev(9, Event::IuRecorded(mcw::InformationUnit::default())),
            ev(
                10,
                Event::NodeTransition(wire::NodeTransition {
                    node_id: "verify".to_owned(),
                    from: wire::NodeState::Queued as i32,
                    to: wire::NodeState::Skipped as i32,
                    note: "visit cap exceeded".to_owned(),
                }),
            ),
            ev(
                11,
                Event::RunFinished(wire::RunFinished {
                    status: wire::RunStatus::Succeeded as i32,
                    total_input_tokens: 500,
                    total_output_tokens: 200,
                    total_usd: 0.0,
                    duration_ms: 42,
                    failure_signal_count: 1,
                    repair_count: 1,
                    summary: String::new(),
                }),
            ),
        ]
    }

    #[test]
    fn fold_counts_only_what_was_journaled() {
        let m = RunMetrics::fold(&sample_events());
        assert_eq!(m.run_id, "run_T");
        assert_eq!(m.graph_id, "graffy.test");
        assert_eq!(m.status, "succeeded");
        assert_eq!(m.input_tokens, 500);
        assert_eq!(m.model_calls, 1);
        assert_eq!(m.escalations, 1);
        assert_eq!(m.max_escalation_level, 1);
        assert_eq!(m.failure_signals, 1);
        assert_eq!(m.failures_by_mode.get("false_alignment"), Some(&1));
        assert_eq!(m.repairs, 1);
        assert_eq!(m.repairs_by_op.get("regrounding"), Some(&1));
        assert_eq!(m.repair_cost_tokens, 120);
        assert_eq!(m.evidence_by_level.get("l1_observational"), Some(&1));
        assert_eq!(m.visit_cap_hits, 1);
        assert_eq!(m.max_node_visits, 2);
        assert_eq!(m.ius_recorded, 1);
    }

    #[test]
    fn unfinished_journal_reports_unfinished_never_a_guess() {
        let events = &sample_events()[..3];
        let m = RunMetrics::fold(events);
        assert_eq!(m.status, "unfinished");
        assert_eq!(m.input_tokens, 0, "no RunFinished frame means no totals");
    }

    #[test]
    fn aggregate_computes_escalation_efficacy_honestly() {
        let escalated_and_won = RunMetrics {
            status: "succeeded".to_owned(),
            escalations: 2,
            ..Default::default()
        };
        let straight_through_failed = RunMetrics {
            status: "failed".to_owned(),
            visit_cap_hits: 1,
            ..Default::default()
        };
        let a = AggregateMetrics::from_rows(&[escalated_and_won, straight_through_failed]);
        assert_eq!(a.runs, 2);
        assert_eq!(a.runs_with_escalation, 1);
        assert_eq!(a.escalation_success_rate, Some(1.0));
        assert_eq!(a.baseline_success_rate, Some(0.0));
        assert_eq!(a.runs_with_cap_hits, 1);
        assert_eq!(a.mean_escalations_per_run, 1.0);

        let empty = AggregateMetrics::from_rows(&[]);
        assert_eq!(
            empty.escalation_success_rate, None,
            "no escalated runs must serialize as null, never a fabricated rate"
        );
    }
}
