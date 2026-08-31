//! External-boundary transcript projection (HRDM adaptation §2).
//!
//! Canonical H/R/D/M is scored only over contributions that actually crossed
//! the HCW/ACW boundary. A graffy run, node, model call, retry attempt, or
//! journal file is an implementation container — NOT automatically a
//! canonical turn or exchange. This module derives `BoundaryTurn` records
//! from full journal event streams, pairs them into completed exchanges,
//! proposes complete five-exchange windows and external repair episodes, and
//! separates internal orchestration (drafts, judge critiques, automatic
//! retries) into a distinct telemetry class that is never silently relabeled
//! as canonical data.
//!
//! Everything here is a MECHANICAL PROPOSAL for a human rater to confirm or
//! correct (adaptation §5.1). Proposals never become scores by themselves,
//! and when boundary visibility cannot be proven the projection says
//! `Unknown` — it does not guess.

use crate::journal::wire;
use graffy_proto::mcw::v1 as mcw;
use graffy_proto::prost_types;
use wire::run_event::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryActor {
    Human,
    Ai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryRole {
    Prompt,
    /// A human correction/clarification captured at intake (source = human).
    Correction,
    /// A human approval decision (approve / reject / edit) on a gate.
    ApprovalDecision,
    /// The verified response actually shown to the human.
    VerifiedResponse,
}

/// Whether the content provably crossed the human/AI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    /// Cannot be proven from the journal — affected units are unratable.
    Unknown,
}

/// One contribution that crossed (or may have crossed) the boundary.
#[derive(Debug, Clone)]
pub struct BoundaryTurn {
    pub actor: BoundaryActor,
    /// The external attempt group whose runs produced this turn — duplicate
    /// prompts collapse ONLY within one group (P0.1); identical text in two
    /// real exchanges stays two turns.
    pub attempt_group_id: String,
    pub role: BoundaryRole,
    /// Verbatim boundary-visible content.
    pub content: String,
    pub at: Option<prost_types::Timestamp>,
    /// Immutable journal references: `journal://<run_id>/<seq>`. Duplicate
    /// prompts across automatic retry attempts collapse into ONE turn whose
    /// refs list every occurrence (dedup provenance).
    pub refs: Vec<String>,
    pub visibility: Visibility,
}

/// An adjacent human→AI pair of boundary turns (canonical exchange).
#[derive(Debug, Clone)]
pub struct Exchange {
    pub human_turn: usize,
    pub ai_turn: usize,
}

/// A candidate EXTERNAL repair episode: opens at a human repair-initiating
/// boundary turn; closes at the first later completed exchange that does not
/// re-raise a correction (v1 proxy for "both parties proceed without
/// re-raising"). `closed_at_exchange = None` means the episode never closed
/// in the observed session.
#[derive(Debug, Clone)]
pub struct RepairEpisode {
    pub initiating_turn: usize,
    pub closed_at_exchange: Option<usize>,
}

/// Internal orchestration telemetry — reported SEPARATELY from canonical
/// units, never as H/R/D/M (adaptation §1.2).
#[derive(Debug, Default, Clone)]
pub struct InternalTelemetry {
    pub runs: u64,
    /// Runs that carried automatic retry feedback (internal repair attempts).
    pub retry_runs: u64,
    pub failure_signals: u64,
    pub repair_actions: u64,
    /// Runs that ended without a verified response (not exchanges).
    pub runs_without_verified_response: u64,
}

/// The full projection for one session.
#[derive(Debug, Default, Clone)]
pub struct BoundaryProjection {
    pub session_id: String,
    pub turns: Vec<BoundaryTurn>,
    /// Completed exchanges only (both boundary turns present).
    pub exchanges: Vec<Exchange>,
    /// COMPLETE five-exchange windows, as inclusive (start, end) exchange
    /// indices. An incomplete trailing group is never included.
    pub complete_windows: Vec<(usize, usize)>,
    /// Exchanges left over after the last complete window (reported, not
    /// silently scored).
    pub trailing_exchanges: usize,
    pub repair_episodes: Vec<RepairEpisode>,
    pub internal: InternalTelemetry,
    /// Blinding note: this projection intentionally carries NO provider,
    /// model, tier, routing, spec/graph-name, judge-verdict, or
    /// machine-assigned failure-label fields (adaptation §5.3).
    pub blinding_profile: &'static str,
}

pub const BLINDING_PROFILE: &str = "boundary-v1";

/// Order a session's runs by validated manifest timestamps (falling back to
/// the first event's timestamp), tie-broken deterministically by run id —
/// NEVER by filename (P0.1: custom journal names cannot reorder a session).
pub fn order_runs(runs: &mut [Vec<wire::RunEvent>]) {
    fn key(events: &[wire::RunEvent]) -> (i64, i32, String) {
        let run_id = events
            .iter()
            .find(|e| !e.run_id.is_empty())
            .map(|e| e.run_id.clone())
            .unwrap_or_default();
        let ts = events
            .iter()
            .find_map(|e| match &e.event {
                Some(Event::RunStarted(m)) => m.started_at,
                _ => None,
            })
            .or_else(|| events.iter().find_map(|e| e.at));
        match ts {
            Some(t) => (t.seconds, t.nanos, run_id),
            None => (i64::MAX, i32::MAX, run_id),
        }
    }
    runs.sort_by_key(|events| key(events));
}

/// The visible boundary conversation as (actor label, verbatim content)
/// pairs, in order — the hydration payload for session continuation.
pub fn hydration_from(projection: &BoundaryProjection) -> Vec<(String, String)> {
    projection
        .turns
        .iter()
        .filter(|t| t.visibility == Visibility::Visible)
        .map(|t| {
            (
                match t.actor {
                    BoundaryActor::Human => "human".to_owned(),
                    BoundaryActor::Ai => "ai".to_owned(),
                },
                t.content.clone(),
            )
        })
        .collect()
}

fn is_human(source: &Option<mcw::ActorRef>) -> bool {
    source
        .as_ref()
        .is_some_and(|a| a.kind == mcw::actor_ref::ActorKind::Human as i32)
}

/// Project a session's runs (each run's full event stream, in session order)
/// into the external-boundary transcript.
pub fn project(runs: &[Vec<wire::RunEvent>]) -> BoundaryProjection {
    let mut p = BoundaryProjection {
        blinding_profile: BLINDING_PROFILE,
        ..Default::default()
    };

    for events in runs {
        p.internal.runs += 1;
        let run_id = events
            .iter()
            .find(|e| !e.run_id.is_empty())
            .map(|e| e.run_id.clone())
            .unwrap_or_default();
        // Attempt-group identity: from the manifest when present; legacy
        // journals (pre-P0.1) fall back to the run id, i.e. no cross-run
        // dedup — the conservative reading.
        let group = events
            .iter()
            .find_map(|e| match &e.event {
                Some(Event::RunStarted(m)) if !m.attempt_group_id.is_empty() => {
                    Some(m.attempt_group_id.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| run_id.clone());
        let kind = events
            .iter()
            .find_map(|e| match &e.event {
                Some(Event::RunStarted(m)) => wire::RunKind::try_from(m.run_kind).ok(),
                _ => None,
            })
            .unwrap_or(wire::RunKind::Unspecified);
        let mut run_succeeded = false;
        let mut had_retry_feedback = kind == wire::RunKind::AutomaticRetry;
        // Candidate turns from this run, applied after we know run status.
        let mut human_candidates: Vec<BoundaryTurn> = Vec::new();
        let mut response_candidate: Option<BoundaryTurn> = None;

        for frame in events {
            match &frame.event {
                Some(Event::RunStarted(man)) => {
                    if p.session_id.is_empty() {
                        p.session_id = man.session_id.clone();
                    }
                }
                Some(Event::RunFinished(fin)) => {
                    run_succeeded = fin.status == wire::RunStatus::Succeeded as i32;
                }
                Some(Event::IuRecorded(iu)) => {
                    let role = iu.attributes.get("role").map(String::as_str);
                    if iu.kind == mcw::IuKind::Goal as i32 && is_human(&iu.source) {
                        human_candidates.push(BoundaryTurn {
                            actor: BoundaryActor::Human,
                            attempt_group_id: group.clone(),
                            role: BoundaryRole::Prompt,
                            content: iu.payload_text.clone(),
                            at: iu.created_at,
                            refs: vec![format!("journal://{}/{}", frame.run_id, frame.seq)],
                            visibility: Visibility::Visible,
                        });
                    } else if iu.kind == mcw::IuKind::Correction as i32 {
                        if is_human(&iu.source) {
                            // A correction the HUMAN externalized.
                            human_candidates.push(BoundaryTurn {
                                actor: BoundaryActor::Human,
                                attempt_group_id: group.clone(),
                                role: BoundaryRole::Correction,
                                content: iu.payload_text.clone(),
                                at: iu.created_at,
                                refs: vec![format!("journal://{}/{}", frame.run_id, frame.seq)],
                                visibility: Visibility::Visible,
                            });
                        } else {
                            // Automatic retry feedback (judge critique carried
                            // by the harness): INTERNAL, never a boundary turn.
                            had_retry_feedback = true;
                        }
                    } else if role == Some("response") {
                        response_candidate = Some(BoundaryTurn {
                            actor: BoundaryActor::Ai,
                            attempt_group_id: group.clone(),
                            role: BoundaryRole::VerifiedResponse,
                            content: iu.payload_text.clone(),
                            at: iu.created_at,
                            refs: vec![format!("journal://{}/{}", frame.run_id, frame.seq)],
                            visibility: Visibility::Visible,
                        });
                    }
                }
                Some(Event::Approval(a)) => {
                    // A human decision at a gate crossed the boundary in both
                    // directions; record the human's contribution.
                    human_candidates.push(BoundaryTurn {
                        actor: BoundaryActor::Human,
                        attempt_group_id: group.clone(),
                        role: BoundaryRole::ApprovalDecision,
                        content: format!(
                            "[approval:{}] {}",
                            wire::ApprovalDecision::try_from(a.decision)
                                .unwrap_or(wire::ApprovalDecision::Unspecified)
                                .as_str_name()
                                .trim_start_matches("APPROVAL_DECISION_")
                                .to_ascii_lowercase(),
                            a.note
                        ),
                        at: a.decided_at,
                        refs: vec![format!("journal://{}/{}", frame.run_id, frame.seq)],
                        visibility: Visibility::Visible,
                    });
                }
                Some(Event::FailureRaised(_)) => p.internal.failure_signals += 1,
                Some(Event::RepairExecuted(_)) => p.internal.repair_actions += 1,
                _ => {}
            }
        }
        let _ = run_id;
        if had_retry_feedback {
            p.internal.retry_runs += 1;
        }

        // Dedup: the same human content re-entering across automatic attempts
        // stays ONE boundary turn — later occurrences append refs only.
        for cand in human_candidates {
            if let Some(existing) = p.turns.iter_mut().find(|t| {
                t.actor == BoundaryActor::Human
                    && t.role == cand.role
                    && t.content == cand.content
                    && t.attempt_group_id == cand.attempt_group_id
            }) {
                existing.refs.extend(cand.refs);
            } else {
                p.turns.push(cand);
            }
        }
        // The AI's contribution crossed the boundary only if a verified
        // response was actually produced (shown to the human). Failed
        // attempts contribute no AI boundary turn — and are not exchanges.
        match (run_succeeded, response_candidate) {
            (true, Some(resp)) => p.turns.push(resp),
            _ => p.internal.runs_without_verified_response += 1,
        }
    }

    // Exchanges: each AI turn closes an exchange with the nearest preceding
    // unpaired human turn (canonical adjacent pair).
    let mut unpaired_human: Option<usize> = None;
    for (idx, turn) in p.turns.iter().enumerate() {
        match turn.actor {
            BoundaryActor::Human => unpaired_human = Some(idx),
            BoundaryActor::Ai => {
                if let Some(h) = unpaired_human.take() {
                    p.exchanges.push(Exchange {
                        human_turn: h,
                        ai_turn: idx,
                    });
                }
            }
        }
    }

    // Complete five-exchange windows ONLY.
    let full = p.exchanges.len() / 5;
    for w in 0..full {
        p.complete_windows.push((w * 5, w * 5 + 4));
    }
    p.trailing_exchanges = p.exchanges.len() % 5;

    // External repair episodes: opened by a human Correction boundary turn;
    // closed at the first later completed exchange whose human turn is not
    // itself another correction (v1 closure proxy — the rater confirms).
    for (idx, turn) in p.turns.iter().enumerate() {
        if turn.actor == BoundaryActor::Human && turn.role == BoundaryRole::Correction {
            let closed = p
                .exchanges
                .iter()
                .position(|ex| {
                    ex.human_turn > idx && p.turns[ex.human_turn].role != BoundaryRole::Correction
                })
                .or_else(|| {
                    // The correction itself opens an exchange that completes
                    // and is followed by no further correction: closed there.
                    p.exchanges.iter().position(|ex| {
                        ex.human_turn == idx
                            && !p.turns[ex.ai_turn + 1..]
                                .iter()
                                .any(|t| t.role == BoundaryRole::Correction)
                    })
                });
            p.repair_episodes.push(RepairEpisode {
                initiating_turn: idx,
                closed_at_exchange: closed,
            });
        }
    }

    p
}

/// Canonical R_ev: distinct repair episodes per 10 completed exchanges,
/// computed ONLY from confirmed external episodes (never internal retries).
/// `None` when there are no exchanges — a rate over nothing is not 0.
pub fn r_ev(confirmed_episodes: usize, exchanges: usize) -> Option<f64> {
    (exchanges > 0).then(|| confirmed_episodes as f64 / (exchanges as f64 / 10.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ev(run: &str, seq: u64, event: Event) -> wire::RunEvent {
        wire::RunEvent {
            run_id: run.to_owned(),
            seq,
            at: None,
            event: Some(event),
        }
    }

    fn manifest(run: &str) -> Event {
        manifest_in_group(run, run)
    }

    fn manifest_in_group(run: &str, group: &str) -> Event {
        Event::RunStarted(wire::RunManifest {
            run_id: run.to_owned(),
            session_id: "ses_T".to_owned(),
            graph_id: "graffy.test".to_owned(),
            graph_name: "SECRET-ARM-B-frontier".to_owned(), // must never leak
            attempt_group_id: group.to_owned(),
            ..Default::default()
        })
    }

    fn human_goal(text: &str) -> Event {
        Event::IuRecorded(mcw::InformationUnit {
            id: format!("iu_{text}"),
            kind: mcw::IuKind::Goal as i32,
            payload_text: text.to_owned(),
            source: Some(mcw::ActorRef {
                kind: mcw::actor_ref::ActorKind::Human as i32,
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    fn graph_correction(text: &str) -> Event {
        Event::IuRecorded(mcw::InformationUnit {
            kind: mcw::IuKind::Correction as i32,
            payload_text: text.to_owned(),
            source: Some(mcw::ActorRef {
                kind: mcw::actor_ref::ActorKind::Graph as i32,
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    fn human_correction(text: &str) -> Event {
        Event::IuRecorded(mcw::InformationUnit {
            kind: mcw::IuKind::Correction as i32,
            payload_text: text.to_owned(),
            source: Some(mcw::ActorRef {
                kind: mcw::actor_ref::ActorKind::Human as i32,
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    fn response(text: &str) -> Event {
        let mut attributes = HashMap::new();
        attributes.insert("role".to_owned(), "response".to_owned());
        Event::IuRecorded(mcw::InformationUnit {
            kind: mcw::IuKind::Other as i32,
            payload_text: text.to_owned(),
            attributes,
            ..Default::default()
        })
    }

    fn finished(ok: bool) -> Event {
        Event::RunFinished(wire::RunFinished {
            status: if ok {
                wire::RunStatus::Succeeded as i32
            } else {
                wire::RunStatus::Failed as i32
            },
            ..Default::default()
        })
    }

    /// One exchange: prompt in, verified response out.
    fn ok_run(run: &str, prompt: &str, resp: &str) -> Vec<wire::RunEvent> {
        vec![
            ev(run, 1, manifest(run)),
            ev(run, 2, human_goal(prompt)),
            ev(run, 3, response(resp)),
            ev(run, 4, finished(true)),
        ]
    }

    #[test]
    fn retry_attempts_collapse_to_one_exchange() {
        // Attempt 1: fails (no verified response). Attempt 2: same prompt +
        // automatic feedback, succeeds. Canonically: ONE human turn (deduped,
        // refs from both attempts), ONE AI turn, ONE exchange.
        let attempt1 = vec![
            ev("r1", 1, manifest_in_group("r1", "g1")),
            ev("r1", 2, human_goal("same prompt")),
            ev("r1", 3, finished(false)),
        ];
        let attempt2 = vec![
            ev("r2", 1, manifest_in_group("r2", "g1")),
            ev("r2", 2, graph_correction("judge critique — internal")),
            ev("r2", 3, human_goal("same prompt")),
            ev("r2", 4, response("verified answer")),
            ev("r2", 5, finished(true)),
        ];
        let p = project(&[attempt1, attempt2]);
        let humans = p
            .turns
            .iter()
            .filter(|t| t.actor == BoundaryActor::Human)
            .count();
        assert_eq!(humans, 1, "duplicate prompt collapses to one human turn");
        assert_eq!(
            p.turns
                .iter()
                .find(|t| t.actor == BoundaryActor::Human)
                .unwrap()
                .refs
                .len(),
            2,
            "dedup keeps provenance from both attempts"
        );
        assert_eq!(p.exchanges.len(), 1, "one completed exchange, not two");
        assert_eq!(p.internal.retry_runs, 1, "retry stays internal telemetry");
        assert_eq!(p.internal.runs_without_verified_response, 1);
        assert!(
            p.repair_episodes.is_empty(),
            "automatic retry is NOT an external repair episode"
        );
    }

    #[test]
    fn same_text_in_two_real_exchanges_stays_two_turns() {
        // P0.1 regression: dedup may collapse duplicates only WITHIN one
        // automatic attempt group. Two REAL exchanges that happen to use
        // identical text are two distinct human turns.
        let p = project(&[
            ok_run("r1", "what time is it?", "noon"),
            ok_run("r2", "what time is it?", "still noon"),
        ]);
        let humans = p
            .turns
            .iter()
            .filter(|t| t.actor == BoundaryActor::Human)
            .count();
        assert_eq!(humans, 2, "distinct exchanges must not be merged");
        assert_eq!(p.exchanges.len(), 2);
    }

    #[test]
    fn windows_require_five_complete_exchanges() {
        let mut runs = Vec::new();
        for i in 0..6 {
            runs.push(ok_run(&format!("r{i}"), &format!("q{i}"), &format!("a{i}")));
        }
        let p = project(&runs);
        assert_eq!(p.exchanges.len(), 6);
        assert_eq!(p.complete_windows, vec![(0, 4)], "only the full window");
        assert_eq!(
            p.trailing_exchanges, 1,
            "trailing exchange reported, not scored"
        );
    }

    #[test]
    fn failed_runs_are_not_exchanges() {
        let failed = vec![
            ev("rf", 1, manifest("rf")),
            ev("rf", 2, human_goal("q")),
            ev("rf", 3, finished(false)),
        ];
        let p = project(&[failed]);
        assert_eq!(p.exchanges.len(), 0);
        assert_eq!(p.internal.runs_without_verified_response, 1);
    }

    #[test]
    fn human_corrections_open_external_episodes_and_graph_ones_do_not() {
        let correction_run = vec![
            ev("rc", 1, manifest("rc")),
            ev("rc", 2, human_correction("no — I meant the OTHER config")),
            ev("rc", 3, response("fixed answer")),
            ev("rc", 4, finished(true)),
        ];
        let follow_up = ok_run("rn", "next question", "next answer");
        let p = project(&[
            ok_run("r0", "first question", "first answer"),
            correction_run,
            follow_up,
        ]);
        assert_eq!(p.repair_episodes.len(), 1, "one external episode");
        assert!(p.repair_episodes[0].closed_at_exchange.is_some());
        assert_eq!(r_ev(1, p.exchanges.len()), Some(1.0 / 0.3));
    }

    #[test]
    fn order_runs_uses_timestamps_never_input_order() {
        fn stamped(run: &str, secs: i64) -> Vec<wire::RunEvent> {
            vec![ev(
                run,
                1,
                Event::RunStarted(wire::RunManifest {
                    run_id: run.to_owned(),
                    session_id: "ses_T".to_owned(),
                    started_at: Some(prost_types::Timestamp {
                        seconds: secs,
                        nanos: 0,
                    }),
                    ..Default::default()
                }),
            )]
        }
        // Handed over in reverse (as a hostile filename sort would produce).
        let mut runs = vec![
            stamped("z-late", 300),
            stamped("a-mid", 200),
            stamped("m-early", 100),
        ];
        order_runs(&mut runs);
        let ids: Vec<&str> = runs.iter().map(|r| r[0].run_id.as_str()).collect();
        assert_eq!(ids, vec!["m-early", "a-mid", "z-late"]);
        // Deterministic tie-break by run id, never input position.
        let mut tied = vec![stamped("bbb", 500), stamped("aaa", 500)];
        order_runs(&mut tied);
        assert_eq!(tied[0][0].run_id, "aaa");
    }

    #[test]
    fn five_dependent_exchanges_project_and_hydrate() {
        let mut runs = Vec::new();
        for i in 0..5 {
            runs.push(ok_run(&format!("r{i}"), &format!("q{i}"), &format!("a{i}")));
        }
        let p = project(&runs);
        assert_eq!(p.exchanges.len(), 5);
        assert_eq!(p.complete_windows, vec![(0, 4)]);
        assert_eq!(p.internal.retry_runs, 0);
        let hydration = hydration_from(&p);
        assert_eq!(hydration.len(), 10, "5 prompts + 5 responses, in order");
        assert_eq!(hydration[0], ("human".to_owned(), "q0".to_owned()));
        assert_eq!(hydration[1], ("ai".to_owned(), "a0".to_owned()));
    }

    #[test]
    fn projection_is_condition_blind_by_construction() {
        let p = project(&[ok_run("r1", "question", "answer")]);
        let dump = format!("{p:?}");
        assert!(
            !dump.contains("SECRET-ARM-B-frontier"),
            "graph/spec names must not leak into the projection"
        );
        assert!(!dump.to_lowercase().contains("provider"));
        assert_eq!(p.blinding_profile, BLINDING_PROFILE);
    }
}
