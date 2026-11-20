//! The shared turn runner (ADR-0010, step 1).
//!
//! One external request is one **turn**: one attempt group, with each attempt
//! its own run and journal. Plain CLI and Ratatui both drive this so identity,
//! whole-turn budgets and retry rules cannot drift between frontends. It never
//! prints and never delivers — rendering and output receipts belong to the
//! frontend that actually shows the answer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use graffy_core::context::{self, ContextLimit};
use graffy_core::control::{Cancellation, RemainingBudget};
use graffy_core::exec::{
    ApprovalHandler, Executor, ModelInvoker, RepairFeedback, RunInput, RunOutcome, ToolInvoker,
};
use graffy_core::journal::{EventTap, JournalReader, wire};
use graffy_core::spec::GraphSpec;
use graffy_proto::mcw::v1::FailureMode;

/// Everything one turn needs. Owned, so a frontend can move it into a task.
pub struct Turn {
    pub spec: GraphSpec,
    pub spec_text: String,
    pub prompt: String,
    /// Explicit human declaration, never inferred from wording.
    pub correction: bool,
    /// Continue this session; `None` starts a new conversation.
    pub session: Option<String>,
    /// What the turn carries of the session so far, context policy applied.
    pub history: TurnHistory,
    pub max_attempts: u32,
    /// Caller-chosen journal path for the first attempt only.
    pub journal: Option<PathBuf>,
    pub runs_dir: PathBuf,
    /// When the turn began — setup time (tool connection) spends budget too.
    pub started: Instant,
    pub invoker: Arc<dyn ModelInvoker>,
    pub tools: Option<Arc<dyn ToolInvoker>>,
    pub approvals: Arc<dyn ApprovalHandler>,
    pub cancellation: Cancellation,
    pub event_tap: Option<EventTap>,
    /// The routing run that selected this graph for the prompt (ADR-0012).
    pub routed_by: Option<String>,
}

/// The visible boundary history a turn carries, after the context policy
/// (ADR-0014). There is no way to hand a turn raw hydrated history: a frontend
/// has to come through [`TurnHistory::select`], so no frontend can skip the
/// policy and every attempt of a turn carries the same selection.
pub struct TurnHistory {
    entries: Vec<(String, String)>,
    notice: Option<String>,
}

impl TurnHistory {
    /// A new conversation: there is no history to carry.
    pub fn none() -> Self {
        Self {
            entries: Vec::new(),
            notice: None,
        }
    }

    /// Apply the context policy to a continued session's hydrated history.
    ///
    /// `max_bytes` is the human's own limit (`GRAFFY_CONTEXT_MAX_BYTES`) and
    /// wins when set; otherwise the limit is the one the graph's token budget
    /// implies. A graph with no token budget implies none, and the history is
    /// then carried whole. An error means the turn cannot be sent within the
    /// limit: nothing has run and no model was called.
    pub fn select(
        spec: &GraphSpec,
        prompt: &str,
        hydrated: Vec<(String, String)>,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<Self> {
        let limit = max_bytes
            .map(ContextLimit::explicit)
            .or_else(|| ContextLimit::from_graph(spec));
        let Some(limit) = limit else {
            return Ok(Self {
                entries: hydrated,
                notice: None,
            });
        };
        let selection = context::select(prompt, hydrated, &limit)?;
        Ok(Self {
            entries: selection.history,
            notice: selection.omission.map(|omission| omission.notice()),
        })
    }

    /// Set when earlier turns were left out. The frontend must show it to the
    /// human; the model and the journal are told by the selection itself.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
}

/// What a finished attempt means for the turn.
pub enum Verdict {
    /// The graph released an answer; the turn is over.
    Released,
    /// The attempt failed with a recorded repair signal; another may run.
    Retry,
}

pub struct TurnRunner {
    turn: Turn,
    continuing: bool,
    session_id: Option<String>,
    attempt_group: Option<String>,
    prior_run_id: Option<String>,
    feedback: Vec<RepairFeedback>,
    tokens_spent: u64,
    usd_spent: f64,
    attempt: u32,
}

impl TurnRunner {
    pub fn new(turn: Turn) -> Self {
        Self {
            continuing: turn.session.is_some(),
            session_id: turn.session.clone(),
            turn,
            attempt_group: None,
            prior_run_id: None,
            feedback: Vec::new(),
            tokens_spent: 0,
            usd_spent: 0.0,
            attempt: 0,
        }
    }

    /// 1-based number of the most recent attempt (0 before the first).
    pub fn attempt_number(&self) -> u32 {
        self.attempt
    }

    pub fn max_attempts(&self) -> u32 {
        self.turn.max_attempts
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Run the next attempt. `on_start(attempt, max)` fires only once the
    /// whole-turn budget check has passed, so a frontend never announces an
    /// attempt that did not start.
    pub async fn attempt(&mut self, on_start: impl FnOnce(u32, u32)) -> anyhow::Result<RunOutcome> {
        let budget = &self.turn.spec.policy.budget;
        let remaining = RemainingBudget {
            tokens: budget
                .max_tokens
                .map(|cap| cap.saturating_sub(self.tokens_spent)),
            usd: budget.max_usd.map(|cap| (cap - self.usd_spent).max(0.0)),
            duration: budget.max_seconds.map(|seconds| {
                Duration::from_secs(seconds).saturating_sub(self.turn.started.elapsed())
            }),
        };
        if remaining.tokens == Some(0)
            || remaining.usd == Some(0.0)
            || remaining.duration.is_some_and(|d| d.is_zero())
        {
            anyhow::bail!("whole-turn budget exhausted; no further attempt started");
        }
        let attempt = self.attempt + 1;
        let path = match (&self.turn.journal, attempt) {
            (Some(path), 1) => path.clone(),
            _ => self
                .turn
                .runs_dir
                .join(format!("{}.journal", graffy_core::id::RunId::generate())),
        };
        on_start(attempt, self.turn.max_attempts);
        let executor = Executor {
            tool_invoker: self.turn.tools.clone(),
            cancellation: self.turn.cancellation.clone(),
            remaining_budget: remaining,
            event_tap: self.turn.event_tap.clone(),
            ..Default::default()
        };
        let run_kind = if attempt > 1 {
            wire::RunKind::AutomaticRetry
        } else if self.continuing {
            wire::RunKind::ExternalFollowup
        } else {
            wire::RunKind::InitialAttempt
        };
        let outcome = executor
            .run(
                &self.turn.spec,
                &self.turn.spec_text,
                RunInput {
                    prompt: self.turn.prompt.clone(),
                    is_correction: self.turn.correction,
                    session_id: self.session_id.clone(),
                    feedback: std::mem::take(&mut self.feedback),
                    attempt_group_id: self.attempt_group.clone(),
                    attempt_index: attempt,
                    retry_of_run_id: self.prior_run_id.clone(),
                    routed_by_run_id: self.turn.routed_by.clone(),
                    run_kind: run_kind as i32,
                    prior_boundary: self.turn.history.entries.clone(),
                },
                &path,
                self.turn.invoker.as_ref(),
                self.turn.approvals.as_ref(),
            )
            .await?;
        self.attempt = attempt;
        self.session_id = Some(outcome.session_id.clone());
        self.attempt_group = Some(outcome.attempt_group_id.clone());
        self.prior_run_id = Some(outcome.run_id.clone());
        self.tokens_spent = self
            .tokens_spent
            .saturating_add(outcome.input_tokens)
            .saturating_add(outcome.output_tokens);
        self.usd_spent += outcome.total_usd;
        Ok(outcome)
    }

    /// Decide what the attempt just returned by [`Self::attempt`] means.
    /// Errors are the honest ends of a turn that released nothing.
    pub fn verdict(&mut self, outcome: &RunOutcome) -> anyhow::Result<Verdict> {
        let status = outcome.status;
        if status == wire::RunStatus::Succeeded {
            return Ok(Verdict::Released);
        }
        if matches!(
            status,
            wire::RunStatus::Cancelled | wire::RunStatus::BudgetExhausted
        ) || self.attempt >= self.turn.max_attempts
        {
            anyhow::bail!("run ended {status:?}; no answer released");
        }
        match harvest_feedback(&outcome.journal_path, self.attempt) {
            Some(item) => {
                self.feedback = vec![item];
                Ok(Verdict::Retry)
            }
            None => {
                anyhow::bail!("run ended {status:?} with no repair signal; no further attempts")
            }
        }
    }
}

/// Pull the most recent failure signal out of a journal — judge-named modes
/// preferred over Unspecified — as the next attempt's repair context.
fn harvest_feedback(journal_path: &std::path::Path, attempt: u32) -> Option<RepairFeedback> {
    let events = JournalReader::read_all(journal_path).ok()?;
    let mut last_any = None;
    let mut last_named = None;
    for ev in &events {
        if let Some(wire::run_event::Event::FailureRaised(f)) = &ev.event {
            last_any = Some(f);
            if f.mode != FailureMode::Unspecified as i32 {
                last_named = Some(f);
            }
        }
    }
    let f = last_named.or(last_any)?;
    Some(RepairFeedback {
        failure_id: f.id.clone(),
        mode: FailureMode::try_from(f.mode).unwrap_or(FailureMode::Unspecified),
        critique: f.early_signal.clone(),
        source_attempt: attempt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use graffy_core::error::ModelError;
    use graffy_core::exec::{AutoApprove, ModelRequest, ModelResponse, OfflineEcho};
    use std::sync::Mutex;

    const CONSTRAINT: &str = "Standing rule: every answer must be in metric units.";

    /// The offline echo, remembering every draft prompt it was handed.
    #[derive(Default)]
    struct RecordingEcho(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl ModelInvoker for RecordingEcho {
        async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
            if request.purpose == "draft" {
                self.0.lock().unwrap().push(request.prompt.clone());
            }
            OfflineEcho.complete(request).await
        }
    }

    /// The built-in conversation graph with its token budget set to `max_tokens`.
    fn conversation(max_tokens: u64) -> (GraphSpec, String) {
        let mut spec = GraphSpec::from_toml_str(graffy_graphs::DEFAULT_CONVERSATION_TOML).unwrap();
        spec.policy.budget.max_tokens = Some(max_tokens);
        let text = spec.to_toml_string().unwrap();
        (spec, text)
    }

    /// A session that opened with a constraint and ran 100 more exchanges.
    fn long_session() -> Vec<(String, String)> {
        let mut history = vec![
            ("human".to_owned(), CONSTRAINT.to_owned()),
            ("ai".to_owned(), "Understood: metric only.".to_owned()),
        ];
        for n in 0..100 {
            history.push(("human".to_owned(), format!("question-{n} about distances")));
            history.push(("ai".to_owned(), format!("answer-{n}: {}", "km ".repeat(30))));
        }
        history
    }

    fn turn(
        spec: &GraphSpec,
        spec_text: &str,
        prompt: &str,
        history: TurnHistory,
        invoker: Arc<dyn ModelInvoker>,
        runs_dir: &std::path::Path,
    ) -> Turn {
        Turn {
            spec: spec.clone(),
            spec_text: spec_text.to_owned(),
            prompt: prompt.to_owned(),
            correction: false,
            session: Some("ses_context_policy".to_owned()),
            history,
            max_attempts: 1,
            journal: None,
            runs_dir: runs_dir.to_owned(),
            started: Instant::now(),
            invoker,
            tools: None,
            approvals: Arc::new(AutoApprove),
            cancellation: Cancellation::default(),
            event_tap: None,
            routed_by: None,
        }
    }

    #[tokio::test]
    async fn a_long_session_turn_is_bounded_disclosed_and_keeps_the_opening_constraint() {
        // 8000 tokens shared by the graph's two model-calling nodes (draft,
        // verify) is a 4000-byte input limit. No human override is set.
        let (spec, spec_text) = conversation(8_000);
        let limit = ContextLimit::from_graph(&spec).unwrap();
        assert_eq!(limit.max_bytes, 4_000);
        let hydrated = long_session();
        let whole: usize = hydrated.iter().map(|(a, c)| a.len() + c.len()).sum();
        assert!(whole as u64 > limit.max_bytes, "the session must not fit");

        let prompt = "how far is the moon?";
        let history = TurnHistory::select(&spec, prompt, hydrated, None).unwrap();
        let notice = history.notice().expect("the human is told").to_owned();
        assert!(notice.contains("the opening human turn"), "{notice}");
        assert!(notice.contains("8000-token budget"), "{notice}");

        let runs_dir = std::env::temp_dir().join(format!(
            "graffy-turn-context-{}",
            graffy_core::id::RunId::generate()
        ));
        std::fs::create_dir_all(&runs_dir).unwrap();
        let invoker = Arc::new(RecordingEcho::default());
        let mut runner = TurnRunner::new(turn(
            &spec,
            &spec_text,
            prompt,
            history,
            invoker.clone(),
            &runs_dir,
        ));
        let outcome = runner.attempt(|_, _| {}).await.unwrap();
        assert_eq!(outcome.status, wire::RunStatus::Succeeded);

        // What the model was actually sent stays under the limit...
        let drafts = invoker.0.lock().unwrap().clone();
        assert_eq!(drafts.len(), 1);
        let sent = &drafts[0];
        assert!(
            sent.len() as u64 <= limit.max_bytes,
            "sent {} bytes over a {}-byte limit",
            sent.len(),
            limit.max_bytes
        );
        // ...keeps the earliest constraint and the latest turns, and says
        // where turns are missing instead of closing the gap silently.
        assert!(sent.contains(CONSTRAINT));
        assert!(sent.contains("answer-99"));
        assert!(!sent.contains("question-0 "), "a middle turn was sent");
        let (before_gap, after_gap) = sent
            .split_once("graffy: [context policy]")
            .expect("the model is told turns are missing");
        assert!(before_gap.contains(CONSTRAINT));
        assert!(after_gap.contains("answer-99") && after_gap.ends_with(prompt));

        // The journal records the same selection, disclosure included.
        let history_actors: Vec<String> = JournalReader::read_all(&outcome.journal_path)
            .unwrap()
            .iter()
            .filter_map(|event| match &event.event {
                Some(wire::run_event::Event::IuRecorded(iu))
                    if iu.attributes.get("role").map(String::as_str)
                        == Some("boundary-history") =>
                {
                    iu.attributes.get("actor").cloned()
                }
                _ => None,
            })
            .collect();
        assert_eq!(history_actors[..2], ["human", context::POLICY_ACTOR]);
        assert!(
            history_actors.len() < 202,
            "the whole session was journaled"
        );
        std::fs::remove_dir_all(&runs_dir).ok();
    }

    #[test]
    fn a_turn_that_cannot_keep_the_opening_constraint_is_refused_before_any_model_call() {
        let (spec, _) = conversation(8_000);
        let error = TurnHistory::select(&spec, "next", long_session(), Some(200))
            .err()
            .expect("200 bytes cannot hold the request, the opening turn and the notice");
        let message = error.to_string();
        assert!(message.contains("turn not sent"), "{message}");
        assert!(message.contains("GRAFFY_CONTEXT_MAX_BYTES"), "{message}");
    }

    #[test]
    fn a_session_that_fits_and_a_graph_without_a_budget_carry_history_whole() {
        let (mut spec, _) = conversation(200_000);
        let short = vec![
            ("human".to_owned(), CONSTRAINT.to_owned()),
            ("ai".to_owned(), "Understood.".to_owned()),
        ];
        let fits = TurnHistory::select(&spec, "next", short.clone(), None).unwrap();
        assert_eq!(fits.entries, short);
        assert!(fits.notice().is_none());

        // No budget, no override: graffy knows of no limit and invents none.
        spec.policy.budget.max_tokens = None;
        let unbounded = TurnHistory::select(&spec, "next", long_session(), None).unwrap();
        assert_eq!(unbounded.entries.len(), 202);
        assert!(unbounded.notice().is_none());
    }
}
