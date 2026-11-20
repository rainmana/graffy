//! graffy — graph-native agentic harness.
//!
//! Nothing — no prompt, no skill, no chat turn — executes outside an
//! inspectable, durable, shareable agent graph. See docs/ARCHITECTURE.md.

mod input;
mod rating;
mod turn;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use graffy_core::exec::ToolInvoker;
use graffy_core::exec::{ApprovalHandler, ApprovalOutcome, ModelInvoker, OfflineEcho, RunOutcome};
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
        /// Apply a reviewed segmentation JSON file tied to this transcript hash.
        #[arg(long)]
        segmentation: Option<PathBuf>,
    },
    /// Execute a TOML graph spec against a prompt.
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
        /// Declare this follow-up as a human correction (requires --session).
        #[arg(long)]
        correction: bool,
    },
    /// Chat in the terminal. Every line you send still runs as a compiled,
    /// journaled graph (ADR-0010); one session carries the conversation.
    Chat {
        /// Graph each turn runs on: a TOML path or a registered graph id.
        /// Default: the session's own graph when resuming, otherwise the
        /// built-in conversation graph.
        #[arg(long)]
        graph: Option<String>,
        /// Deterministic offline echo invoker (no model, no network).
        #[arg(long)]
        offline: bool,
        /// Extra automatic attempts per turn: a number, or 'auto' (3).
        #[arg(long)]
        retry: Option<String>,
        /// Resume an existing session instead of starting a new one.
        #[arg(long)]
        session: Option<String>,
        /// Use the Ratatui conversation view (composer, transcript, inline
        /// approvals, run details) instead of the plain line-based chat.
        #[arg(long)]
        tui: bool,
        /// Route each message onto a registered graph that opted in with
        /// `route_on` (ADR-0012); anything else runs as plain conversation.
        /// `--route` matches trigger phrases. `--route=decider` asks a
        /// decision model instead (GRAFFY_DECIDER_URL, GRAFFY_DECIDER_MODEL,
        /// GRAFFY_DECIDER_API_KEY, GRAFFY_ROUTER_MIN_CONFIDENCE) — your
        /// messages are sent to that endpoint. Start a message with `/chat `
        /// to skip routing.
        #[arg(
            long,
            conflicts_with = "graph",
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = "phrases",
            value_parser = ["phrases", "decider"]
        )]
        route: Option<String>,
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

    let tui_active = matches!(
        &cli.command,
        None | Some(Command::Tui)
            | Some(Command::Run { tui: true, .. })
            | Some(Command::Chat { tui: true, .. })
            | Some(Command::Replay { tui: true, .. })
    );
    let default_filter = match cli.verbose {
        // Log lines written over a live TUI corrupt it; the journal is the record.
        0 if tui_active => "off",
        // Chat prints its own step progress; raw log lines would bury the conversation.
        0 if matches!(&cli.command, Some(Command::Chat { .. })) => "warn",
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
            segmentation,
        } => cmd_rate(session, dir, rater, blinded, segmentation).await,
        Command::Run {
            spec,
            prompt,
            tui,
            offline,
            journal,
            retry,
            session,
            correction,
        } => {
            run_graph(
                spec, prompt, tui, offline, journal, retry, session, correction,
            )
            .await
        }
        Command::Chat {
            graph,
            offline,
            retry,
            session,
            tui,
            route,
        } => chat(graph, offline, retry, session, tui, route).await,
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
    if aggregate.routing_runs > 0 {
        println!(
            "routing : {} routing run(s), {} decision-model token(s) — counted here only, never as execution runs",
            aggregate.routing_runs, aggregate.routing_tokens
        );
    }
    if aggregate.rating_observations > 0 {
        println!(
            "  ratings      : {} observation journal(s) — excluded from every execution aggregate",
            aggregate.rating_observations
        );
    }
    let _ = &attempt_groups;
    println!(
        "  repair fx    : run-passed {} of {} ({}) · legacy mode non-recurrence {} of {} assessed ({})",
        aggregate.repairs_run_passed,
        aggregate.repairs,
        rate_str(aggregate.repair_run_passed_rate),
        aggregate.legacy_mode_not_repeated,
        aggregate.legacy_recurrence_assessed,
        rate_str(aggregate.legacy_mode_nonrecurrence_rate)
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

#[allow(clippy::too_many_arguments)]
async fn run_graph(
    spec_arg: String,
    prompt: String,
    tui: bool,
    offline: bool,
    journal: Option<PathBuf>,
    retry: Option<String>,
    session: Option<String>,
    correction: bool,
) -> anyhow::Result<()> {
    let spec_text = resolve_spec(&spec_arg).await?;
    let spec = GraphSpec::from_toml_str(&spec_text)?;
    // Validate before starting any provider or MCP transport.
    graffy_core::graph::CompiledGraph::compile(&spec)?;
    let max_attempts = max_attempts_from(retry.as_deref())?;
    if tui && (max_attempts > 1 || session.is_some() || correction) {
        anyhow::bail!(
            "--retry, --session and --correction require plain CLI mode; `graffy chat --tui` is the interactive view"
        );
    }
    if correction && session.is_none() {
        anyhow::bail!("--correction requires --session so the repair has conversation context");
    }
    if journal.as_ref().is_some_and(|p| p.exists()) {
        anyhow::bail!(
            "journal already exists; choose a new path (existing journals are never overwritten)"
        );
    }
    let prior_boundary = match &session {
        Some(id) => hydrate_history(id).await?,
        None => Vec::new(),
    };
    let invoker = build_invoker(offline);
    let turn_started = std::time::Instant::now();
    let tool_plane = connect_tools(&spec).await?;
    let cancellation = graffy_core::control::Cancellation::default();
    // Both frontends drive the same runner (ADR-0010): only rendering differs.
    let turn = turn::Turn {
        spec: spec.clone(),
        spec_text: spec_text.clone(),
        prompt,
        correction,
        session,
        prior_boundary,
        max_attempts,
        journal,
        runs_dir: runs_dir(),
        started: turn_started,
        invoker,
        tools: tool_plane,
        approvals: Arc::new(CliApprovalHandler {
            input: input::Lines::stdin(),
        }),
        cancellation: cancellation.clone(),
        event_tap: None,
        routed_by: None,
    };
    if tui {
        let outcome = graffy_tui::run_live(&spec, move |tap, approvals, cancellation| async move {
            turn::TurnRunner::new(turn::Turn {
                event_tap: Some(tap),
                approvals,
                cancellation,
                ..turn
            })
            .attempt(|_, _| {})
            .await
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("run failed; see the journal"))?;
        report_outcome(&spec, &spec_text, &outcome, false).await?;
        if outcome.status != wire::RunStatus::Succeeded {
            anyhow::bail!("run ended {:?}", outcome.status);
        }
        return Ok(());
    }
    let cancel_on_signal = cancellation.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel_on_signal.cancel();
        }
    });
    let mut runner = turn::TurnRunner::new(turn);
    let result = drive_turn(&spec, &spec_text, &mut runner, false, &mut None).await;
    signal_task.abort();
    result?;
    println!(
        "continue session: graffy run {spec_arg} --prompt \"...\" --session {}",
        runner.session_id().unwrap_or_default()
    );
    Ok(())
}

/// `--retry` → total attempts per turn (the first attempt plus retries).
fn max_attempts_from(retry: Option<&str>) -> anyhow::Result<u32> {
    let retries = match retry {
        None => 0,
        Some("auto") => DEFAULT_AUTO_RETRIES,
        Some(n) => n
            .parse::<u32>()
            .map_err(|_| anyhow::anyhow!("--retry takes a number or 'auto', got '{n}'"))?,
    };
    retries
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("--retry is too large"))
}

/// Visible boundary history of an existing session, for model context.
/// Unknown sessions and unreadable history fail loudly — never "empty".
async fn hydrate_history(session: &str) -> anyhow::Result<Vec<(String, String)>> {
    let history = session_history(session).await?;
    if history.2 {
        eprintln!("warning: {UNCONFIRMED_HISTORY}");
    }
    Ok(history.0)
}

const UNCONFIRMED_HISTORY: &str =
    "history includes unconfirmed delivery; those contributions are excluded from model context";

/// Visible history, the id of the graph the session's latest run used, and
/// whether some history was excluded because its delivery is unconfirmed.
async fn session_history(session: &str) -> anyhow::Result<(Vec<(String, String)>, String, bool)> {
    let runs = load_session_runs(&runs_dir(), session).await?;
    if runs.is_empty() {
        anyhow::bail!("unknown session '{session}'; omit --session to start a new conversation");
    }
    let projection = graffy_core::boundary::project(&runs);
    let unconfirmed = projection
        .turns
        .iter()
        .any(|t| t.visibility != graffy_core::boundary::Visibility::Visible);
    let last_graph = runs
        .last()
        .and_then(|events| {
            events.iter().find_map(|e| match &e.event {
                Some(wire::run_event::Event::RunStarted(m)) => Some(m.graph_id.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();
    Ok((
        graffy_core::boundary::hydration_from(&projection),
        last_graph,
        unconfirmed,
    ))
}

/// Connect the graph's tool plane, bounded by the graph's own time budget.
async fn connect_tools(spec: &GraphSpec) -> anyhow::Result<Option<Arc<dyn ToolInvoker>>> {
    match spec.policy.budget.max_seconds {
        Some(seconds) => tokio::time::timeout(
            std::time::Duration::from_secs(seconds),
            build_tool_plane(spec),
        )
        .await
        .map_err(|_| anyhow::anyhow!("run deadline exhausted while connecting tools"))?,
        None => build_tool_plane(spec).await,
    }
}

/// Plain-terminal rendering of one turn: run attempts until the graph
/// releases an answer or the turn honestly ends with nothing released.
async fn drive_turn(
    spec: &GraphSpec,
    spec_text: &str,
    runner: &mut turn::TurnRunner,
    compact: bool,
    progress: &mut Option<tokio::sync::mpsc::UnboundedReceiver<wire::RunEvent>>,
) -> anyhow::Result<()> {
    // Step progress goes to stderr so stdout stays the conversation itself.
    fn show(frame: &wire::RunEvent) {
        if let Some(wire::run_event::Event::NodeTransition(t)) = &frame.event
            && t.to == wire::NodeState::Running as i32
        {
            eprintln!("  · step: {}", t.node_id);
        }
    }
    loop {
        let outcome = {
            let attempt = runner.attempt(|attempt, max| {
                if !compact {
                    println!(
                        "graph '{}' v{} — executing (attempt {attempt}/{max}, every step journaled)",
                        spec.graph.name, spec.graph.version
                    )
                }
            });
            tokio::pin!(attempt);
            loop {
                tokio::select! {
                    Some(frame) = async {
                        match progress.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => show(&frame),
                    outcome = &mut attempt => break outcome?,
                }
            }
        };
        // Everything the attempt recorded is shown before its answer is.
        while let Some(frame) = progress.as_mut().and_then(|rx| rx.try_recv().ok()) {
            show(&frame);
        }
        report_outcome(spec, spec_text, &outcome, compact).await?;
        match runner.verdict(&outcome)? {
            turn::Verdict::Released => {
                if runner.attempt_number() > 1 {
                    println!(
                        "converged on attempt {}/{} (internal execution only)",
                        runner.attempt_number(),
                        runner.max_attempts()
                    );
                }
                return Ok(());
            }
            turn::Verdict::Retry => {
                println!("retrying with recorded review feedback (whole-turn budget still applies)")
            }
        }
    }
}

/// `graffy chat`: a line-at-a-time conversation where every line is one turn
/// through the shared runner. Works piped (EOF ends the chat; it never
/// approves or sends anything).
async fn chat(
    graph: Option<String>,
    offline: bool,
    retry: Option<String>,
    mut session: Option<String>,
    tui: bool,
    route: Option<String>,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let (route, route_mode) = (route.is_some(), route);
    use std::io::{IsTerminal, Write};
    if tui && !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        anyhow::bail!("chat --tui needs a terminal; plain `graffy chat` works with piped input");
    }
    let resumed = match &session {
        Some(id) => Some(session_history(id).await?),
        None => None,
    };
    // An explicit pin wins. Otherwise a resumed chat stays on the graph it was
    // using — silently swapping graphs could drop that graph's approval gate.
    let graph_arg = match (&graph, &resumed) {
        (Some(pinned), _) => pinned.clone(),
        // With --route the floor is always plain conversation; the router
        // decides, message by message, whether another graph runs instead.
        (None, Some((_, session_graph, _))) if !route => session_graph.clone(),
        _ => "graffy.builtin.conversation".to_owned(),
    };
    let spec_text = resolve_spec(&graph_arg).await.with_context(|| {
        format!(
            "cannot load the graph for this chat ('{graph_arg}'); if the session used a graph file, pass --graph <path>"
        )
    })?;
    let spec = GraphSpec::from_toml_str(&spec_text)?;
    graffy_core::graph::CompiledGraph::compile(&spec)?;
    let max_attempts = max_attempts_from(retry.as_deref())?;
    if let Some((history, session_graph, unconfirmed)) = &resumed {
        if *unconfirmed {
            eprintln!("warning: {UNCONFIRMED_HISTORY}");
        }
        if session_graph != &spec.graph.id && !route {
            eprintln!(
                "note: this session last ran on graph '{session_graph}'; continuing on '{}' because --graph pins it",
                spec.graph.id
            );
        }
        // Resuming restores the recorded transcript; it re-executes nothing.
        if !tui {
            for (actor, content) in history {
                println!("{actor}> {content}");
            }
        }
    }
    let invoker = build_invoker(offline);
    let tools = connect_tools(&spec).await?;
    let graphs = Arc::new(
        ChatGraphs::load(
            spec.clone(),
            spec_text.clone(),
            tools,
            route_mode.as_deref(),
        )
        .await?,
    );
    // Repeat how this chat was started so the printed command resumes it alike.
    let resume_flags = match (&graph, route) {
        (Some(pinned), _) => format!(" --graph \"{pinned}\""),
        (None, true) if route_mode.as_deref() == Some("decider") => " --route=decider".to_owned(),
        (None, true) => " --route".to_owned(),
        (None, false) => String::new(),
    };
    if tui {
        let history = resumed.map(|r| r.0).unwrap_or_default();
        let session = chat_tui(graphs, max_attempts, invoker, history, session).await?;
        if let Some(id) = session {
            println!("resume this chat: graffy chat --tui --session {id}{resume_flags}");
        }
        return Ok(());
    }
    println!(
        "graffy chat — graph '{}' v{}. Every message runs as a journaled graph.\n\
         /correct <text> sends a correction · /quit or end-of-input leaves · Ctrl-C stops the current turn",
        spec.graph.name, spec.graph.version
    );
    if route {
        println!("{}", graphs.describe());
    }

    // Ctrl-C cancels the turn in flight; with no turn running it leaves chat.
    let active: Arc<std::sync::Mutex<Option<graffy_core::control::Cancellation>>> =
        Default::default();
    let active_for_signal = active.clone();
    let signal_task = tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            match active_for_signal.lock().unwrap().as_ref() {
                Some(cancellation) => cancellation.cancel(),
                None => std::process::exit(130),
            }
        }
    });

    let (tap, progress) = tokio::sync::mpsc::unbounded_channel();
    let mut progress = Some(progress);
    let interactive = std::io::stdin().is_terminal();
    // One reader serves this loop and the approval prompt, so a line typed
    // after a gate was abandoned reaches the chat instead of vanishing (#35).
    let input = input::Lines::stdin();
    // Piped lines are conversation. Only a human at a terminal can approve.
    let approvals: Arc<dyn ApprovalHandler> = if interactive {
        Arc::new(CliApprovalHandler {
            input: input.clone(),
        })
    } else {
        Arc::new(PipedChatApprovals)
    };
    let mut unreleased = 0u32;
    loop {
        if interactive {
            print!("\nyou> ");
            std::io::stdout().flush()?;
        }
        let Some(text) = input.next().await? else {
            break; // end of input: leave without sending anything
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if text == "/quit" {
            break;
        }
        // `/chat <text>` is the human's override: plain conversation, no routing.
        let (text, plain) = match text.strip_prefix("/chat ") {
            Some(rest) => (rest.trim(), true),
            None => (text, false),
        };
        let (prompt, correction) = match text.strip_prefix("/correct ") {
            Some(rest) => (rest.trim(), true),
            None => (text, false),
        };
        if correction && session.is_none() {
            eprintln!("nothing to correct yet: /correct needs an earlier turn in this chat");
            continue;
        }
        let prior_boundary = match &session {
            Some(id) => match hydrate_history(id).await {
                Ok(history) => history,
                Err(err) => {
                    unreleased += 1;
                    eprintln!("turn not started: {err}");
                    continue;
                }
            },
            None => Vec::new(),
        };
        let cancellation = graffy_core::control::Cancellation::default();
        *active.lock().unwrap() = Some(cancellation.clone());
        let chosen = match graphs
            .pick(prompt, plain, session.clone(), &invoker, &cancellation)
            .await
        {
            Ok(chosen) => chosen,
            Err(err) => {
                *active.lock().unwrap() = None;
                unreleased += 1;
                eprintln!("turn not started: routing failed: {err}");
                continue;
            }
        };
        if let Some(note) = &chosen.note {
            println!("routing : {note}");
        }
        let (spec, spec_text) = (&chosen.spec, &chosen.spec_text);
        let mut runner = turn::TurnRunner::new(turn::Turn {
            spec: spec.clone(),
            spec_text: spec_text.clone(),
            prompt: prompt.to_owned(),
            correction,
            session: session.clone(),
            prior_boundary,
            max_attempts,
            journal: None,
            runs_dir: runs_dir(),
            started: std::time::Instant::now(),
            invoker: invoker.clone(),
            tools: chosen.tools.clone(),
            approvals: approvals.clone(),
            cancellation,
            event_tap: Some(tap.clone()),
            routed_by: chosen.routed_by.clone(),
        });
        let result = drive_turn(spec, spec_text, &mut runner, true, &mut progress).await;
        *active.lock().unwrap() = None;
        if let Some(id) = runner.session_id() {
            session = Some(id.to_owned());
        }
        if let Err(err) = result {
            unreleased += 1;
            eprintln!("turn failed: {err}");
        }
    }
    signal_task.abort();
    if let Some(id) = &session {
        println!("\nresume this chat: graffy chat --session {id}{resume_flags}");
    }
    if unreleased > 0 {
        anyhow::bail!("{unreleased} turn(s) released no answer");
    }
    Ok(())
}

async fn report_outcome(
    spec: &GraphSpec,
    spec_text: &str,
    outcome: &RunOutcome,
    compact: bool,
) -> anyhow::Result<()> {
    use std::io::Write;
    let delivered = (|| -> anyhow::Result<()> {
        let stdout = std::io::stdout();
        let mut output = stdout.lock();
        if compact {
            // Chat shows the conversation; run details stay one replay away.
            if outcome.status != wire::RunStatus::Succeeded {
                writeln!(
                    output,
                    "attempt ended {:?}; no answer released",
                    outcome.status
                )?;
                for note in &outcome.notes {
                    writeln!(output, "note    : {note}")?;
                }
            }
        } else {
            writeln!(
                output,
                "\nrun     : {}\nsession : {}\nstatus  : {:?}",
                outcome.run_id, outcome.session_id, outcome.status
            )?;
            writeln!(
                output,
                "tokens  : {} in / {} out   cost: unavailable (pricing not configured)   wall: {}ms",
                outcome.input_tokens, outcome.output_tokens, outcome.duration_ms
            )?;
            for note in &outcome.notes {
                writeln!(output, "note    : {note}")?;
            }
            writeln!(
                output,
                "journal : {}\ninspect : graffy replay \"{}\" --tui",
                outcome.journal_path.display(),
                outcome.journal_path.display()
            )?;
        }
        if outcome.status == wire::RunStatus::Succeeded && outcome.final_text.is_some() {
            JournalWriter::deliver_response(&outcome.journal_path, &outcome.run_id, &mut output)?;
        }
        if compact {
            writeln!(
                output,
                "details : graffy replay \"{}\" --tui",
                outcome.journal_path.display()
            )?;
        }
        output.flush()?;
        Ok(())
    })();
    match index_run(spec, spec_text, outcome).await {
        Ok(_) => {}
        Err(err) => eprintln!("warning: indexing failed ({err}); journal remains canonical"),
    }
    delivered
}

async fn load_session_runs(
    dir: &std::path::Path,
    session: &str,
) -> anyhow::Result<Vec<Vec<graffy_core::journal::wire::RunEvent>>> {
    use graffy_core::journal::wire::run_event::Event as JEvent;
    let mut paths = std::collections::BTreeSet::new();
    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "journal") {
                paths.insert(std::fs::canonicalize(path)?);
            }
        }
    }
    if dir == runs_dir() {
        let store = open_store().await?;
        for path in store.session_journal_paths(session).await? {
            paths.insert(std::fs::canonicalize(&path).map_err(|err| {
                anyhow::anyhow!("session journal missing or unreadable: {path}: {err}")
            })?);
        }
    }
    let mut seen = std::collections::HashMap::new();
    let mut runs = Vec::new();
    for path in paths {
        let events = JournalReader::read_all(&path).map_err(|err| {
            anyhow::anyhow!(
                "cannot establish complete session history: {}: {err}",
                path.display()
            )
        })?;
        let manifests: Vec<_> = events
            .iter()
            .filter_map(|e| match &e.event {
                Some(JEvent::RunStarted(m)) => Some(m),
                _ => None,
            })
            .collect();
        if manifests.len() != 1 {
            anyhow::bail!("invalid journal manifest in {}", path.display());
        }
        let m = manifests[0];
        if m.session_id != session
            || m.graph_id == "graffy.mcw.rating"
            || m.run_kind == wire::RunKind::RatingObservation as i32
            // A routing run is about a prompt; it is never a conversation turn.
            || m.run_kind == wire::RunKind::Routing as i32
        {
            continue;
        }
        if !events
            .iter()
            .any(|e| matches!(e.event, Some(JEvent::RunFinished(_))))
        {
            anyhow::bail!(
                "unfinished session journal {}; inspect it with replay before continuing",
                path.display()
            );
        }
        let id = m.run_id.clone();
        if let Some(previous) = seen.get(&id) {
            if previous != &events {
                anyhow::bail!("conflicting journals for run {id}");
            }
            continue;
        }
        seen.insert(id, events.clone());
        runs.push(events);
    }
    graffy_core::boundary::order_runs(&mut runs);
    Ok(runs)
}

/// Canonical rubric pin (immutable: framework commit + rubric content hash)
/// plus the adaptation identifier, per §6 of the conformance review. The
/// `-draft` suffix drops only after the section-8 gates pass AND the
/// framework author explicitly ratifies that exact version.
const HRDM_RUBRIC_VERSION: &str = "mcw-hrdm@8365d220+sha256.87ed2be6 + graffy-journal-map@v2-draft";
const HRDM_ADAPTATION_VERSION: &str = "graffy-journal-map@v2-draft";
const HRDM_SEGMENTATION_VERSION: &str = "boundary-projection@v3";

/// The pinned anchor tables, quoted VERBATIM from the canonical rubric file
/// (commit 8365d220, sha256 87ed2be6). Rendered in full — never paraphrased.
const PINNED_ANCHORS: &str = include_str!("../docs/mcw/hrdm-anchors-pinned.md");

/// Extract one proxy's verbatim section from the pinned anchors document.
fn pinned_section(marker: &str) -> &'static str {
    let start = PINNED_ANCHORS.find(marker).unwrap_or(0);
    let rest = &PINNED_ANCHORS[start..];
    let end = rest[3..].find("\n## ").map(|i| i + 3).unwrap_or(rest.len());
    &rest[..end]
}

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

/// Read one free-text line; None = abort (EOF or explicit 'q').
fn ask_line(
    lines: &mut dyn Iterator<Item = std::io::Result<String>>,
    label: &str,
) -> Option<String> {
    println!("{label}");
    match lines.next() {
        None | Some(Err(_)) => None,
        Some(Ok(line)) => {
            let t = line.trim().to_owned();
            if t == "q" || t == "Q" { None } else { Some(t) }
        }
    }
}

/// Read a REQUIRED nonempty line (unratable reasons, citations); loops until
/// nonempty; None = abort.
fn ask_required(
    lines: &mut dyn Iterator<Item = std::io::Result<String>>,
    label: &str,
) -> Option<String> {
    loop {
        let answer = ask_line(lines, label)?;
        if !answer.is_empty() {
            return Some(answer);
        }
        println!("  a nonempty answer is required (or 'q' to abort)");
    }
}

fn rating_aborted() -> anyhow::Result<()> {
    println!("\naborted — nothing was recorded (silence never registers).");
    Ok(())
}

/// Only explicitly listed choices count as a segmentation decision.
fn ask_choice<R: std::io::BufRead>(
    lines: &mut std::io::Lines<R>,
    prompt: &str,
    choices: &[&str],
) -> Option<String> {
    loop {
        let answer = ask_line(lines, prompt)?;
        if choices
            .iter()
            .any(|choice| answer.eq_ignore_ascii_case(choice))
        {
            return Some(answer);
        }
        println!("  choose one of the listed actions; 'q' aborts.");
    }
}

/// `graffy rate <session>` — human H/R/D/M sampling over the EXTERNAL
/// boundary transcript (docs/mcw/hrdm-in-graffy.md). The mechanical proposal
/// is FROZEN and persisted; the rater's structured decisions (accept /
/// exclude / adjust) are persisted separately for every unit — even when
/// nothing is scored — and the corrected segmentation is what gets scored.
/// Aborting at ANY prompt writes nothing.
async fn cmd_rate(
    session: String,
    dir: Option<PathBuf>,
    rater: Option<String>,
    blinded: bool,
    segmentation: Option<PathBuf>,
) -> anyhow::Result<()> {
    use graffy_core::boundary::{self, BoundaryActor};
    use std::io::BufRead;

    let dir = dir.unwrap_or_else(runs_dir);
    let runs_events = load_session_runs(&dir, &session).await?;
    if runs_events.is_empty() {
        anyhow::bail!(
            "no runs found for session '{session}' in {} — every run prints its session id",
            dir.display()
        );
    }

    let mut projection = boundary::project(&runs_events);
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
    if blinded {
        println!("(blinded: internal retry/failure/repair telemetry suppressed)");
    } else {
        println!(
            "internal orchestration telemetry (NOT canonical H/R/D/M): {} runs · {} retry-carrying · {} failure signals · {} repair actions · {} without a verified response",
            projection.internal.runs,
            projection.internal.retry_runs,
            projection.internal.failure_signals,
            projection.internal.repair_actions,
            projection.internal.runs_without_verified_response
        );
    }
    println!("\nboundary transcript ({} turns):", projection.turns.len());
    for (i, t) in projection.turns.iter().enumerate() {
        let actor = match t.actor {
            BoundaryActor::Human => "HUMAN",
            BoundaryActor::Ai => "AI   ",
        };
        println!(
            "  [{}] {} {:?} {:?} ({})",
            i,
            actor,
            t.role,
            t.visibility,
            t.refs.join(" ")
        );
        for line in t.content.lines() {
            println!("      {line}");
        }
    }

    // FROZEN mechanical proposal (persisted verbatim + hashed).
    let mut episode_lines: Vec<String> = Vec::new();
    for (ei, ep) in projection.repair_episodes.iter().enumerate() {
        episode_lines.push(format!(
            "episode {} — initiating turn {} — {}",
            ei + 1,
            ep.initiating_turn,
            match ep.closed_at_exchange {
                Some(x) => format!("closed at exchange {}", x + 1),
                None => "OPEN (right-censored at session end)".to_owned(),
            }
        ));
    }
    let proposal_summary = format!(
        "turns {} · exchanges {} · complete windows {:?} · trailing {} · {}",
        projection.turns.len(),
        projection.exchanges.len(),
        projection.complete_windows,
        projection.trailing_exchanges,
        if episode_lines.is_empty() {
            "no episodes proposed".to_owned()
        } else {
            episode_lines.join(" | ")
        }
    );
    let frozen = rating::snapshot(&projection);
    let proposal_hash = graffy_core::exec::sha256_hex(frozen.as_bytes());
    println!("snapshot sha256: {proposal_hash}");
    println!("\nproposed segmentation (frozen): {proposal_summary}");

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let rater_id = rater
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "human".to_owned());

    let mut seg_records: Vec<wire::SegmentationRecord> = vec![wire::SegmentationRecord {
        unit_id: "projection".to_owned(),
        proposal: frozen.clone(),
        proposal_sha256: proposal_hash.clone(),
        decision: "frozen".to_owned(),
        correction: String::new(),
    }];
    if let Some(path) = segmentation {
        let input = std::fs::read_to_string(&path)?;
        let correction = rating::apply(&mut projection, &proposal_hash, &input)?;
        seg_records.push(wire::SegmentationRecord {
            unit_id: "projection".into(),
            proposal: frozen,
            proposal_sha256: proposal_hash.clone(),
            decision: "adjusted".into(),
            correction,
        });
        println!(
            "Applied reviewed segmentation: {} exchanges, {} windows, {} episodes.",
            projection.exchanges.len(),
            projection.complete_windows.len(),
            projection.repair_episodes.len()
        );
    }
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
        note: String::new(),
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
             (see adaptation §1.3). Build exchanges with:  graffy run <graph> --prompt \"...\" --session {session}"
        );
    }

    // Per-proxy scoring helper: (score, refs, reason) with citations
    // required whenever the anchor depends on observed events (score > 0).
    macro_rules! score_proxy {
        ($letter:expr, $marker:expr, $default_refs:expr, $cite_label:expr, $needs_cite:expr, $window:expr) => {{
            println!("\n{}", pinned_section($marker));
            match ask_score(&mut lines, concat!("score ", $letter)) {
                ScoreAnswer::Abort => return rating_aborted(),
                ScoreAnswer::Unratable => {
                    let Some(reason) = ask_required(
                        &mut lines,
                        concat!("  unratable reason for ", $letter, " (required):"),
                    ) else {
                        return rating_aborted();
                    };
                    (None, Vec::new(), Some(reason))
                }
                ScoreAnswer::Score(v) => {
                    #[allow(clippy::redundant_closure_call)]
                    let needs = ($needs_cite)(v);
                    let refs: Vec<String> = if needs {
                        loop {
                            let Some(cited) = ask_required(&mut lines, $cite_label) else {
                                return rating_aborted();
                            };
                            if let Some(refs) = rating::citation_refs(&projection, &cited, $window, $letter) {
                                break refs;
                            }
                            println!("  cite visible turns in this window; D needs introduction/discovery >=3 exchanges apart; M requires human turns");
                        }
                    } else {
                        $default_refs
                    };
                    (Some(v), refs, None)
                }
            }
        }};
    }

    for (wi, (ws, we)) in projection.complete_windows.iter().enumerate() {
        let unit = format!("window:{}", wi + 1);
        let bounds = format!("exchanges {}-{}", ws + 1, we + 1);
        println!("\n===== {unit} ({bounds}) =====");
        let mut window_refs: Vec<String> = Vec::new();
        for ex in &projection.exchanges[*ws..=*we] {
            for idx in [ex.human_turn, ex.ai_turn] {
                window_refs.extend(projection.turns[idx].refs.iter().cloned());
            }
        }
        let Some(decision) = ask_choice(
            &mut lines,
            "score this window? [enter = accept / x = exclude]:",
            &["", "x"],
        ) else {
            return rating_aborted();
        };
        if decision.eq_ignore_ascii_case("x") {
            let Some(why) = ask_required(&mut lines, "  exclusion note (required):") else {
                return rating_aborted();
            };
            seg_records.push(wire::SegmentationRecord {
                unit_id: unit,
                proposal: bounds,
                proposal_sha256: proposal_hash.clone(),
                decision: "excluded".to_owned(),
                correction: why,
            });
            continue;
        }
        seg_records.push(wire::SegmentationRecord {
            unit_id: unit.clone(),
            proposal: bounds,
            proposal_sha256: proposal_hash.clone(),
            decision: "accepted".to_owned(),
            correction: String::new(),
        });

        let (h, h_refs, h_reason) = score_proxy!(
            "H",
            "## H — MCW Health",
            window_refs.clone(),
            "  cite the turn indices evidencing the misalignment/rework (e.g. 0,3):",
            |v: i32| v < 3,
            (*ws, *we)
        );
        let (d, d_refs, d_reason) = score_proxy!(
            "D",
            "## D — Drift Rate",
            window_refs.clone(),
            "  cite INTRODUCTION and DISCOVERY turn indices (e.g. 0,7):",
            |v: i32| v > 0,
            (*ws, *we)
        );
        let (m, m_refs, m_reason) = score_proxy!(
            "M",
            "## M — Misattribution",
            window_refs.clone(),
            "  cite the capability-blame turn indices (e.g. 4):",
            |v: i32| v > 0,
            (*ws, *we)
        );
        for (letter, score, refs, reason) in [
            ("H", h, h_refs, h_reason),
            ("D", d, d_refs, d_reason),
            ("M", m, m_refs, m_reason),
        ] {
            let mut s = base_sample(unit.clone(), refs);
            s.scope = format!("{unit}/{letter}");
            match letter {
                "H" => s.health = score,
                "D" => s.drift = score,
                _ => s.misattribution = score,
            }
            if let Some(r) = reason {
                s.unratable_reasons = format!("{letter}: {r}");
            }
            samples.push(s);
        }
    }

    let mut confirmed_episodes = 0usize;
    for (ei, ep) in projection.repair_episodes.iter().enumerate() {
        let unit = format!("repair_episode:turn-{}", ep.initiating_turn);
        let t = &projection.turns[ep.initiating_turn];
        let status = match ep.closed_at_exchange {
            Some(x) => format!("closed at exchange {}", x + 1),
            None => "OPEN — right-censored at session end (absence of another correction is not closure)".to_owned(),
        };
        println!("\n===== candidate external repair episode {} =====", ei + 1);
        println!(
            "  initiation: turn [{}] ({})",
            ep.initiating_turn,
            t.refs.join(" ")
        );
        for line in t.content.lines() {
            println!("      {line}");
        }
        println!("  proposed status: {status}");
        let Some(decision) = ask_choice(
            &mut lines,
            "confirm episode? [enter = accept / x = exclude / o = mark OPEN instead]:",
            &["", "x", "o"],
        ) else {
            return rating_aborted();
        };
        let (applied_closure, seg_decision, correction) = if decision.eq_ignore_ascii_case("x") {
            let Some(why) = ask_required(&mut lines, "  exclusion note (required):") else {
                return rating_aborted();
            };
            seg_records.push(wire::SegmentationRecord {
                unit_id: unit,
                proposal: status,
                proposal_sha256: proposal_hash.clone(),
                decision: "excluded".to_owned(),
                correction: why,
            });
            continue;
        } else if decision.eq_ignore_ascii_case("o") {
            (
                None,
                "adjusted",
                "rater marked episode OPEN (right-censored)".to_owned(),
            )
        } else {
            (ep.closed_at_exchange, "accepted", String::new())
        };
        seg_records.push(wire::SegmentationRecord {
            unit_id: unit.clone(),
            proposal: status.clone(),
            proposal_sha256: proposal_hash.clone(),
            decision: seg_decision.to_owned(),
            correction: correction.clone(),
        });
        confirmed_episodes += 1;

        // R citations are structural: initiation + repair work + closure.
        let mut refs: Vec<String> = t.refs.clone();
        let work_end = applied_closure
            .and_then(|x| projection.exchanges.get(x))
            .map(|ex| ex.ai_turn)
            .unwrap_or(projection.turns.len().saturating_sub(1));
        for turn in &projection.turns[ep.initiating_turn..=work_end] {
            refs.extend(turn.refs.iter().cloned());
        }
        refs.dedup();

        if applied_closure.is_none() {
            let mut s = base_sample(unit.clone(), refs);
            s.scope = format!("{unit}/R");
            s.unratable_reasons = "R: right-censored; total repair cost is not observed".into();
            s.note = "episode remains open; no numeric completed repair-cost score".into();
            samples.push(s);
            println!("R unratable: episode is open; completed repair cost is not observed.");
            continue;
        }
        println!("\n{}", pinned_section("## R — Repair Cost"));
        let r = match ask_score(&mut lines, "score R") {
            ScoreAnswer::Abort => return rating_aborted(),
            ScoreAnswer::Unratable => {
                let Some(reason) = ask_required(&mut lines, "  unratable reason for R (required):")
                else {
                    return rating_aborted();
                };
                let mut s = base_sample(unit.clone(), refs);
                s.scope = format!("{unit}/R");
                s.unratable_reasons = format!("R: {reason}");
                if applied_closure.is_none() {
                    s.note = "episode right-censored (open at session end)".to_owned();
                }
                samples.push(s);
                continue;
            }
            ScoreAnswer::Score(v) => Some(v),
        };
        let mut s = base_sample(unit.clone(), refs);
        s.scope = format!("{unit}/R");
        s.repair_cost = r;
        if applied_closure.is_none() {
            s.note = "episode right-censored (open at session end)".to_owned();
        }
        samples.push(s);
    }

    let r_ev = boundary::r_ev(confirmed_episodes, projection.exchanges.len());
    if let Some(rate) = r_ev {
        println!("\nR_ev (confirmed external episodes per 10 exchanges): {rate:.2}");
    }

    // Persist whenever ANY structured decision or sample exists — segmentation
    // audit records are kept even when nothing was scored. Only a full abort
    // (handled above, atomically) writes nothing.
    if samples.is_empty() && seg_records.len() <= 1 {
        println!("\nnothing was decided or scored — nothing recorded.");
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
        routed_by_run_id: String::new(),
    }))?;
    for record in &seg_records {
        writer.append(JEvent::Segmentation(record.clone()))?;
    }
    let count = samples.len();
    for s in samples {
        writer.append(JEvent::HrdmSampled(s))?;
    }
    writer.append(JEvent::RunFinished(wire::RunFinished {
        status: wire::RunStatus::Succeeded as i32,
        summary: format!(
            "hrdm CALIBRATION rating: {count} sample(s), {} segmentation record(s) by {rater_id} · {} · proposal sha256 {} · R_ev {}",
            seg_records.len(),
            HRDM_RUBRIC_VERSION,
            proposal_hash,
            r_ev.map(|v| format!("{v:.2}")).unwrap_or_else(|| "n/a".to_owned()),
        ),
        ..Default::default()
    }))?;
    println!(
        "\nrecorded {count} calibration sample(s) + {} segmentation record(s) → {}",
        seg_records.len(),
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
            journal_path: std::fs::canonicalize(&outcome.journal_path)?
                .display()
                .to_string(),
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
    // The decision-model router (`chat --route=decider`). Checked exactly as
    // chat would check it, without contacting the endpoint.
    println!("\ndecision-model router (chat --route=decider; nothing is contacted here):");
    if std::env::var("GRAFFY_DECIDER_URL").is_ok_and(|v| !v.trim().is_empty()) {
        match decider_from_env() {
            Ok(routing) => println!(
                "  ready — messages would be sent to {}; picks below confidence {:.2} are not followed",
                routing.decider.endpoint(),
                routing.min_confidence
            ),
            Err(err) => println!("  not usable: {err}"),
        }
    } else {
        println!(
            "  not configured — set GRAFFY_DECIDER_URL and GRAFFY_ROUTER_MIN_CONFIDENCE (docs/decider.md)"
        );
    }
    for key in ["GRAFFY_DECIDER_MODEL", "GRAFFY_DECIDER_API_KEY"] {
        let set = std::env::var(key).is_ok_and(|v| !v.is_empty());
        println!("  {key:<24} {}", if set { "set" } else { "-" });
    }
    Ok(())
}

/// Plain-terminal approval parity (accessibility: every TUI capability has a
/// non-TUI equivalent). Asks on stdin; empty input / EOF REJECTS, so piped or
/// headless runs can never rubber-stamp a release gate.
struct CliApprovalHandler {
    input: input::Lines,
}

#[async_trait::async_trait]
impl ApprovalHandler for CliApprovalHandler {
    fn describe(&self) -> &'static str {
        "human-cli"
    }

    fn is_human(&self) -> bool {
        true
    }

    async fn resolve(&self, node_id: &str, question: &str) -> ApprovalOutcome {
        use std::io::Write;
        {
            let stdout = std::io::stdout();
            let mut output = stdout.lock();
            if writeln!(output, "\napproval required at node '{node_id}': {question}\n[y]es approves; other input rejects. Edits must be submitted as a new correction turn.")
                .and_then(|()| output.flush()).is_err() { return ApprovalOutcome::Abandoned; }
        }
        // If the turn is cancelled this wait is dropped. The shared reader then
        // hands the next line to whoever asks next; it cannot decide this gate.
        let Ok(Some(line)) = self.input.next().await else {
            return ApprovalOutcome::Abandoned; // end of input or a read error
        };
        let observed = line.trim_end_matches(['\r', '\n']).to_owned();
        if matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            ApprovalOutcome::ApprovedInput(observed)
        } else {
            ApprovalOutcome::RejectedInput(observed)
        }
    }
}

/// The graph one chat turn runs on, and how it was chosen.
#[derive(Clone)]
struct TurnGraph {
    spec: GraphSpec,
    spec_text: String,
    tools: Option<Arc<dyn ToolInvoker>>,
    /// The routing run that chose it; `None` for a pin or a `/chat` override.
    routed_by: Option<String>,
    /// The recorded reason, shown to the human as one plain line.
    note: Option<String>,
}

/// Graph selection for chat: a fixed floor graph, plus — with `--route` — the
/// journaled prompt router over graphs that opted in (ADR-0012).
struct ChatGraphs {
    floor: TurnGraph,
    router: Option<PromptRouter>,
}

struct PromptRouter {
    spec: GraphSpec,
    spec_text: String,
    candidates: Vec<graffy_core::route::RouteCandidate>,
    /// Opted-in specs by graph id.
    specs: std::collections::HashMap<String, (GraphSpec, String)>,
    /// `Some`: a decision model chooses (`--route=decider`).
    decider: Option<graffy_core::route::DeciderRouting>,
    /// Tool planes are connected on first use and reused for the chat.
    tools: tokio::sync::Mutex<std::collections::HashMap<String, Option<Arc<dyn ToolInvoker>>>>,
}

impl ChatGraphs {
    async fn load(
        spec: GraphSpec,
        spec_text: String,
        tools: Option<Arc<dyn ToolInvoker>>,
        route: Option<&str>,
    ) -> anyhow::Result<Self> {
        let floor = TurnGraph {
            spec,
            spec_text,
            tools,
            routed_by: None,
            note: None,
        };
        let Some(mode) = route else {
            return Ok(Self {
                floor,
                router: None,
            });
        };
        let decider = match mode {
            "decider" => Some(decider_from_env()?),
            _ => None,
        };
        let spec_text = resolve_spec("graffy.builtin.router").await?;
        let spec = GraphSpec::from_toml_str(&spec_text)?;
        graffy_core::graph::CompiledGraph::compile(&spec)?;
        let mut candidates = Vec::new();
        let mut specs = std::collections::HashMap::new();
        for record in open_store().await?.list_graphs().await? {
            // The registry only holds lawful graphs; parse to read the opt-in.
            let candidate = GraphSpec::from_toml_str(&record.spec_toml)?;
            if candidate.graph.route_on.is_empty() {
                continue; // explicit-only
            }
            candidates.push(graffy_core::route::RouteCandidate {
                graph_id: candidate.graph.id.clone(),
                name: candidate.graph.name.clone(),
                route_on: candidate.graph.route_on.clone(),
                route_when: candidate
                    .graph
                    .route_when
                    .clone()
                    .unwrap_or_else(|| candidate.graph.description.clone()),
            });
            specs.insert(candidate.graph.id.clone(), (candidate, record.spec_toml));
        }
        Ok(Self {
            floor,
            router: Some(PromptRouter {
                spec,
                spec_text,
                candidates,
                specs,
                decider,
                tools: Default::default(),
            }),
        })
    }

    /// The spec of a graph this chat can run, by id — so a view describes the
    /// graph that actually executed rather than assuming the floor.
    fn spec_for(&self, graph_id: &str) -> Option<GraphSpec> {
        if graph_id == self.floor.spec.graph.id {
            return Some(self.floor.spec.clone());
        }
        let router = self.router.as_ref()?;
        router.specs.get(graph_id).map(|(spec, _)| spec.clone())
    }

    /// One plain line naming what the router can choose from.
    fn describe(&self) -> String {
        match &self.router {
            Some(router) if router.candidates.is_empty() => {
                "routing : on, but no registered graph declares route_on phrases — every message runs as plain conversation".into()
            }
            Some(router) => {
                let names = router
                    .candidates
                    .iter()
                    .map(|c| format!("'{}'", c.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                match &router.decider {
                    // The prompt leaves this process: say where, up front.
                    Some(routing) => format!(
                        "routing : on — a decision model at {} may choose {names}; picks below confidence {:.2} are not followed.\n\
                         routing : every routed message is SENT to {}. Start a message with /chat to keep it out of routing.",
                        routing.decider.endpoint(),
                        routing.min_confidence,
                        routing.decider.endpoint()
                    ),
                    None => format!(
                        "routing : on — may choose {names}. Start a message with /chat to use plain conversation instead."
                    ),
                }
            }
            None => String::new(),
        }
    }

    /// Choose the graph for one message. With routing on (and no `/chat`
    /// override) this executes the router graph as its own journaled run.
    async fn pick(
        &self,
        prompt: &str,
        plain: bool,
        session: Option<String>,
        invoker: &Arc<dyn ModelInvoker>,
        cancellation: &graffy_core::control::Cancellation,
    ) -> anyhow::Result<TurnGraph> {
        let Some(router) = self.router.as_ref().filter(|_| !plain) else {
            return Ok(self.floor.clone());
        };
        let mut registry = graffy_core::exec::NodeRegistry::with_defaults();
        registry.register(
            "router",
            Arc::new(graffy_core::route::RouterNode {
                candidates: router.candidates.clone(),
                floor: graffy_core::route::RouteCandidate {
                    graph_id: self.floor.spec.graph.id.clone(),
                    name: self.floor.spec.graph.name.clone(),
                    route_on: Vec::new(),
                    route_when: "Ordinary conversation, or none of the other options clearly fits."
                        .into(),
                },
                decider: router.decider.clone(),
            }),
        );
        let executor = graffy_core::exec::Executor {
            registry,
            cancellation: cancellation.clone(),
            ..Default::default()
        };
        let outcome = executor
            .run(
                &router.spec,
                &router.spec_text,
                graffy_core::exec::RunInput {
                    prompt: prompt.to_owned(),
                    // Before the first turn there is no session yet; that
                    // routing run carries its own id and is linked from the
                    // target run's manifest instead.
                    session_id: session,
                    run_kind: wire::RunKind::Routing as i32,
                    ..Default::default()
                },
                &runs_dir().join(format!("{}.journal", ulid_like_stamp())),
                invoker.as_ref(),
                &graffy_core::exec::AutoApprove,
            )
            .await?;
        if let Err(err) = index_run(&router.spec, &router.spec_text, &outcome).await {
            tracing::warn!("indexing the routing run failed ({err}); journal remains canonical");
        }
        if outcome.status != wire::RunStatus::Succeeded {
            anyhow::bail!("routing run ended {:?}", outcome.status);
        }
        let decision =
            graffy_core::route::decision_from(&JournalReader::read_all(&outcome.journal_path)?)
                .ok_or_else(|| anyhow::anyhow!("routing run recorded no decision"))?;
        let mut chosen = if decision.fallback {
            self.floor.clone()
        } else {
            let (spec, spec_text) = router
                .specs
                .get(&decision.chosen_graph_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("router chose unknown graph '{}'", decision.chosen_graph_id)
                })?
                .clone();
            let mut planes = router.tools.lock().await;
            let tools = match planes.get(&spec.graph.id) {
                Some(plane) => plane.clone(),
                None => {
                    let plane = connect_tools(&spec).await?;
                    planes.insert(spec.graph.id.clone(), plane.clone());
                    plane
                }
            };
            TurnGraph {
                spec,
                spec_text,
                tools,
                routed_by: None,
                note: None,
            }
        };
        chosen.routed_by = Some(outcome.run_id);
        chosen.note = Some(decision.reason);
        Ok(chosen)
    }
}

/// The decision-model router's configuration, from the environment. Nothing
/// is defaulted: no endpoint, no model, and no confidence threshold — graffy
/// has no evidence for a threshold on its own prompts (ADR-0012).
fn decider_from_env() -> anyhow::Result<graffy_core::route::DeciderRouting> {
    let decider = graffy_providers::decider::HttpDecider::from_env()
        .map_err(|err| anyhow::anyhow!("--route=decider: {err}"))?;
    let raw = std::env::var("GRAFFY_ROUTER_MIN_CONFIDENCE").map_err(|_| {
        anyhow::anyhow!(
            "--route=decider: GRAFFY_ROUTER_MIN_CONFIDENCE is not set. Choose the confidence \
             (between 0 and 1) below which the decision model's pick is ignored and plain \
             conversation runs; graffy ships no default."
        )
    })?;
    let min_confidence = raw
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| (0.0..=1.0).contains(v))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "GRAFFY_ROUTER_MIN_CONFIDENCE must be a number between 0 and 1, got '{raw}'"
            )
        })?;
    Ok(graffy_core::route::DeciderRouting {
        decider: Arc::new(decider),
        min_confidence,
    })
}

/// `graffy chat --tui`: the Ratatui conversation view over the same turn
/// runner. Returns the session to resume, if any turn ran.
async fn chat_tui(
    graphs: Arc<ChatGraphs>,
    max_attempts: u32,
    invoker: Arc<dyn ModelInvoker>,
    history: Vec<(String, String)>,
    session: Option<String>,
) -> anyhow::Result<Option<String>> {
    let current = Arc::new(std::sync::Mutex::new(session.clone()));
    let for_turns = current.clone();
    let for_reset = current.clone();
    let view_spec = graphs.floor.spec.clone();
    let for_view = graphs.clone();
    graffy_tui::run_chat(
        &view_spec,
        move |graph_id| for_view.spec_for(graph_id),
        history,
        // Routing disclosure (which endpoint receives messages, how to opt a
        // message out) is on screen before the first message can be sent.
        graphs
            .describe()
            .lines()
            .map(|line| line.trim_start_matches("routing : ").to_owned())
            .collect(),
        session,
        move |turn: graffy_tui::ChatTurn, tap, approvals, cancellation| {
            let (invoker, graphs, current) = (invoker.clone(), graphs.clone(), for_turns.clone());
            async move {
                let mut reply = graffy_tui::ChatReply::default();
                let session = current.lock().unwrap().clone();
                if turn.correction && session.is_none() {
                    reply
                        .notices
                        .push("nothing to correct yet: /correct needs an earlier turn".into());
                    return Ok(reply);
                }
                let prior_boundary = match &session {
                    Some(id) => {
                        let (history, _, unconfirmed) = session_history(id).await?;
                        if unconfirmed {
                            reply.notices.push(UNCONFIRMED_HISTORY.into());
                        }
                        history
                    }
                    None => Vec::new(),
                };
                let chosen = graphs
                    .pick(
                        &turn.prompt,
                        turn.plain,
                        session.clone(),
                        &invoker,
                        &cancellation,
                    )
                    .await
                    .map_err(|err| anyhow::anyhow!("routing failed: {err}"))?;
                reply.notices.extend(chosen.note.iter().map(|n| {
                    format!("routing: {n} — start a message with /chat for plain conversation")
                }));
                let (spec, spec_text) = (chosen.spec.clone(), chosen.spec_text.clone());
                let mut runner = turn::TurnRunner::new(turn::Turn {
                    spec: spec.clone(),
                    spec_text: spec_text.clone(),
                    prompt: turn.prompt,
                    correction: turn.correction,
                    session,
                    prior_boundary,
                    max_attempts,
                    journal: None,
                    runs_dir: runs_dir(),
                    started: std::time::Instant::now(),
                    invoker,
                    tools: chosen.tools.clone(),
                    approvals,
                    cancellation,
                    event_tap: Some(tap),
                    routed_by: chosen.routed_by.clone(),
                });
                // Same attempt loop as plain chat; notes instead of prints.
                let released = loop {
                    let outcome = match runner.attempt(|_, _| {}).await {
                        Ok(outcome) => outcome,
                        Err(err) => break Err(err),
                    };
                    let verdict = runner.verdict(&outcome);
                    if matches!(verdict, Ok(turn::Verdict::Released)) {
                        break Ok(outcome);
                    }
                    // An attempt that released nothing is indexed right away.
                    if let Err(err) = index_run(&spec, &spec_text, &outcome).await {
                        reply.notices.push(format!(
                            "indexing failed ({err}); journal remains canonical"
                        ));
                    }
                    reply.notices.extend(outcome.notes.iter().cloned());
                    reply.notices.push(format!(
                        "attempt {} ended {:?} — details: graffy replay \"{}\" --tui",
                        runner.attempt_number(),
                        outcome.status,
                        outcome.journal_path.display()
                    ));
                    if let Err(err) = verdict {
                        break Err(err);
                    }
                    reply
                        .notices
                        .push("retrying with recorded review feedback".into());
                };
                if let Some(id) = runner.session_id() {
                    *current.lock().unwrap() = Some(id.to_owned());
                    reply.session = Some(id.to_owned());
                }
                match released {
                    Ok(outcome) => {
                        reply.answer = JournalWriter::undelivered_response(
                            &outcome.journal_path,
                            &outcome.run_id,
                        )?
                        .map(|(label, content)| (label.to_owned(), content));
                        // Receipt first, then the index mirror, once the view
                        // has drawn the answer.
                        reply.confirm_shown = Some(Box::pin(async move {
                            JournalWriter::confirm_delivery(
                                &outcome.journal_path,
                                &outcome.run_id,
                            )?;
                            index_run(&spec, &spec_text, &outcome).await?;
                            Ok(())
                        }));
                    }
                    Err(err) => reply.notices.push(format!("{err}")),
                }
                Ok(reply)
            }
        },
        move || *for_reset.lock().unwrap() = None,
    )
    .await?;
    Ok(current.lock().unwrap().clone())
}

/// Approvals for piped `graffy chat`: every stdin line is a conversation
/// message, so none is ever read as a decision. The gate is abandoned and
/// nothing is released — a chat message saying "yes" must not approve.
struct PipedChatApprovals;

#[async_trait::async_trait]
impl ApprovalHandler for PipedChatApprovals {
    fn describe(&self) -> &'static str {
        "noninteractive-chat"
    }

    async fn resolve(&self, node_id: &str, _question: &str) -> ApprovalOutcome {
        eprintln!(
            "approval required at node '{node_id}', but chat input is not a terminal: abandoned, nothing released"
        );
        ApprovalOutcome::Abandoned
    }
}

/// Time-sortable journal file stamp (ULID via graffy-core's id types).
fn ulid_like_stamp() -> String {
    graffy_core::id::RunId::generate().to_string()
}
