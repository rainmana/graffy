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
    /// Visible boundary history hydrated for a continued session.
    pub prior_boundary: Vec<(String, String)>,
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
                    run_kind: run_kind as i32,
                    prior_boundary: self.turn.prior_boundary.clone(),
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
