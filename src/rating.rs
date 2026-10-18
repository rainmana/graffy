//! Draft rating-substrate helpers. These validate structure, never assign scores.
use anyhow::{Result, ensure};
use graffy_core::boundary::{
    BoundaryActor, BoundaryProjection, Exchange, RepairEpisode, Visibility,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segmentation {
    pub proposal_sha256: String,
    pub note: String,
    /// Adjacent boundary turn pairs, in chronological order (either direction).
    pub exchanges: Vec<[usize; 2]>,
    /// Inclusive exchange indices; each window contains exactly five exchanges.
    pub windows: Vec<[usize; 2]>,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Episode {
    pub initiating_turn: usize,
    pub closed_at_exchange: Option<usize>,
}

/// Freeze content, visibility, ordering, provenance and all proposed boundaries,
/// not just counts. No model/provider/routing metadata enters the rating snapshot.
pub fn snapshot(p: &BoundaryProjection) -> String {
    json!({
        "version": "boundary-projection@v3",
        "session_id": p.session_id,
        "turns": p.turns.iter().map(|t| json!({
            "actor": format!("{:?}", t.actor), "role": format!("{:?}", t.role),
            "content": t.content, "refs": t.refs,
            "visibility": format!("{:?}", t.visibility),
            "at": t.at.map(|ts| (ts.seconds, ts.nanos)),
            "attempt_group_id": t.attempt_group_id
        })).collect::<Vec<_>>(),
        "exchanges": p.exchanges.iter().map(|e| [e.human_turn.min(e.ai_turn), e.human_turn.max(e.ai_turn)]).collect::<Vec<_>>(),
        "windows": p.complete_windows,
        "trailing_exchanges": p.trailing_exchanges,
        "episodes": p.repair_episodes.iter().map(|e| json!({"initiating_turn": e.initiating_turn, "closed_at_exchange": e.closed_at_exchange})).collect::<Vec<_>>()
    }).to_string()
}

pub fn apply(p: &mut BoundaryProjection, proposed_hash: &str, input: &str) -> Result<String> {
    let correction: Segmentation = serde_json::from_str(input)?;
    ensure!(
        correction.proposal_sha256 == proposed_hash,
        "segmentation is for a different transcript snapshot"
    );
    ensure!(
        !correction.note.trim().is_empty(),
        "segmentation correction requires a note"
    );
    let mut exchanges = Vec::new();
    let mut previous_end = None;
    for [a, b] in &correction.exchanges {
        ensure!(
            a.checked_add(1) == Some(*b) && *b < p.turns.len(),
            "exchange must pair adjacent transcript turns"
        );
        ensure!(
            previous_end.is_none_or(|end| *a > end),
            "exchange pairs must be ordered and non-overlapping"
        );
        let first = &p.turns[*a];
        let second = &p.turns[*b];
        ensure!(
            first.actor != second.actor
                && first.visibility == Visibility::Visible
                && second.visibility == Visibility::Visible,
            "exchange requires visible contributions from both parties"
        );
        let (human_turn, ai_turn) = if first.actor == BoundaryActor::Human {
            (*a, *b)
        } else {
            (*b, *a)
        };
        exchanges.push(Exchange {
            human_turn,
            ai_turn,
            initiated_by: first.actor,
        });
        previous_end = Some(*b);
    }
    let mut previous_window = None;
    for [a, b] in &correction.windows {
        ensure!(
            a.checked_add(4) == Some(*b) && *b < exchanges.len(),
            "window must contain five consecutive exchanges"
        );
        ensure!(
            previous_window.is_none_or(|end| *a > end),
            "windows must be ordered and non-overlapping"
        );
        previous_window = Some(*b);
    }
    let mut previous_start = None;
    for episode in &correction.episodes {
        let turn = p
            .turns
            .get(episode.initiating_turn)
            .ok_or_else(|| anyhow::anyhow!("unknown repair initiation turn"))?;
        ensure!(
            turn.actor == BoundaryActor::Human && turn.visibility == Visibility::Visible,
            "repair initiation requires a visible human turn"
        );
        ensure!(
            previous_start.is_none_or(|start| episode.initiating_turn > start),
            "episodes must have distinct ordered initiations"
        );
        if let Some(end) = episode.closed_at_exchange {
            let exchange = exchanges
                .get(end)
                .ok_or_else(|| anyhow::anyhow!("unknown closure exchange"))?;
            ensure!(
                exchange.human_turn.min(exchange.ai_turn) > episode.initiating_turn,
                "closure requires a later completed exchange"
            );
        }
        previous_start = Some(episode.initiating_turn);
    }
    let record = serde_json::to_string(&correction)?;
    p.exchanges = exchanges;
    p.complete_windows = correction
        .windows
        .into_iter()
        .map(|[a, b]| (a, b))
        .collect();
    p.trailing_exchanges = p.exchanges.len() - p.complete_windows.len() * 5;
    p.repair_episodes = correction
        .episodes
        .into_iter()
        .map(|e| RepairEpisode {
            initiating_turn: e.initiating_turn,
            closed_at_exchange: e.closed_at_exchange,
        })
        .collect();
    Ok(record)
}

/// H/M evidence stays inside the rated window. D can cite an introduction
/// before the window, but discovery must be inside it, at least three
/// completed exchanges later. The rater still judges semantic continuity.
pub fn citation_refs(
    p: &BoundaryProjection,
    input: &str,
    window: (usize, usize),
    proxy: &str,
) -> Option<Vec<String>> {
    let indices: Vec<usize> = input
        .split(',')
        .map(|s| s.trim().parse::<usize>())
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    if indices.is_empty() {
        return None;
    }
    let exchange_of = |idx| {
        p.exchanges
            .iter()
            .position(|e| e.human_turn == idx || e.ai_turn == idx)
    };
    let inside = |idx| exchange_of(idx).is_some_and(|x| (window.0..=window.1).contains(&x));
    if proxy == "D" {
        if indices.len() != 2 || indices[0] >= indices[1] || !inside(indices[1]) {
            return None;
        }
        let introduction = exchange_of(indices[0])?;
        let discovery = exchange_of(indices[1])?;
        if discovery.saturating_sub(introduction) < 3 {
            return None;
        }
    } else if !indices.iter().all(|i| inside(*i)) {
        return None;
    }
    let mut refs = Vec::new();
    for idx in indices {
        let t = p.turns.get(idx)?;
        if t.visibility != Visibility::Visible || (proxy == "M" && t.actor != BoundaryActor::Human)
        {
            return None;
        }
        refs.extend(t.refs.iter().cloned());
    }
    (!refs.is_empty()).then_some(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graffy_core::boundary::{BoundaryRole, BoundaryTurn};
    fn projection() -> BoundaryProjection {
        let mut p = BoundaryProjection::default();
        for i in 0..10 {
            p.turns.push(BoundaryTurn {
                actor: if i % 2 == 0 {
                    BoundaryActor::Human
                } else {
                    BoundaryActor::Ai
                },
                role: if i % 2 == 0 {
                    BoundaryRole::Prompt
                } else {
                    BoundaryRole::Response
                },
                attempt_group_id: format!("g{}", i / 2),
                content: format!("turn {i}"),
                at: None,
                refs: vec![format!("journal://r/{i}")],
                visibility: Visibility::Visible,
            });
        }
        p.exchanges = (0..5)
            .map(|x| Exchange {
                human_turn: 2 * x,
                ai_turn: 2 * x + 1,
                initiated_by: BoundaryActor::Human,
            })
            .collect();
        p.complete_windows.push((0, 4));
        p
    }
    #[test]
    fn snapshot_commits_to_content_and_refs() {
        let mut p = projection();
        let before = snapshot(&p);
        p.turns[0].content.push('!');
        assert_ne!(before, snapshot(&p));
        let before = snapshot(&p);
        p.turns[0].refs.push("different".into());
        assert_ne!(before, snapshot(&p));
    }
    #[test]
    fn correction_adds_missed_episode_and_rejects_stale_or_invalid_units() {
        let mut p = projection();
        let hash = graffy_core::exec::sha256_hex(snapshot(&p).as_bytes());
        let mut c = json!({"proposal_sha256":hash,"note":"human identified repair","exchanges":[[0,1],[2,3],[4,5],[6,7],[8,9]],"windows":[[0,4]],"episodes":[{"initiating_turn":2,"closed_at_exchange":3}]});
        apply(&mut p, &hash, &c.to_string()).unwrap();
        assert_eq!(p.repair_episodes[0].initiating_turn, 2);
        assert!(apply(&mut p, "stale", &c.to_string()).is_err());
        c["windows"] = json!([[0, 3]]);
        assert!(apply(&mut p, &hash, &c.to_string()).is_err());
        c["windows"] = json!([]);
        c["exchanges"] = json!([[0, 3]]);
        assert!(apply(&mut p, &hash, &c.to_string()).is_err());
    }
    #[test]
    fn citations_check_scope_late_discovery_and_human_blame() {
        let p = projection();
        assert!(citation_refs(&p, "0,7", (0, 4), "D").is_some());
        for s in ["0,3", "7,0", "0", "0,99"] {
            assert!(citation_refs(&p, s, (0, 4), "D").is_none());
        }
        assert!(citation_refs(&p, "1", (0, 4), "M").is_none());
        assert!(citation_refs(&p, "0", (1, 4), "H").is_none());
        assert!(citation_refs(&p, "2", (0, 4), "M").is_some());
    }
}
