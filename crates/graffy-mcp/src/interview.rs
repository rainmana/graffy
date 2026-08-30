//! The usage interview, v1 (design doc §4/§8) — pure decision logic.
//!
//! The CLI owns the asking; this module owns the judging, so the
//! MCW-instrumented parts (answer classification, the False Alignment guard)
//! are unit-tested without a terminal. The interview becomes a first-class
//! graph once the human-input node kind lands; until then the CLI form keeps
//! the same semantics: conservative annotations win over optimistic humans.

use crate::DiscoveredTool;

/// What a Q2 answer ("does it change things or just look them up?") claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedRole {
    Evidence,
    Effector,
    /// MCW ambiguity signal → the CLI asks the Disambiguation follow-up.
    Ambiguous,
}

/// Classify a free-text Q2 answer. Deliberately word-boundary-ish and
/// conservative: anything unrecognized is Ambiguous, never silently safe.
pub fn classify_change_answer(answer: &str) -> ClaimedRole {
    let a = answer.to_lowercase();
    let evidence_hits = [
        "read",
        "look",
        "search",
        "fetch",
        "query",
        "just looks",
        "nothing",
        "no",
    ];
    let effector_hits = [
        "write", "change", "send", "delete", "create", "deploy", "post", "modif", "yes",
    ];
    let ev = evidence_hits.iter().any(|w| a.contains(w));
    let ef = effector_hits.iter().any(|w| a.contains(w));
    match (ev, ef) {
        (true, false) => ClaimedRole::Evidence,
        (false, true) => ClaimedRole::Effector,
        _ => ClaimedRole::Ambiguous,
    }
}

/// The §8 False Alignment guard: a human claiming "read-only" while any tool
/// declares `destructiveHint` is an early failure-mode signal. The
/// conservative annotation wins until explicitly overridden.
pub fn false_alignment(claimed: ClaimedRole, tools: &[DiscoveredTool]) -> Option<Vec<String>> {
    if claimed != ClaimedRole::Evidence {
        return None;
    }
    let destructive: Vec<String> = tools
        .iter()
        .filter(|t| t.destructive == Some(true))
        .map(|t| t.name.clone())
        .collect();
    if destructive.is_empty() {
        None
    } else {
        Some(destructive)
    }
}

/// Resolve the final server-default role from the interview.
/// `override_confirmed` is only true after the human explicitly overrides a
/// surfaced conflict — silence keeps the conservative answer.
pub fn resolve_role(
    claimed: ClaimedRole,
    conflict: bool,
    override_confirmed: bool,
) -> &'static str {
    match claimed {
        ClaimedRole::Effector => "effector",
        ClaimedRole::Evidence if conflict && !override_confirmed => "effector",
        ClaimedRole::Evidence => "evidence",
        ClaimedRole::Ambiguous => "effector",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destructive_tool() -> DiscoveredTool {
        DiscoveredTool {
            name: "delete_repo".into(),
            description: String::new(),
            read_only: None,
            destructive: Some(true),
            schema_json: "{}".into(),
        }
    }

    fn benign_tool() -> DiscoveredTool {
        DiscoveredTool {
            name: "search".into(),
            description: String::new(),
            read_only: Some(true),
            destructive: Some(false),
            schema_json: "{}".into(),
        }
    }

    #[test]
    fn answers_classify_conservatively() {
        assert_eq!(
            classify_change_answer("it just looks things up"),
            ClaimedRole::Evidence
        );
        assert_eq!(
            classify_change_answer("it can delete branches"),
            ClaimedRole::Effector
        );
        assert_eq!(
            classify_change_answer("it kind of does stuff"),
            ClaimedRole::Ambiguous
        );
        assert_eq!(classify_change_answer(""), ClaimedRole::Ambiguous);
        // Mixed signals are ambiguity, not optimism.
        assert_eq!(
            classify_change_answer("reads and writes"),
            ClaimedRole::Ambiguous
        );
    }

    #[test]
    fn false_alignment_fires_only_on_contradicted_read_only_claims() {
        let tools = vec![benign_tool(), destructive_tool()];
        let hit = false_alignment(ClaimedRole::Evidence, &tools);
        assert_eq!(hit, Some(vec!["delete_repo".to_owned()]));
        assert!(false_alignment(ClaimedRole::Effector, &tools).is_none());
        assert!(false_alignment(ClaimedRole::Evidence, &[benign_tool()]).is_none());
    }

    #[test]
    fn conservative_annotation_wins_unless_explicitly_overridden() {
        assert_eq!(resolve_role(ClaimedRole::Evidence, true, false), "effector");
        assert_eq!(resolve_role(ClaimedRole::Evidence, true, true), "evidence");
        assert_eq!(
            resolve_role(ClaimedRole::Evidence, false, false),
            "evidence"
        );
        assert_eq!(
            resolve_role(ClaimedRole::Ambiguous, false, false),
            "effector"
        );
        assert_eq!(
            resolve_role(ClaimedRole::Effector, false, false),
            "effector"
        );
    }
}
