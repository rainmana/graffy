//! graffy — graph-native agentic harness.
//!
//! Nothing — no prompt, no skill, no chat turn — executes outside an
//! inspectable, durable, shareable agent graph. See docs/ARCHITECTURE.md.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use graffy_core::exec::ToolInvoker;
use graffy_core::exec::{
    ApprovalHandler, ApprovalOutcome, Executor, ModelInvoker, OfflineEcho, RunInput, RunOutcome,
};
use graffy_core::journal::{JournalReader, JournalWriter, event_kind, summarize, wire};
use graffy_core::spec::GraphSpec;
use graffy_mcp::{LiveServer, RegistryToolInvoker, ServerBinding};
use graffy_memory::{McpServerRecord, RunRecord, Store};
use graffy_providers::RigInvoker;

#[derive(Parser)]
#[command(
    name = "graffy",
    version,
    about = "Graph-native agentic harness — no prompt or skill ever executes outside a graph."
)]
struct Cli {
    /// Increase log verbosity (-v, -vv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the TUI journal browser (pick past runs to inspect).
    Tui,
    /// Execute a TOML graph spec against a prompt.
    /// Create the graffy home (default ~/.graffy) and seed the store.
    Init,
    /// Fold run journals into research metrics (the MCW dataset, aggregated).
    Metrics {
        /// Directory of .journal files (default: <home>/runs).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Emit machine-readable JSON instead of the text summary.
        #[arg(long)]
        json: bool,
    },
    /// Score a session's coordination health against the HRDM anchors
    /// (docs/mcw/hrdm-in-graffy.md). Judgments are yours; samples are
    /// journaled observations, never computed scores.
    Rate {
        /// Session id to rate (its runs are found in the runs directory).
        session: String,
        /// Directory of .journal files (default: <home>/runs).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Rater identity stamped on every sample (default: $USER).
        #[arg(long)]
        rater: Option<String>,
        /// Condition-blinded rating: suppress graph names and any
        /// arm-revealing header fields (the projection itself carries no
        /// provider/model/tier/judge-label fields by construction).
        #[arg(long)]
        blinded: bool,
    },
    Run {
        /// Path to a TOML graph spec (see graphs/), or a registered graph id.
        spec: String,
        /// User prompt handed to the graph's intake node.
        #[arg(long)]
        prompt: String,
        /// Watch the run live in the TUI (node states, journal feed,
        /// step inspector) instead of plain logs.
        #[arg(long)]
        tui: bool,
        /// Run against the deterministic offline echo invoker (no model,
        /// no network — for demos and engine testing; clearly labeled).
        #[arg(long)]
        offline: bool,
        /// Journal output path (default: <home>/runs/<ulid>.journal).
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Retry failed runs with the judge's critique fed back as repair
        /// context: a number of extra attempts, or 'auto' (bounded — 3).
        #[arg(long)]
        retry: Option<String>,
        /// Continue an existing coordination session: boundary exchanges
        /// accumulate across prompts, and five completed exchanges make a
        /// canonical H/D/M window (docs/mcw/hrdm-in-graffy.md §1.3).
        #[arg(long)]
        session: Option<String>,
    },
    /// Replay a run journal: fold the event stream into a summary.
    Replay {
        /// Path to a .journal file produced by `graffy run`.
        journal: PathBuf,
        /// Open the TUI step inspector instead of printing a summary.
        #[arg(long)]
        tui: bool,
        /// Also list every event frame (plain mode).
        #[arg(long)]
        events: bool,
    },
    /// List recent runs from the store.
    Runs {
        /// How many runs to show.
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Inspect, export, and import durable graph objects.
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
    /// Convert a skill (SKILL.md) or raw prompt file into a durable graph —
    /// the founding invariant: nothing ever executes as raw text.
    Graphify {
        /// Path to a SKILL.md / markdown skill or a plain prompt file.
        path: PathBuf,
        /// Override the graph name (default: frontmatter, first heading, or file name).
        #[arg(long)]
        name: Option<String>,
        /// Treat the input as a raw prompt even if it looks like markdown.
        #[arg(long)]
        prompt: bool,
        /// Involvement mode: auto (v1) | guided | collaborative (TUI flows, coming).
        #[arg(long, default_value = "auto")]
        mode: String,
    },
    /// Manage MCP servers: add (discovery + facade generation) and list.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Print environment diagnostics.
    Doctor,
}

#[derive(Subcommand)]
enum McpCommand {
    /// Connect a stdio MCP server, discover its tools, seed roles from
    /// annotations, and register skill-fronted facade graphs.
    Add {
        /// Logical server name (graphs reference this, never the transport).
        name: String,
        /// The stdio command line, quoted (e.g. "npx -y @modelcontextprotocol/server-everything").
        #[arg(long)]
        stdio: String,
        /// Default role for unannotated tools: evidence | effector
        /// (default effector — the conservative choice).
        #[arg(long)]
        role: Option<String>,
        /// Evidence level granted to this server's results (L0|L1|L2).
        #[arg(long, default_value = "L1")]
        evidence_level: String,
        /// Usage knowledge to front this server's facades (skips the interview).
        #[arg(long)]
        knowledge: Option<String>,
        /// Skip the usage interview even when the server ships no knowledge.
        #[arg(long)]
        skip_interview: bool,
        /// Register the server without generating facade graphs.
        #[arg(long)]
        no_facades: bool,
    },
    /// List registered MCP servers.
    List,
}

#[derive(Subcommand)]
enum GraphCommand {
    /// List installed graphs (built-ins are seeded automatically).
    List,
    /// Export a graph's TOML spec for sharing.
    Export {
        /// Registered graph id (see `graffy graph list`).
        id: String,
        /// Output path (default: <id>.toml in the current directory).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Import a shared graph spec (validated: parse + compile before it lands).
    Import { path: PathBuf },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let default_filter = match cli.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => graffy_tui::run_home(&runs_dir()),
        Command::Init => cmd_init().await,
        Command::Metrics { dir, json } => cmd_metrics(dir, json),
        Command::Rate {
            session,
            dir,
            rater,
            blinded,
        } => cmd_rate(session, dir, rater, blinded),
        Command::Run {
            spec,
            prompt,
            tui,
            offline,
            journal,
            retry,
            session,
        } => run_graph(spec, prompt, tui, offline, journal, retry, session).await,
        Command::Replay {
            journal,
            tui,
            events,
        } => {
            if tui {
                graffy_tui::run_replay(&journal)
            } else {
                replay(journal, events)
            }
        }
        Command::Runs { limit } => list_runs(limit).await,
        Command::Graph { command } => match command {
            GraphCommand::List => graph_list().await,
            GraphCommand::Export { id, out } => graph_export(id, out).await,
            GraphCommand::Import { path } => graph_import(path).await,
        },
        Command::Graphify {
            path,
            name,
            prompt,
            mode,
        } => graphify_cmd(path, name, prompt, mode).await,
        Command::Mcp { command } => match command {
            McpCommand::Add {
                name,
                stdio,
                role,
                evidence_level,
                knowledge,
                skip_interview,
                no_facades,
            } => {
                mcp_add(
                    name,
                    stdio,
                    role,
                    evidence_level,
                    knowledge,
                    skip_interview,
                    no_facades,
                )
                .await
            }
            McpCommand::List => mcp_list().await,
        },
        Command::Doctor => doctor().await,
    }
}

// ---------------------------------------------------------------------------
// Store plumbing
// ---------------------------------------------------------------------------

/// Resolve the graffy home. Precedence: GRAFFY_DATA_DIR (legacy name, kept
/// working), GRAFFY_HOME, a pre-alpha.5 platform data dir IF a store already
/// lives there (nobody's history disappears on upgrade), else `~/.graffy`.
fn graffy_home() -> PathBuf {
    for var in ["GRAFFY_DATA_DIR", "GRAFFY_HOME"] {
        if let Ok(dir) = std::env::var(var)
            && !dir.is_empty()
        {
            return PathBuf::from(dir);
        }
    }
    if let Some(dirs) = directories::ProjectDirs::from("dev", "graffy", "graffy") {
        let legacy = dirs.data_dir().to_path_buf();
        if legacy.join("graffy.db").exists() {
            return legacy;
        }
    }
    default_home()
}

/// `~/.graffy`, or `./.graffy` when no home directory can be determined.
fn default_home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".graffy"))
        .unwrap_or_else(|| PathBuf::from(".graffy"))
}

fn db_path() -> PathBuf {
    graffy_home().join("graffy.db")
}

/// Where run journals land by default (`<home>/runs`).
fn runs_dir() -> PathBuf {
    graffy_home().join("runs")
}

/// `graffy init` — create the home, seed built-ins, say where everything is.
async fn cmd_init() -> anyhow::Result<()> {
    std::fs::create_dir_all(runs_dir())?;
    let _store = open_store().await?;
    println!("graffy home : {}", graffy_home().display());
    println!("store       : {}", db_path().display());
    println!("run journals: {}", runs_dir().display());
    println!("override    : set GRAFFY_HOME (or GRAFFY_DATA_DIR) to relocate");
    if let Some(dirs) = directories::ProjectDirs::from("dev", "graffy", "graffy") {
        let legacy = dirs.data_dir().join("graffy.db");
        if legacy.exists() && legacy != db_path() {
            println!(
                "note: a legacy store exists at {} — move it into the home above to keep that history",
                legacy.display()
            );
        }
    }
    Ok(())
}

/// `graffy metrics` — fold run journals into research metrics (C5 v1).
fn cmd_metrics(dir: Option<PathBuf>, json: bool) -> anyhow::Result<()> {
    use graffy_core::journal::JournalReader;
    use graffy_core::metrics::{AggregateMetrics, MetricsReport, RunMetrics};

    let dir = dir.unwrap_or_else(|| {
        let preferred = runs_dir();
        let legacy = PathBuf::from("graffy-runs");
        if !preferred.exists() && legacy.exists() {
            eprintln!(
                "note: reading ./graffy-runs (legacy location); new runs land in {}",
                preferred.display()
            );
            legacy
        } else {
            preferred
        }
    });
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "journal"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    if paths.is_empty() {
        println!("no journals found in {}", dir.display());
        println!("try: graffy run graffy.builtin.conversation --prompt \"hello\" --offline");
        return Ok(());
    }
    let mut rows = Vec::new();
    for p in &paths {
        match JournalReader::read_all(p) {
            Ok(events) => rows.push(RunMetrics::fold(&events)),
            Err(err) => eprintln!("skipping {}: {err}", p.display()),
        }
    }
    let aggregate = AggregateMetrics::from_rows(&rows);
    let sessions = graffy_core::metrics::sessions_from_rows(&rows);
    let attempt_groups = graffy_core::metrics::attempt_groups_from_rows(&rows);
    if json {
        let report = MetricsReport {
            generated_by: format!("graffy {}", env!("CARGO_PKG_VERSION")),
            runs: rows,
            sessions,
            attempt_groups,
            aggregate,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!(
        "graffy metrics — {} runs from {}",
        rows.len(),
        dir.display()
    );
    println!();
    for r in &rows {
        println!(
            "  {}  {:<9} esc {} · fail {} · repair {} · caps {}  ({})",
            r.run_id,
            r.status,
            r.escalations,
            r.failure_signals,
            r.repairs,
            r.visit_cap_hits,
            r.graph_name
        );
    }
    println!();
    println!("aggregate");
    println!(
        "  runs         : {} {:?}",
        aggregate.runs, aggregate.runs_by_status
    );
    println!(
        "  tokens       : {} in / {} out   cost: ${:.4}",
        aggregate.input_tokens, aggregate.output_tokens, aggregate.cost_usd
    );
    println!(
        "  model calls  : {} · tool calls {} · IUs {}",
        aggregate.model_calls, aggregate.tool_calls, aggregate.ius_recorded
    );
    println!(
        "  failures     : {} {:?}",
        aggregate.failure_signals, aggregate.failures_by_mode
    );
    println!(
        "  repairs      : {} {:?} (cost {} tokens)",
        aggregate.repairs, aggregate.repairs_by_op, aggregate.repair_cost_tokens
    );
    println!(
        "  evidence     : {} {:?}",
        aggregate.evidence_artifacts, aggregate.evidence_by_level
    );
    println!(
        "  escalation   : {} runs escalated · success with/without escalation: {} / {}",
        aggregate.runs_with_escalation,
        rate_str(aggregate.escalation_success_rate),
        rate_str(aggregate.baseline_success_rate)
    );
    println!(
        "  convergence  : {} runs hit a visit cap · mean escalations/run {:.2}",
        aggregate.runs_with_cap_hits, aggregate.mean_escalations_per_run
    );
    println!(
        "  sessions     : {} · attempt groups {} ({} retried, {} converged after retry, mean attempts {})",
        aggregate.sessions,
        aggregate.attempt_groups,
        aggregate.groups_with_retries,
        aggregate.groups_converged_after_retry,
        aggregate
            .mean_attempts_to_converge
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "n/a".to_owned())
    );
    if aggregate.rating_observations > 0 {
        println!(
            "  ratings      : {} observation journal(s) — excluded from every execution aggregate",
            aggregate.rating_observations
        );
    }
    let _ = &attempt_groups;
    println!(
        "  repair fx    : run-passed {} of {} ({}) · target resolved {} of {} assessed ({})",
        aggregate.repairs_run_passed,
        aggregate.repairs,
        rate_str(aggregate.repair_run_passed_rate),
        aggregate.repairs_target_resolved,
        aggregate.repairs_resolution_assessed,
        rate_str(aggregate.repair_target_resolution_rate)
    );
    let _ = &sessions;
    println!(
        "  hrdm samples : {} (rubric-scored — see docs/design/phase-3-learning.md)",
        aggregate.hrdm_samples
    );
    Ok(())
}

fn rate_str(r: Option<f64>) -> String {
    r.map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or_else(|| "n/a".to_owned())
}

/// Open the store and make sure shipped built-ins are registered.
async fn open_store() -> anyhow::Result<Store> {
    let store = Store::open(&db_path()).await?;
    let seeded = store.seed_builtins(&graffy_graphs::builtin_specs()).await?;
    if seeded > 0 {
        tracing::info!(seeded, "registered/updated built-in graphs");
    }
    Ok(store)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn build_invoker(offline: bool) -> Arc<dyn ModelInvoker> {
    if offline {
        println!("mode: OFFLINE ECHO — deterministic, no real model is consulted\n");
        return Arc::new(OfflineEcho);
    }
    match RigInvoker::from_env() {
        Ok(invoker) => {
            for (tier, target) in invoker.bound_tiers() {
                println!("tier {tier:<10} -> {target}");
            }
            println!();
            Arc::new(invoker)
        }
        Err(err) => {
            eprintln!("cannot start a live run: {err}");
            eprintln!("hint: `graffy run --offline …` exercises the engine without a model.");
            std::process::exit(2);
        }
    }
}

/// Resolve a spec argument: a path to a TOML file, or a registered graph id.
async fn resolve_spec(spec_arg: &str) -> anyhow::Result<String> {
    let as_path = PathBuf::from(spec_arg);
    if as_path.exists() {
        return Ok(std::fs::read_to_string(&as_path)?);
    }
    let store = open_store().await?;
    if let Some(record) = store.get_graph(spec_arg).await? {
        println!(
            "resolved '{}' from the registry (source: {}, sha {})",
            spec_arg,
            record.source,
            &record.spec_sha256[..8.min(record.spec_sha256.len())]
        );
        return Ok(record.spec_toml);
    }
    anyhow::bail!(
        "'{spec_arg}' is neither a readable file nor a registered graph id \
         (see `graffy graph list`)"
    );
}

/// 'auto' retry cap — bounded by design, like every loop in graffy.
const DEFAULT_AUTO_RETRIES: u32 = 3;

async fn run_graph(
    spec_arg: String,
    prompt: String,
    tui: bool,
    offline: bool,
    journal: Option<PathBuf>,
    retry: Option<String>,
    session: Option<String>,
) -> anyhow::Result<()> {
    let spec_text = resolve_spec(&spec_arg).await?;
    let spec = GraphSpec::from_toml_str(&spec_text)?;
    let invoker = build_invoker(offline);
    let tool_plane = build_tool_plane(&spec).await?;

    // C2: retries are ALWAYS bounded — 'auto' means "until PASS or the
    // default attempt cap", never "forever". Per-attempt token/time budgets
    // still enforce inside each run.
    let max_attempts: u32 = match retry.as_deref() {
        None => 1,
        Some("auto") => 1 + DEFAULT_AUTO_RETRIES,
        Some(n) => {
            1 + n.parse::<u32>().map_err(|_| {
                anyhow::anyhow!("--retry takes a number of extra attempts or 'auto', got '{n}'")
            })?
        }
    };
    if tui && max_attempts > 1 {
        anyhow::bail!(
            "--retry is not supported with --tui yet — run plain, then replay any attempt in the TUI"
        );
    }
    if tui && session.is_some() {
        anyhow::bail!(
            "--session is not supported with --tui yet — boundary hydration is CLI-only for now \
             (rejecting explicitly rather than silently ignoring the flag)"
        );
    }

    if tui {
        let journal_path =
            journal.unwrap_or_else(|| runs_dir().join(format!("{}.journal", ulid_like_stamp())));
        let Some(outcome) = graffy_tui::run_live(
            spec.clone(),
            spec_text.clone(),
            prompt,
            journal_path.clone(),
            invoker,
            tool_plane.clone(),
        )
        .await?
        else {
            std::process::exit(1);
        };
        report_outcome(&spec, &spec_text, outcome).await;
        return Ok(());
    }

    let continuing_session = session.is_some();
    // P0.1: continuing a session hydrates the prior VISIBLE boundary
    // conversation into the new run's context — otherwise "prompt 2 can
    // reference response 1" would be false advertising.
    let prior_boundary: Vec<(String, String)> = match &session {
        Some(ses) => {
            let runs = load_session_runs(&runs_dir(), ses);
            let projection = graffy_core::boundary::project(&runs);
            graffy_core::boundary::hydration_from(&projection)
        }
        None => Vec::new(),
    };
    let mut session_id: Option<String> = session;
    let mut feedback: Vec<graffy_core::exec::RepairFeedback> = Vec::new();
    let mut attempt_group: Option<String> = None;
    let mut prior_run_id: Option<String> = None;
    for attempt in 1..=max_attempts {
        let journal_path = match (&journal, attempt) {
            (Some(path), 1) => path.clone(),
            // Extra attempts always mint fresh journals — one file per run,
            // linked by the shared session id (the repair episode's spine).
            _ => runs_dir().join(format!("{}.journal", ulid_like_stamp())),
        };
        println!(
            "graph '{}' v{} — executing (attempt {attempt}/{max_attempts}, every step journaled)",
            spec.graph.name, spec.graph.version
        );
        let executor = Executor {
            tool_invoker: tool_plane.clone(),
            ..Default::default()
        };
        let outcome = executor
            .run(
                &spec,
                &spec_text,
                RunInput {
                    prompt: prompt.clone(),
                    session_id: session_id.clone(),
                    feedback: std::mem::take(&mut feedback),
                    attempt_group_id: attempt_group.clone(),
                    attempt_index: attempt,
                    retry_of_run_id: if attempt > 1 {
                        prior_run_id.clone()
                    } else {
                        None
                    },
                    run_kind: if attempt > 1 {
                        graffy_core::journal::wire::RunKind::AutomaticRetry as i32
                    } else if continuing_session {
                        graffy_core::journal::wire::RunKind::ExternalFollowup as i32
                    } else {
                        graffy_core::journal::wire::RunKind::InitialAttempt as i32
                    },
                    prior_boundary: prior_boundary.clone(),
                },
                &journal_path,
                invoker.as_ref(),
                &CliApprovalHandler,
            )
            .await?;
        session_id = Some(outcome.session_id.clone());
        attempt_group = Some(outcome.attempt_group_id.clone());
        prior_run_id = Some(outcome.run_id.clone());
        let succeeded = outcome.status == graffy_proto::journal::v1::RunStatus::Succeeded;
        let harvest_path = outcome.journal_path.clone();
        report_outcome(&spec, &spec_text, outcome).await;

        if succeeded {
            if attempt > 1 {
                println!(
                    "\nconverged on attempt {attempt}/{max_attempts} — internal repair sequence closed"
                );
            }
            if let Some(ses) = &session_id {
                println!(
                    "continue this session (adds a boundary exchange): graffy run {spec_arg} --prompt \"...\" --session {ses}"
                );
            }
            return Ok(());
        }
        if attempt == max_attempts {
            if max_attempts > 1 {
                println!(
                    "\nretry budget exhausted ({max_attempts} attempts) — failing honestly; \
                     the convergence series lives in this session's journals"
                );
            }
            return Ok(());
        }
        match harvest_feedback(&harvest_path, attempt) {
            Some(item) => {
                println!(
                    "\nattempt {attempt} failed ({}) — retrying with the judge's critique as repair feedback",
                    item.mode
                        .as_str_name()
                        .trim_start_matches("FAILURE_MODE_")
                        .to_ascii_lowercase()
                );
                feedback = vec![item];
            }
            None => {
                println!(
                    "\nattempt {attempt} failed with no failure signal to feed back — \
                     stopping retries (nothing to repair from)"
                );
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Pull the most recent failure signal out of a journal — judge-named modes
/// preferred over Unspecified — as the next attempt's repair context.
fn harvest_feedback(
    journal_path: &std::path::Path,
    attempt: u32,
) -> Option<graffy_core::exec::RepairFeedback> {
    let events = graffy_core::journal::JournalReader::read_all(journal_path).ok()?;
    let mut last_any = None;
    let mut last_named = None;
    for ev in &events {
        if let Some(graffy_core::journal::wire::run_event::Event::FailureRaised(f)) = &ev.event {
            last_any = Some(f);
            if f.mode != graffy_proto::mcw::v1::FailureMode::Unspecified as i32 {
                last_named = Some(f);
            }
        }
    }
    let f = last_named.or(last_any)?;
    Some(graffy_core::exec::RepairFeedback {
        failure_id: f.id.clone(),
        mode: graffy_proto::mcw::v1::FailureMode::try_from(f.mode)
            .unwrap_or(graffy_proto::mcw::v1::FailureMode::Unspecified),
        critique: f.early_signal.clone(),
        source_attempt: attempt,
    })
}

/// Print one run's summary and index it in the store (shared by the plain
/// and TUI paths, and by every retry attempt).
async fn report_outcome(spec: &GraphSpec, spec_text: &str, outcome: RunOutcome) {
    println!("\nrun     : {}", outcome.run_id);
    println!("session : {}", outcome.session_id);
    println!("status  : {:?}", outcome.status);
    println!(
        "tokens  : {} in / {} out   cost: ${:.4}   wall: {}ms",
        outcome.input_tokens, outcome.output_tokens, outcome.total_usd, outcome.duration_ms
    );
    for note in &outcome.notes {
        println!("note    : {note}");
    }
    println!("journal : {}", outcome.journal_path.display());
    println!(
        "inspect : graffy replay {} --tui",
        outcome.journal_path.display()
    );

    match index_run(spec, spec_text, &outcome).await {
        Ok(events) => println!(
            "stored  : run indexed, {events} events mirrored to {}",
            db_path().display()
        ),
        Err(err) => {
            eprintln!("warning : store indexing failed ({err}) — journal file remains canonical")
        }
    }

    if let Some(text) = outcome.final_text {
        println!("\n================ verified response ================\n{text}");
    }
}

/// Load a session's EXECUTION runs (rating observations excluded), ordered
/// by validated manifest timestamps with deterministic tie-breaking — never
/// by filename (P0.1).
fn load_session_runs(
    dir: &std::path::Path,
    session: &str,
) -> Vec<Vec<graffy_core::journal::wire::RunEvent>> {
    use graffy_core::journal::wire::run_event::Event as JEvent;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "journal"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort(); // read order only; NOT the session order
    let mut runs: Vec<Vec<graffy_core::journal::wire::RunEvent>> = Vec::new();
    for p in &paths {
        if let Ok(events) = JournalReader::read_all(p) {
            let manifest = events.iter().find_map(|e| match &e.event {
                Some(JEvent::RunStarted(m)) => Some(m.clone()),
                _ => None,
            });
            if let Some(m) = manifest
                && m.session_id == session
                && m.graph_id != "graffy.mcw.rating"
                && m.run_kind != graffy_core::journal::wire::RunKind::RatingObservation as i32
            {
                runs.push(events);
            }
        }
    }
    graffy_core::boundary::order_runs(&mut runs);
    runs
}

/// Canonical rubric pin (immutable: framework commit + rubric content hash)
/// plus the adaptation identifier, per §6 of the conformance review. The
/// `-draft` suffix drops only after the section-8 gates pass AND the
/// framework author explicitly ratifies that exact version.
const HRDM_RUBRIC_VERSION: &str = "mcw-hrdm@8365d220+sha256.87ed2be6 + graffy-journal-map@v1-draft";
const HRDM_ADAPTATION_VERSION: &str = "graffy-journal-map@v1-draft";
const HRDM_SEGMENTATION_VERSION: &str = "boundary-projection@v1";

// Canonical anchors, quoted from the pinned mcw-framework
// docs/experiments/hrdm_rubrics.md (displayed in full during rating — the
// framework page remains the source of truth).
const H_ANCHORS: &str = "H — MCW Health (0-3), per five-exchange window:\n\
  H3 strong   — no misalignment surfaces; references to earlier content used correctly; at most one trivial clarification, resolved within a single exchange.\n\
  H2 adequate — minor misalignment surfaces but is repaired within the window in <= 2 dedicated repair turns; no completed work discarded or redone.\n\
  H1 strained — at least one misalignment forces rework or discarding of a work product, OR the same IU requires >= 2 clarification attempts without convergence.\n\
  H0 broken   — the parties demonstrably pursue different goals or referents; a work product is rejected wholesale; or the interaction is abandoned or reset.";
const R_ANCHORS: &str = "R — Repair Cost (0-3), per EXTERNAL repair episode:\n\
  R0 low  — realignment within <= 1 dedicated repair turn; no work discarded.\n\
  R1      — realignment within 2 dedicated repair turns, or minor rework.\n\
  R2      — 3-5 dedicated repair turns, or a work product substantially redone.\n\
  R3 high — > 5 dedicated repair turns, a full restart/reset, or repair abandoned with the misalignment left standing.";
const D_ANCHORS: &str = "D — Drift Rate (0-3), per window, via LATE DISCOVERIES (misalignments surfacing >= 3 completed exchanges after the turn that introduced them; both ends need citations):\n\
  D0 stable — no late discoveries; misalignment surfaces within 2 exchanges of introduction.\n\
  D1        — exactly one late discovery in the window.\n\
  D2        — two late discoveries, or one whose introducing turn lies more than 10 exchanges back.\n\
  D3 rapid  — three or more late discoveries, or end-of-window goal statements materially disagree.";
const M_ANCHORS: &str = "M — Misattribution (0-3), per window, Human-AI only. A COMPLETE transcript with NO capability-blame statements is M0. Reserve unratable for an INCOMPLETE record or unverifiable capability-vs-context evidence — absence of blame is not a reason to mark unratable, and never infer untyped human thoughts:\n\
  M0 none     — no capability-blame statements; failures attributed to information not exchanged.\n\
  M1          — capability blame expressed once but withdrawn or corrected during repair.\n\
  M2          — blame recurs (>= 2 statements) without verification, or capability-flavored corrective action while the needed IU was never externalized.\n\
  M3 frequent — the interaction strategy reorganizes around presumed incapability while the transcript shows the needed IU was never sent.";

enum ScoreAnswer {
    Score(i32),
    Unratable,
    Abort,
}

/// Read one anchored score from stdin: 0-3, 'u' (unratable — recorded as
/// absent WITH a required reason), or 'q'/EOF (abort — silence never
/// registers anything).
fn ask_score(lines: &mut dyn Iterator<Item = std::io::Result<String>>, label: &str) -> ScoreAnswer {
    loop {
        println!("{label} [0-3 / u / q]:");
        match lines.next() {
            None | Some(Err(_)) => return ScoreAnswer::Abort,
            Some(Ok(line)) => match line.trim() {
                "q" | "Q" => return ScoreAnswer::Abort,
                "u" | "U" => return ScoreAnswer::Unratable,
                s => {
                    if let Ok(v) = s.parse::<i32>()
                        && (0..=3).contains(&v)
                    {
                        return ScoreAnswer::Score(v);
                    }
                    println!("  enter 0-3, 'u' for unratable, or 'q' to abort");
                }
            },
        }
    }
}

/// Read one free-text line (empty allowed); None = abort.
fn ask_line(
    lines: &mut dyn Iterator<Item = std::io::Result<String>>,
    label: &str,
) -> Option<String> {
    println!("{label}");
    match lines.next() {
        None | Some(Err(_)) => None,
        Some(Ok(line)) => Some(line.trim().to_owned()),
    }
}

fn rating_aborted() -> anyhow::Result<()> {
    println!("\naborted — nothing was recorded (silence never registers).");
    Ok(())
}

/// `graffy rate <session>` — human H/R/D/M sampling over the EXTERNAL
/// boundary transcript (docs/mcw/hrdm-in-graffy.md). graffy reconstructs the
/// conversation the human and AI actually had, proposes canonical units, and
/// separates internal orchestration into telemetry that is never rated. The
/// anchored judgments are the human's; every score carries citations,
/// provenance, immutable pins, and calibration status.
fn cmd_rate(
    session: String,
    dir: Option<PathBuf>,
    rater: Option<String>,
    blinded: bool,
) -> anyhow::Result<()> {
    use graffy_core::boundary::{self, BoundaryActor};
    use std::io::BufRead;

    let dir = dir.unwrap_or_else(runs_dir);
    let runs_events = load_session_runs(&dir, &session);
    if runs_events.is_empty() {
        anyhow::bail!(
            "no runs found for session '{session}' in {} — every run prints its session id",
            dir.display()
        );
    }

    let projection = boundary::project(&runs_events);
    let blinding_profile = if blinded {
        format!("{}+redacted-cli", boundary::BLINDING_PROFILE)
    } else {
        boundary::BLINDING_PROFILE.to_owned()
    };

    println!(
        "rating session {session} — boundary projection ({}), adaptation {}",
        blinding_profile, HRDM_ADAPTATION_VERSION
    );
    println!("STATUS: calibration-only — this adaptation is DRAFT (not ratified).\n");
    println!(
        "internal orchestration telemetry (NOT canonical H/R/D/M): {} runs · {} retry-carrying · {} failure signals · {} repair actions · {} without a verified response",
        projection.internal.runs,
        projection.internal.retry_runs,
        projection.internal.failure_signals,
        projection.internal.repair_actions,
        projection.internal.runs_without_verified_response
    );
    println!("\nboundary transcript ({} turns):", projection.turns.len());
    for (i, t) in projection.turns.iter().enumerate() {
        let actor = match t.actor {
            BoundaryActor::Human => "HUMAN",
            BoundaryActor::Ai => "AI   ",
        };
        println!("  [{}] {} {:?} ({})", i, actor, t.role, t.refs.join(" "));
        for line in t.content.lines() {
            println!("      {line}");
        }
    }
    println!(
        "\nproposed segmentation: {} completed exchanges · {} complete window(s) · {} trailing exchange(s) NOT scored · {} candidate external repair episode(s)",
        projection.exchanges.len(),
        projection.complete_windows.len(),
        projection.trailing_exchanges,
        projection.repair_episodes.len()
    );

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(seg_answer) = ask_line(
        &mut lines,
        "accept the proposed segmentation? [enter = yes / type a correction note — corrections are kept as audit trail, the mechanical proposal is preserved]:",
    ) else {
        return rating_aborted();
    };
    let seg_note = if seg_answer.is_empty() {
        "segmentation: accepted as proposed".to_owned()
    } else {
        format!("segmentation correction (audit): {seg_answer}")
    };

    let rater_id = rater
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "human".to_owned());
    let mut samples: Vec<graffy_proto::mcw::v1::HrdmSample> = Vec::new();
    let base_sample = |unit_id: String, refs: Vec<String>| graffy_proto::mcw::v1::HrdmSample {
        sampled_at: Some(graffy_core::exec::now_ts()),
        session_id: session.clone(),
        run_id: String::new(),
        scope: unit_id.clone(),
        health: None,
        repair_cost: None,
        drift: None,
        misattribution: None,
        source: graffy_proto::mcw::v1::ScoreSource::HumanRater as i32,
        rubric_version: HRDM_RUBRIC_VERSION.to_owned(),
        rater_id: rater_id.clone(),
        note: seg_note.clone(),
        unit_id,
        evidence_refs: refs,
        adaptation_version: HRDM_ADAPTATION_VERSION.to_owned(),
        blinding_profile: blinding_profile.clone(),
        segmentation_version: HRDM_SEGMENTATION_VERSION.to_owned(),
        calibration_status: "calibration".to_owned(),
        unratable_reasons: String::new(),
    };

    if projection.complete_windows.is_empty() {
        println!(
            "\nno complete five-exchange windows: this session is NOT eligible for canonical H/D/M \
             (a retry session is one attempted exchange, not five — see adaptation §1.3). \
             Build exchanges with:  graffy run <graph> --prompt \"...\" --session {session}"
        );
    }

    for (wi, (ws, we)) in projection.complete_windows.iter().enumerate() {
        println!(
            "\n===== window {} (exchanges {}-{}) =====",
            wi + 1,
            ws + 1,
            we + 1
        );
        let mut refs: Vec<String> = Vec::new();
        for ex in &projection.exchanges[*ws..=*we] {
            for idx in [ex.human_turn, ex.ai_turn] {
                let t = &projection.turns[idx];
                refs.extend(t.refs.iter().cloned());
                println!(
                    "  [{idx}] {} {:?}: {}",
                    if t.actor == BoundaryActor::Human {
                        "HUMAN"
                    } else {
                        "AI"
                    },
                    t.role,
                    t.content.lines().next().unwrap_or("")
                );
            }
        }
        let mut reasons: Vec<String> = Vec::new();
        println!("\n{H_ANCHORS}");
        let h = match ask_score(&mut lines, "score H") {
            ScoreAnswer::Abort => return rating_aborted(),
            ScoreAnswer::Unratable => {
                let Some(r) = ask_line(&mut lines, "  unratable reason for H (required):") else {
                    return rating_aborted();
                };
                reasons.push(format!("H: {r}"));
                None
            }
            ScoreAnswer::Score(v) => Some(v),
        };
        println!("\n{D_ANCHORS}");
        let d = match ask_score(&mut lines, "score D") {
            ScoreAnswer::Abort => return rating_aborted(),
            ScoreAnswer::Unratable => {
                let Some(r) = ask_line(&mut lines, "  unratable reason for D (required):") else {
                    return rating_aborted();
                };
                reasons.push(format!("D: {r}"));
                None
            }
            ScoreAnswer::Score(v) => Some(v),
        };
        println!("\n{M_ANCHORS}");
        let m = match ask_score(&mut lines, "score M") {
            ScoreAnswer::Abort => return rating_aborted(),
            ScoreAnswer::Unratable => {
                let Some(r) = ask_line(
                    &mut lines,
                    "  unratable reason for M (required — incomplete record / unverifiable evidence; absence of blame is M0, not unratable):",
                ) else {
                    return rating_aborted();
                };
                reasons.push(format!("M: {r}"));
                None
            }
            ScoreAnswer::Score(v) => Some(v),
        };
        let mut s = base_sample(format!("window:{}", wi + 1), refs);
        s.health = h;
        s.drift = d;
        s.misattribution = m;
        s.unratable_reasons = reasons.join("; ");
        samples.push(s);
    }

    let mut confirmed_episodes = 0usize;
    for (ei, ep) in projection.repair_episodes.iter().enumerate() {
        let t = &projection.turns[ep.initiating_turn];
        println!(
            "\n===== candidate external repair episode {} — initiated at turn [{}] ({}) =====",
            ei + 1,
            ep.initiating_turn,
            t.refs.join(" ")
        );
        for line in t.content.lines() {
            println!("      {line}");
        }
        match ep.closed_at_exchange {
            Some(x) => println!("  proposed closure: exchange {}", x + 1),
            None => println!("  proposed closure: NOT closed in this session"),
        }
        let Some(confirm) = ask_line(
            &mut lines,
            "confirm this as an external repair episode? [enter = yes / n = exclude]:",
        ) else {
            return rating_aborted();
        };
        if confirm.eq_ignore_ascii_case("n") {
            println!("  excluded (kept in audit trail as unconfirmed proposal).");
            continue;
        }
        confirmed_episodes += 1;
        println!("\n{R_ANCHORS}");
        let r = match ask_score(&mut lines, "score R") {
            ScoreAnswer::Abort => return rating_aborted(),
            ScoreAnswer::Unratable => {
                let Some(reason) = ask_line(&mut lines, "  unratable reason for R (required):")
                else {
                    return rating_aborted();
                };
                let mut s = base_sample(
                    format!("repair_episode:turn-{}", ep.initiating_turn),
                    t.refs.clone(),
                );
                s.unratable_reasons = format!("R: {reason}");
                samples.push(s);
                continue;
            }
            ScoreAnswer::Score(v) => Some(v),
        };
        let mut s = base_sample(
            format!("repair_episode:turn-{}", ep.initiating_turn),
            t.refs.clone(),
        );
        s.repair_cost = r;
        samples.push(s);
    }

    // Canonical R_ev: confirmed EXTERNAL episodes per 10 completed exchanges.
    let r_ev = boundary::r_ev(confirmed_episodes, projection.exchanges.len());
    if let Some(rate) = r_ev {
        println!("\nR_ev (confirmed external episodes per 10 exchanges): {rate:.2}");
    }

    if samples.is_empty() {
        println!("\nnothing was scored — nothing recorded.");
        return Ok(());
    }

    let pseudo_run = graffy_core::id::RunId::generate().to_string();
    let out_path = dir.join(format!("{}-rating.journal", ulid_like_stamp()));
    let mut writer = JournalWriter::create(&out_path, &pseudo_run)?;
    use graffy_core::journal::wire::run_event::Event as JEvent;
    writer.append(JEvent::RunStarted(wire::RunManifest {
        run_id: pseudo_run.clone(),
        graph_id: "graffy.mcw.rating".to_owned(),
        graph_name: "HRDM rating session".to_owned(),
        graph_version: env!("CARGO_PKG_VERSION").to_owned(),
        spec_sha256: String::new(),
        session_id: session.clone(),
        started_at: Some(graffy_core::exec::now_ts()),
        evidence_mode: "strict".to_owned(),
        evidence_min_level: String::new(),
        graffy_version: env!("CARGO_PKG_VERSION").to_owned(),
        attempt_group_id: pseudo_run.clone(),
        attempt_index: 1,
        retry_of_run_id: String::new(),
        run_kind: wire::RunKind::RatingObservation as i32,
    }))?;
    let count = samples.len();
    for s in samples {
        writer.append(JEvent::HrdmSampled(s))?;
    }
    writer.append(JEvent::RunFinished(wire::RunFinished {
        status: wire::RunStatus::Succeeded as i32,
        summary: format!(
            "hrdm CALIBRATION rating: {count} sample(s) by {rater_id} · {} · R_ev {} · {seg_note}",
            HRDM_RUBRIC_VERSION,
            r_ev.map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "n/a".to_owned()),
        ),
        ..Default::default()
    }))?;
    println!(
        "\nrecorded {count} calibration sample(s) → {}",
        out_path.display()
    );
    println!("(draft adaptation: these are calibration data, never headline data.)");
    Ok(())
}

/// Connect the MCP servers a spec's tool.invoke nodes reference (design doc
/// §5: specs name servers logically; the store binds transports).
async fn build_tool_plane(spec: &GraphSpec) -> anyhow::Result<Option<Arc<dyn ToolInvoker>>> {
    let needed: std::collections::BTreeSet<String> = spec
        .nodes
        .iter()
        .filter(|n| n.kind == "tool.invoke")
        .filter_map(|n| {
            n.params
                .get("server")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .collect();
    if needed.is_empty() {
        return Ok(None);
    }
    let store = open_store().await?;
    let mut bindings = Vec::new();
    for name in &needed {
        let Some(record) = store.get_mcp_server(name).await? else {
            anyhow::bail!(
                "this graph needs MCP server '{name}', which is not registered — \
                 run: graffy mcp add {name} --stdio \"<command…>\""
            );
        };
        if record.transport != "stdio" {
            anyhow::bail!(
                "server '{name}' uses transport '{}' — only stdio is wired in this slice",
                record.transport
            );
        }
        bindings.push(ServerBinding {
            name: record.name,
            command: record.command,
            args: record.args.split_whitespace().map(str::to_owned).collect(),
        });
    }
    println!(
        "tool plane: connecting {}",
        needed.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let invoker = RegistryToolInvoker::connect_all(bindings).await?;
    Ok(Some(Arc::new(invoker)))
}

async fn mcp_add(
    name: String,
    stdio: String,
    role: Option<String>,
    evidence_level: String,
    knowledge_flag: Option<String>,
    skip_interview: bool,
    no_facades: bool,
) -> anyhow::Result<()> {
    let mut parts = stdio.split_whitespace().map(str::to_owned);
    let command = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("--stdio needs a command line"))?;
    let args: Vec<String> = parts.collect();
    println!(
        "connecting '{name}' via stdio: {command} {}",
        args.join(" ")
    );
    let server = LiveServer::connect_stdio(&command, &args).await?;
    let discovery = server.discover().await?;

    println!("\ndiscovered {} tools:", discovery.tools.len());
    for tool in &discovery.tools {
        let seeded = graffy_mcp::seed_role(tool, role.as_deref().unwrap_or("effector"));
        println!(
            "  {:<28} role: {:<9} read_only:{:<12} destructive:{:<12} {}",
            tool.name,
            seeded,
            format!("{:?}", tool.read_only),
            format!("{:?}", tool.destructive),
            truncate_chars(&tool.description, 44)
        );
    }
    if !discovery.prompts.is_empty() {
        println!("\nserver-shipped skills (MCP prompts):");
        for prompt in &discovery.prompts {
            println!(
                "  {:<28} content:{:<4} {}",
                prompt.name,
                if prompt.content.is_some() {
                    "yes"
                } else {
                    "no"
                },
                truncate_chars(&prompt.description, 52)
            );
        }
    }

    // Usage knowledge, in order of authority: explicit flag > server-shipped
    // prompts > the interview > nothing (design doc §2/§4).
    let prompt_knowledge = graffy_mcp::usage_knowledge_from_prompts(&discovery.prompts);
    let mut usage_knowledge = knowledge_flag
        .clone()
        .or(prompt_knowledge)
        .unwrap_or_default();
    let mut server_role = role.clone();

    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let should_interview = server_role.is_none()
        && knowledge_flag.is_none()
        && usage_knowledge.is_empty()
        && !skip_interview
        && interactive;
    if should_interview {
        let (interview_role, interview_knowledge) =
            run_usage_interview(&name, &discovery.tools).await;
        if let Some(r) = interview_role {
            server_role = Some(r.to_owned());
        }
        usage_knowledge = interview_knowledge;
    } else if usage_knowledge.is_empty() && server_role.is_none() {
        println!(
            "\n(no usage knowledge: server ships none and the interview was skipped — \
             facades run on schemas alone; add some later with --knowledge)"
        );
    }
    let server_default = server_role.as_deref().unwrap_or("effector");

    let store = open_store().await?;
    let tools_meta: Vec<serde_json::Value> = discovery
        .tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "read_only": t.read_only,
                "destructive": t.destructive,
            })
        })
        .collect();
    store
        .add_mcp_server(&McpServerRecord {
            name: name.clone(),
            transport: "stdio".to_owned(),
            command,
            args: args.join(" "),
            role_default: server_default.to_owned(),
            evidence_level: evidence_level.clone(),
            tools_json: serde_json::to_string(&tools_meta)?,
            usage_knowledge: usage_knowledge.clone(),
            added_at: unix_now_secs(),
        })
        .await?;

    let knowledge_opt = if usage_knowledge.is_empty() {
        None
    } else {
        Some(usage_knowledge.as_str())
    };
    let mut facades = 0usize;
    if !no_facades {
        for tool in &discovery.tools {
            let seeded = graffy_mcp::seed_role(tool, server_default);
            let facade =
                graffy_mcp::generate_facade(&name, tool, seeded, &evidence_level, knowledge_opt);
            let toml_text = facade.to_toml_string()?;
            store.register_graph(&toml_text, "mcp-facade").await?;
            facades += 1;
        }
    }
    server.shutdown().await;

    println!("\nregistered server '{name}' with {facades} skill-fronted facade graphs");
    println!(
        "usage knowledge: {}",
        if usage_knowledge.is_empty() {
            "none".to_owned()
        } else {
            format!(
                "{} chars fronting every prepare node",
                usage_knowledge.chars().count()
            )
        }
    );
    println!("list them : graffy graph list");
    println!(
        "run one   : graffy run graffy.mcp.{}.<tool> --prompt \"…\" [--offline] [--tui]",
        name
    );
    Ok(())
}

async fn graphify_cmd(
    path: PathBuf,
    name_override: Option<String>,
    force_prompt: bool,
    mode: String,
) -> anyhow::Result<()> {
    use graffy_graphs::graphify::{self, Mode};

    let Some(mode) = Mode::parse(&mode) else {
        anyhow::bail!("unknown mode '{mode}' — auto | guided | collaborative");
    };

    let raw = std::fs::read_to_string(&path)?;
    let fallback = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".to_owned());

    let looks_like_skill = !force_prompt
        && (raw.trim_start().starts_with("---") || raw.contains("\n# ") || raw.starts_with("# "));
    let mut spec = if looks_like_skill {
        let mut doc = graphify::parse_skill_md(&raw, &fallback)?;
        if let Some(name) = name_override {
            doc.name = name;
        }
        println!("graphifying skill '{}' ({mode:?})…", doc.name);
        graphify::graphify_skill(
            &doc,
            match mode {
                Mode::Auto => "auto",
                Mode::Guided => "guided",
                Mode::Collaborative => "collaborative",
            },
        )
    } else {
        let name = name_override.unwrap_or(fallback);
        println!("graphifying prompt '{name}' ({mode:?})…");
        graphify::graphify_prompt(
            &name,
            &raw,
            match mode {
                Mode::Auto => "auto",
                Mode::Guided => "guided",
                Mode::Collaborative => "collaborative",
            },
        )?
    };

    if mode != Mode::Auto {
        let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
        if interactive {
            let collaborative = mode == Mode::Collaborative;
            match graffy_tui::review_spec(&mut spec, collaborative)? {
                graffy_tui::ReviewDecision::Reject => {
                    println!("rejected in review — nothing was registered.");
                    return Ok(());
                }
                graffy_tui::ReviewDecision::Accept => {}
            }
        } else if mode == Mode::Guided {
            // Plain-terminal parity: show the exact TOML, ask, EOF rejects.
            println!("\n===== generated graph (nothing registered yet) =====\n");
            println!("{}", spec.to_toml_string()?);
            let answer = ask("register this graph? [y/N] ").await;
            if !answer.trim().eq_ignore_ascii_case("y") {
                println!("rejected — nothing was registered.");
                return Ok(());
            }
        } else {
            anyhow::bail!(
                "collaborative mode needs an interactive terminal (guided mode has piped \
                 parity: it prints the TOML and asks y/N)"
            );
        }
    }

    let toml_text = spec.to_toml_string()?;
    let store = open_store().await?;
    let record = store.register_graph(&toml_text, "graphified").await?;
    println!(
        "\nregistered '{}' — {} nodes, verified floor (intake → ground → apply → verify → respond)",
        record.id,
        spec.nodes.len()
    );
    println!("sha256  : {}", record.spec_sha256);
    println!(
        "run it  : graffy run {} --prompt \"…\" [--offline] [--tui]",
        record.id
    );
    println!("share it: graffy graph export {}", record.id);
    Ok(())
}

/// The v1 usage interview (design doc §4/§8): three plain questions on
/// stdin, judged by the pure logic in graffy_mcp::interview — the False
/// Alignment guard means a "read-only" claim never overrides a server's own
/// destructive annotations without an explicit override. Becomes a
/// first-class graph when the human-input node kind lands.
async fn run_usage_interview(
    server: &str,
    tools: &[graffy_mcp::DiscoveredTool],
) -> (Option<&'static str>, String) {
    use graffy_mcp::interview::{
        ClaimedRole, classify_change_answer, false_alignment, resolve_role,
    };

    println!("\nquick setup for '{server}' — three questions, Enter to skip any:");
    let mut knowledge_lines: Vec<String> = Vec::new();

    let q1 = ask("1) What do you usually use this server for? ").await;
    if !q1.trim().is_empty() {
        knowledge_lines.push(format!(
            "Owner's description of intended use: {}",
            q1.trim()
        ));
    }

    let q2 = ask("2) Does it change anything outside your machine, or just look things up? ").await;
    let mut claimed = classify_change_answer(&q2);
    if claimed == ClaimedRole::Ambiguous && !q2.trim().is_empty() {
        // Disambiguation repair, used preventively (§8).
        let follow = ask(
            "   Follow-up: does using it modify anything (files, services, messages)? [yes/no] ",
        )
        .await;
        claimed = classify_change_answer(&follow);
    }
    let mut override_confirmed = false;
    let conflict = false_alignment(claimed, tools);
    if let Some(destructive) = &conflict {
        println!(
            "   ⚠ the server itself marks these tools destructive: {} — keeping the safe \
             default (effector, approval-gated).",
            destructive.join(", ")
        );
        let over =
            ask("   Type 'override' to trust read-only anyway, Enter to accept the safe default: ")
                .await;
        override_confirmed = over.trim().eq_ignore_ascii_case("override");
    }
    let role = resolve_role(claimed, conflict.is_some(), override_confirmed);

    let q3 = ask("3) Should graphs reach for it automatically, or only when you ask? ").await;
    if !q3.trim().is_empty() {
        knowledge_lines.push(format!("Adoption policy: {}", q3.trim()));
    }

    let role_out = if q2.trim().is_empty() {
        None
    } else {
        Some(role)
    };
    (role_out, knowledge_lines.join("\n"))
}

async fn ask(prompt: &str) -> String {
    print!("{prompt}");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    tokio::task::spawn_blocking(|| {
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf).ok();
        buf
    })
    .await
    .unwrap_or_default()
}

async fn mcp_list() -> anyhow::Result<()> {
    let store = open_store().await?;
    let servers = store.list_mcp_servers().await?;
    if servers.is_empty() {
        println!("no MCP servers registered — graffy mcp add <name> --stdio \"<command…>\"");
        return Ok(());
    }
    println!(
        "{:<16} {:<8} {:<10} {:<6} {:<10} command",
        "name", "trans", "role", "level", "knowledge"
    );
    for s in servers {
        println!(
            "{:<16} {:<8} {:<10} {:<6} {:<10} {} {}",
            s.name,
            s.transport,
            s.role_default,
            s.evidence_level,
            if s.usage_knowledge.is_empty() {
                "-".to_owned()
            } else {
                format!("{}ch", s.usage_knowledge.chars().count())
            },
            s.command,
            s.args
        );
    }
    Ok(())
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

async fn index_run(
    spec: &GraphSpec,
    spec_text: &str,
    outcome: &graffy_core::exec::RunOutcome,
) -> anyhow::Result<usize> {
    let store = open_store().await?;
    let events = JournalReader::read_all(&outcome.journal_path)?;
    let (session_id, started_at) = events
        .iter()
        .find_map(|frame| match &frame.event {
            Some(wire::run_event::Event::RunStarted(m)) => Some((
                m.session_id.clone(),
                m.started_at.as_ref().map(|t| t.seconds).unwrap_or_default(),
            )),
            _ => None,
        })
        .unwrap_or_default();

    store
        .record_run(&RunRecord {
            run_id: outcome.run_id.clone(),
            graph_id: spec.graph.id.clone(),
            graph_name: spec.graph.name.clone(),
            session_id,
            status: format!("{:?}", outcome.status),
            started_at,
            duration_ms: outcome.duration_ms as i64,
            input_tokens: outcome.input_tokens as i64,
            output_tokens: outcome.output_tokens as i64,
            total_usd: outcome.total_usd,
            journal_path: outcome.journal_path.display().to_string(),
            spec_sha256: graffy_core::exec::sha256_hex(spec_text.as_bytes()),
        })
        .await?;
    Ok(store.mirror_journal(&events).await?)
}

async fn list_runs(limit: u32) -> anyhow::Result<()> {
    let store = open_store().await?;
    let runs = store.recent_runs(limit).await?;
    if runs.is_empty() {
        println!("no runs indexed yet — `graffy run …` records every run here.");
        return Ok(());
    }
    println!(
        "{:<30} {:<28} {:<10} {:>9} {:>9}  journal",
        "run", "graph", "status", "tok in", "tok out"
    );
    for run in runs {
        println!(
            "{:<30} {:<28} {:<10} {:>9} {:>9}  {}",
            run.run_id,
            run.graph_name,
            run.status,
            run.input_tokens,
            run.output_tokens,
            run.journal_path
        );
    }
    Ok(())
}

async fn graph_list() -> anyhow::Result<()> {
    let store = open_store().await?;
    let graphs = store.list_graphs().await?;
    println!("{:<48} {:<9} {:<9} name", "id", "version", "source");
    for graph in graphs {
        println!(
            "{:<48} {:<9} {:<9} {}",
            graph.id, graph.version, graph.source, graph.name
        );
    }
    println!("\nrun one:  graffy run <id> --prompt \"…\" [--offline] [--tui]");
    println!("export:   graffy graph export <id>");
    Ok(())
}

async fn graph_export(id: String, out: Option<PathBuf>) -> anyhow::Result<()> {
    let store = open_store().await?;
    let Some(record) = store.get_graph(&id).await? else {
        anyhow::bail!("no graph registered as '{id}' (see `graffy graph list`)");
    };
    let out = out.unwrap_or_else(|| PathBuf::from(format!("{id}.toml")));
    std::fs::write(&out, &record.spec_toml)?;
    println!(
        "exported '{}' v{} -> {}",
        record.id,
        record.version,
        out.display()
    );
    println!("sha256   : {}", record.spec_sha256);
    println!("share it : the TOML file IS the durable graph object — git, email, anywhere.");
    Ok(())
}

async fn graph_import(path: PathBuf) -> anyhow::Result<()> {
    let toml = std::fs::read_to_string(&path)?;
    let store = open_store().await?;
    let record = store.register_graph(&toml, "imported").await?;
    println!(
        "imported '{}' v{} ({} nodes validated: parse + compile + cycle guards)",
        record.id,
        record.version,
        GraphSpec::from_toml_str(&record.spec_toml)?.nodes.len()
    );
    println!("sha256  : {}", record.spec_sha256);
    println!("run it  : graffy run {} --prompt \"…\"", record.id);
    Ok(())
}

fn replay(journal: PathBuf, list_events: bool) -> anyhow::Result<()> {
    let events = JournalReader::read_all(&journal)?;
    let summary = summarize(&events);

    println!("run      : {}", summary.run_id);
    println!("graph    : {}", summary.graph_name);
    println!("status   : {:?}", summary.status);
    println!("events   : {}", summary.event_count);
    println!(
        "IUs {} | evidence {} | failures {} | repairs {} | model calls {} | routing {}",
        summary.iu_count,
        summary.evidence_count,
        summary.failure_signal_count,
        summary.repair_count,
        summary.model_calls,
        summary.routing_decisions
    );
    println!(
        "tokens   : {} in / {} out   cost: ${:.4}",
        summary.total_input_tokens, summary.total_output_tokens, summary.total_usd
    );
    println!("node states:");
    for (node, state) in &summary.node_states {
        println!("  {node:<12} {state:?}");
    }

    if list_events {
        println!("\nevent frames:");
        for frame in &events {
            println!("  #{:<4} {}", frame.seq, event_kind(frame));
        }
    }
    println!("\ntip: graffy replay {} --tui", journal.display());
    Ok(())
}

async fn doctor() -> anyhow::Result<()> {
    println!("graffy {}", env!("CARGO_PKG_VERSION"));
    println!(
        "proto packages: {}",
        graffy_proto::PROTO_PACKAGES.join(", ")
    );
    println!("home : {}", graffy_home().display());
    println!("store: {}", db_path().display());
    println!("runs : {}", runs_dir().display());
    match open_store().await {
        Ok(store) => {
            let (graphs, runs, events) = store.stats().await?;
            println!("  graphs {graphs} · runs {runs} · mirrored events {events}");
        }
        Err(err) => println!("  (store unavailable: {err})"),
    }
    if let Some(dirs) = directories::ProjectDirs::from("dev", "graffy", "graffy") {
        println!("config dir: {}", dirs.config_dir().display());
    }
    println!("\ntier bindings (GRAFFY_MODEL_*):");
    let bindings = graffy_providers::bindings_from_env();
    if bindings.is_empty() {
        println!("  none — live runs need GRAFFY_MODEL_FAST/_BALANCED/_FRONTIER=provider:model");
    } else {
        let mut tiers: Vec<_> = bindings.iter().collect();
        tiers.sort_by(|a, b| a.0.cmp(b.0));
        for (tier, binding) in tiers {
            println!(
                "  {tier:<10} -> {}:{}",
                binding.provider.name(),
                binding.model
            );
        }
    }
    println!("\ncredentials present (values never shown):");
    for key in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "VENICE_API_KEY",
        "OLLAMA_API_BASE_URL",
    ] {
        let set = std::env::var(key).is_ok_and(|v| !v.is_empty());
        println!("  {key:<20} {}", if set { "set" } else { "-" });
    }
    Ok(())
}

/// Plain-terminal approval parity (accessibility: every TUI capability has a
/// non-TUI equivalent). Asks on stdin; empty input / EOF REJECTS, so piped or
/// headless runs can never rubber-stamp a release gate.
struct CliApprovalHandler;

#[async_trait::async_trait]
impl ApprovalHandler for CliApprovalHandler {
    fn describe(&self) -> &'static str {
        "human-cli"
    }

    async fn resolve(&self, node_id: &str, question: &str) -> ApprovalOutcome {
        println!("\napproval required at node '{node_id}': {question}");
        println!("  [y]es approve · anything else rejects · 'e <text>' approves with an edit");
        let line = tokio::task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            buf
        })
        .await
        .unwrap_or_default();
        let line = line.trim();
        if line.eq_ignore_ascii_case("y") || line.eq_ignore_ascii_case("yes") {
            ApprovalOutcome::Approved
        } else if let Some(edit) = line.strip_prefix("e ") {
            ApprovalOutcome::Edited(edit.to_owned())
        } else {
            ApprovalOutcome::Rejected
        }
    }
}

/// Time-sortable journal file stamp (ULID via graffy-core's id types).
fn ulid_like_stamp() -> String {
    graffy_core::id::RunId::generate().to_string()
}
