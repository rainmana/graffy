//! Context policy for continued sessions (issue #30, ADR-0014) — a stop-gap.
//!
//! A continued turn carries the session's visible boundary history into the
//! model's context. Left alone, that history grows with every turn until a
//! provider request overflows or the adapter's input reserve eats the whole
//! token budget. This module bounds it, and it never does so quietly:
//!
//! * the request plus history is **measured** before the turn starts, exactly
//!   as a model node assembles it ([`assemble`]), in UTF-8 bytes — the same
//!   deliberately conservative "one token per byte" unit the provider adapter
//!   reserves input with. graffy has no tokenizer, so it does not pretend to
//!   count tokens. A node's own system text, context from earlier steps and
//!   review feedback are not part of the measure: they belong to the graph
//!   and do not grow with the conversation;
//! * over the limit, a **bounded selection** is made: the opening human turn
//!   (where a conversation's standing constraints are usually set) plus as many
//!   of the most recent turns as fit;
//! * the omission is **disclosed** in the selection itself, as a line the model
//!   reads and the journal records, and to the human by the frontend;
//! * when even the request plus the opening turn cannot fit, the turn
//!   **stops** before any model is called.
//!
//! MCW: an undisclosed trim is Overcompression that neither party can see
//! (Constraint Opacity). Disclosure does not restore the omitted turns; it
//! makes the loss visible so the human can repair it by restating.
//!
//! What this is not: compaction. Nothing is summarized, and graffy does not
//! infer from wording which turns hold constraints. An IU-aware compaction
//! graph is the planned replacement (ROADMAP).

use crate::spec::GraphSpec;

/// Actor label of the disclosure line placed where history was omitted.
pub const POLICY_ACTOR: &str = "graffy";

/// Actor label of human turns in a hydrated history.
const HUMAN_ACTOR: &str = "human";

const HISTORY_HEADER: &str = "Conversation so far (visible boundary exchanges):\n";
const REQUEST_HEADER: &str = "\n\nCurrent request:\n";

/// One history entry as a model node renders it into a prompt.
pub fn history_line(actor: &str, content: &str) -> String {
    format!("{actor}: {content}")
}

/// The request with the conversation so far in front of it — the one place
/// this text is built. Model nodes call it, and [`select`] measures by the
/// same layout, so what is measured is what is sent.
pub fn assemble(request: &str, history_lines: &[String]) -> String {
    if history_lines.is_empty() {
        return request.to_owned();
    }
    format!(
        "{HISTORY_HEADER}{}{REQUEST_HEADER}{request}",
        history_lines.join("\n")
    )
}

/// Bytes an entry adds to an assembled prompt: its line plus one newline.
fn entry_bytes(actor: &str, content: &str) -> u64 {
    history_line(actor, content).len() as u64 + 1
}

/// Bytes [`assemble`] adds around a non-empty history. Every entry is counted
/// with a newline, but the last line has none, hence the one byte less.
const FRAMING_BYTES: u64 = (HISTORY_HEADER.len() + REQUEST_HEADER.len() - 1) as u64;

/// Where a limit came from. Disclosed with every selection and every stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitSource {
    /// The human set it. It wins over anything derived.
    Explicit,
    /// Derived from the graph's own whole-turn token budget.
    GraphBudget { max_tokens: u64, model_calls: u64 },
}

/// The most input, in UTF-8 bytes, a continued turn may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextLimit {
    pub max_bytes: u64,
    pub source: LimitSource,
}

impl ContextLimit {
    /// A limit the human chose for the model they bound.
    pub fn explicit(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            source: LimitSource::Explicit,
        }
    }

    /// The limit a graph implies when the human set none.
    ///
    /// Every `model` and `verify` node dispatches the history once on a clean
    /// pass, and the adapter reserves one token per input byte each time. So
    /// under that same accounting the history can take at most
    /// `max_tokens / model-calling nodes`. This is a planning share, not a
    /// guarantee: system text, outputs and revise loops also spend the
    /// budget, and running out is still a loud `BudgetExhausted`.
    ///
    /// `None` when the graph declares no token budget or calls no model:
    /// graffy then knows of no limit and does not invent one.
    pub fn from_graph(spec: &GraphSpec) -> Option<Self> {
        let max_tokens = spec.policy.budget.max_tokens?;
        let model_calls = spec
            .nodes
            .iter()
            .filter(|node| matches!(node.kind.as_str(), "model" | "verify"))
            .count() as u64;
        (model_calls > 0).then(|| Self {
            max_bytes: max_tokens / model_calls,
            source: LimitSource::GraphBudget {
                max_tokens,
                model_calls,
            },
        })
    }

    /// Plain-language origin of the limit, for disclosures.
    pub fn describe(&self) -> String {
        match &self.source {
            LimitSource::Explicit => "set by GRAFFY_CONTEXT_MAX_BYTES".to_owned(),
            LimitSource::GraphBudget {
                max_tokens,
                model_calls,
            } => format!(
                "the graph's {max_tokens}-token budget shared by its {model_calls} model-calling node(s), counting one token per byte"
            ),
        }
    }
}

/// What a bounded selection left out. Counts are exact, never estimates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Omission {
    pub omitted_turns: usize,
    pub omitted_bytes: u64,
    /// Turns the session's visible history held before selection.
    pub total_turns: usize,
    /// Whether the opening human turn was kept (always, when there is one).
    pub kept_opening: bool,
    /// How many of the most recent turns were kept.
    pub kept_recent: usize,
    pub limit: ContextLimit,
}

impl Omission {
    /// The disclosure a frontend shows the human.
    pub fn notice(&self) -> String {
        let kept = self.kept_recent + usize::from(self.kept_opening);
        let which = match (self.kept_opening, self.kept_recent) {
            (true, 0) => "only the opening human turn".to_owned(),
            (true, n) => format!("the opening human turn and the {n} most recent"),
            (false, n) => format!("the {n} most recent"),
        };
        format!(
            "context policy: the model was sent {kept} of {} earlier turns ({which}); {} turn(s) ({} bytes) were left out to fit {} bytes ({}). The model was told turns are missing. Restate anything from them that still applies, or start a new chat.",
            self.total_turns,
            self.omitted_turns,
            self.omitted_bytes,
            self.limit.max_bytes,
            self.limit.describe()
        )
    }
}

/// The history a turn will actually carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Entries to hydrate, in order. When turns were omitted, a
    /// [`POLICY_ACTOR`] line stands exactly where they were.
    pub history: Vec<(String, String)>,
    /// `None` when the whole history fit and nothing was left out.
    pub omission: Option<Omission>,
    /// UTF-8 bytes of the request assembled with the selected history.
    pub measured_bytes: u64,
}

/// The turn cannot be sent within the limit without dropping the request or
/// the opening human turn — so it is not sent at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "context policy: turn not sent. {what} come to {needed_bytes} bytes, over the {} bytes allowed ({}). No model was called and nothing was dropped silently. Shorten the request, start a new chat, or raise the limit (GRAFFY_CONTEXT_MAX_BYTES, or the graph's token budget).",
    .limit.max_bytes,
    .limit.describe()
)]
pub struct ContextOverflow {
    pub what: String,
    pub needed_bytes: u64,
    pub limit: ContextLimit,
}

/// The line that stands in for omitted turns. The model reads it.
fn marker(omitted_turns: usize, omitted_bytes: u64, max_bytes: u64) -> String {
    format!(
        "[context policy] {omitted_turns} earlier turn(s) ({omitted_bytes} bytes) were left out at this point to fit a {max_bytes}-byte input limit. You cannot see them. Do not guess what they said; if the answer depends on them, say so and ask the human to restate."
    )
}

/// Choose the history a continued turn carries.
///
/// A history that fits is returned whole. Otherwise the opening human turn is
/// kept, then the most recent turns that fit, newest first, stopping at the
/// first that does not fit so the recent window has no hidden gaps. A session
/// with no history is returned untouched: an oversized single request is the
/// token budget's business, which already fails loudly.
pub fn select(
    prompt: &str,
    history: Vec<(String, String)>,
    limit: &ContextLimit,
) -> Result<Selection, ContextOverflow> {
    let prompt_bytes = prompt.len() as u64;
    if history.is_empty() {
        return Ok(Selection {
            history,
            omission: None,
            measured_bytes: prompt_bytes,
        });
    }
    let costs: Vec<u64> = history.iter().map(|(a, c)| entry_bytes(a, c)).collect();
    let history_bytes: u64 = costs.iter().sum();
    let fixed_bytes = prompt_bytes + FRAMING_BYTES;
    if fixed_bytes + history_bytes <= limit.max_bytes {
        return Ok(Selection {
            history,
            omission: None,
            measured_bytes: fixed_bytes + history_bytes,
        });
    }

    let opening = history.iter().position(|(actor, _)| actor == HUMAN_ACTOR);
    let opening_bytes = opening.map_or(0, |ix| costs[ix]);
    // Reserve the disclosure at its largest possible size: the real line names
    // counts no greater than these, so it can only be shorter.
    let marker_reserve = entry_bytes(
        POLICY_ACTOR,
        &marker(history.len(), history_bytes, limit.max_bytes),
    );
    let required = fixed_bytes + opening_bytes + marker_reserve;
    if required > limit.max_bytes {
        return Err(ContextOverflow {
            what: match opening {
                Some(_) => format!(
                    "The request ({prompt_bytes} bytes), the opening human turn ({opening_bytes} bytes) and the notice of omitted turns"
                ),
                None => {
                    format!("The request ({prompt_bytes} bytes) and the notice of omitted turns")
                }
            },
            needed_bytes: required,
            limit: limit.clone(),
        });
    }

    let mut room = limit.max_bytes - required;
    let first_candidate = opening.map_or(0, |ix| ix + 1);
    let mut tail_start = history.len();
    while tail_start > first_candidate && costs[tail_start - 1] <= room {
        tail_start -= 1;
        room -= costs[tail_start];
    }

    let kept_recent = history.len() - tail_start;
    let kept_bytes = opening_bytes + costs[tail_start..].iter().sum::<u64>();
    let omitted_turns = history.len() - kept_recent - usize::from(opening.is_some());
    let omitted_bytes = history_bytes - kept_bytes;
    let notice = (
        POLICY_ACTOR.to_owned(),
        marker(omitted_turns, omitted_bytes, limit.max_bytes),
    );
    let measured_bytes = fixed_bytes + kept_bytes + entry_bytes(&notice.0, &notice.1);

    let mut selected = Vec::with_capacity(kept_recent + 2);
    let mut entries = history.into_iter().enumerate();
    if let Some(ix) = opening {
        selected.extend(entries.by_ref().nth(ix).map(|(_, entry)| entry));
    }
    selected.push(notice);
    selected.extend(
        entries
            .filter(|(ix, _)| *ix >= tail_start)
            .map(|(_, entry)| entry),
    );
    Ok(Selection {
        history: selected,
        omission: Some(Omission {
            omitted_turns,
            omitted_bytes,
            total_turns: costs.len(),
            kept_opening: opening.is_some(),
            kept_recent,
            limit: limit.clone(),
        }),
        measured_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONSTRAINT: &str = "Standing rule: every answer must be in metric units.";

    /// A session that opens with a constraint and then runs for `exchanges`
    /// more human/ai exchanges of filler.
    fn long_session(exchanges: usize) -> Vec<(String, String)> {
        let mut history = vec![
            ("human".to_owned(), CONSTRAINT.to_owned()),
            ("ai".to_owned(), "Understood: metric only.".to_owned()),
        ];
        for n in 0..exchanges {
            history.push((
                "human".to_owned(),
                format!("question {n} {}", "q".repeat(80)),
            ));
            history.push(("ai".to_owned(), format!("answer {n} {}", "a".repeat(160))));
        }
        history
    }

    /// Size of the prompt a model node would really build from this history.
    fn rendered_bytes(prompt: &str, history: &[(String, String)]) -> u64 {
        let lines: Vec<String> = history.iter().map(|(a, c)| history_line(a, c)).collect();
        assemble(prompt, &lines).len() as u64
    }

    fn history_bytes(history: &[(String, String)]) -> u64 {
        history.iter().map(|(a, c)| entry_bytes(a, c)).sum()
    }

    #[test]
    fn a_history_that_fits_is_sent_whole_with_nothing_to_disclose() {
        let history = long_session(3);
        let limit = ContextLimit::explicit(100_000);
        let selection = select("next", history.clone(), &limit).unwrap();
        assert_eq!(selection.history, history);
        assert_eq!(selection.omission, None);
        assert_eq!(selection.measured_bytes, rendered_bytes("next", &history));
    }

    #[test]
    fn a_long_session_selects_a_bounded_history_under_the_limit() {
        let history = long_session(200);
        let limit = ContextLimit::explicit(4_000);
        assert!(rendered_bytes("next question", &history) > limit.max_bytes);

        let selection = select("next question", history.clone(), &limit).unwrap();

        assert!(selection.measured_bytes <= limit.max_bytes);
        assert_eq!(
            selection.measured_bytes,
            rendered_bytes("next question", &selection.history),
            "the reported measure is the selection as a model node renders it"
        );
        let omission = selection.omission.expect("an omission must be disclosed");
        assert!(
            omission.kept_recent > 0,
            "recent turns are kept when they fit"
        );
        assert_eq!(
            omission.omitted_turns + omission.kept_recent + 1,
            history.len(),
            "every turn is either kept or counted as omitted"
        );
        assert_eq!(
            omission.omitted_bytes,
            history_bytes(&history) - history_bytes(&selection.history)
                + entry_bytes(POLICY_ACTOR, &selection.history[1].1),
            "omitted bytes are exact"
        );
        // The most recent turns survive, in their original order.
        let tail = &history[history.len() - omission.kept_recent..];
        assert_eq!(&selection.history[2..], tail);
    }

    #[test]
    fn the_earliest_constraint_survives_trimming_and_the_gap_is_marked() {
        let history = long_session(200);
        let selection = select("next", history, &ContextLimit::explicit(4_000)).unwrap();

        assert_eq!(
            selection.history[0],
            ("human".to_owned(), CONSTRAINT.to_owned()),
            "the opening human turn is never trimmed"
        );
        let (actor, text) = &selection.history[1];
        assert_eq!(actor, POLICY_ACTOR);
        let omission = selection.omission.unwrap();
        assert!(
            text.contains(&format!("{} earlier turn(s)", omission.omitted_turns)),
            "the model is told how much is missing, where it is missing: {text}"
        );
        assert!(omission.notice().contains("the opening human turn"));
        assert!(omission.notice().contains("GRAFFY_CONTEXT_MAX_BYTES"));
    }

    #[test]
    fn a_turn_that_cannot_keep_the_opening_constraint_stops_instead() {
        let history = long_session(200);
        let limit = ContextLimit::explicit(400);
        let prompt = "p".repeat(380);
        let error = select(&prompt, history, &limit).unwrap_err();
        assert!(error.needed_bytes > limit.max_bytes);
        let message = error.to_string();
        assert!(message.contains("turn not sent"), "{message}");
        assert!(message.contains("opening human turn"), "{message}");
        assert!(message.contains("No model was called"), "{message}");
    }

    #[test]
    fn when_no_recent_turn_fits_the_opening_turn_alone_is_kept_and_said_so() {
        let mut history = long_session(0);
        history.push(("human".to_owned(), "h".repeat(5_000)));
        history.push(("ai".to_owned(), "a".repeat(5_000)));
        let selection = select("next", history, &ContextLimit::explicit(1_000)).unwrap();
        let omission = selection.omission.unwrap();
        assert_eq!((omission.kept_recent, omission.omitted_turns), (0, 3));
        assert_eq!(selection.history.len(), 2);
        assert!(omission.notice().contains("only the opening human turn"));
        assert!(selection.measured_bytes <= 1_000);
    }

    #[test]
    fn a_new_conversation_is_never_measured_against_the_history_limit() {
        let prompt = "p".repeat(10_000);
        let selection = select(&prompt, Vec::new(), &ContextLimit::explicit(10)).unwrap();
        assert!(selection.history.is_empty());
        assert_eq!(selection.omission, None);
    }

    fn spec_with(budget: &str, kinds: &[&str]) -> GraphSpec {
        let nodes: String = kinds
            .iter()
            .enumerate()
            .map(|(n, kind)| format!("[[node]]\nid = \"n{n}\"\nkind = \"{kind}\"\n"))
            .collect();
        GraphSpec::from_toml_str(&format!(
            "[graph]\nid = \"t.ctx\"\nname = \"ctx\"\nversion = \"0.0.1\"\n{budget}\n{nodes}"
        ))
        .unwrap()
    }

    #[test]
    fn the_graph_limit_is_its_token_budget_shared_by_its_model_calling_nodes() {
        let spec = spec_with(
            "[policy.budget]\nmax_tokens = 9000",
            &[
                "intake",
                "model",
                "tool.invoke",
                "model",
                "verify",
                "respond",
            ],
        );
        let limit = ContextLimit::from_graph(&spec).unwrap();
        assert_eq!(limit.max_bytes, 3_000);
        assert_eq!(
            limit.source,
            LimitSource::GraphBudget {
                max_tokens: 9_000,
                model_calls: 3
            }
        );
        assert!(limit.describe().contains("9000-token budget"));
    }

    #[test]
    fn no_limit_is_invented_without_a_budget_or_without_model_calls() {
        assert_eq!(
            ContextLimit::from_graph(&spec_with("", &["intake", "model", "respond"])),
            None
        );
        assert_eq!(
            ContextLimit::from_graph(&spec_with(
                "[policy.budget]\nmax_tokens = 9000",
                &["intake", "respond"]
            )),
            None
        );
    }
}
