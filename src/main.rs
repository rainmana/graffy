//! graffy — graph-native agentic harness.
//!
//! Nothing — no prompt, no skill, no chat turn — executes outside an
//! inspectable, durable, shareable agent graph. See docs/ARCHITECTURE.md.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use graffy_core::exec::{AutoApprove, Executor, ModelInvoker, OfflineEcho, RunInput};
use graffy_core::journal::{JournalReader, event_kind, summarize, wire};
use graffy_core::spec::GraphSpec;
use graffy_memory::{RunRecord, Store};
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
        /// Journal output path (default: graffy-runs/<ulid>.journal).
        #[arg(long)]
        journal: Option<PathBuf>,
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
    /// Print environment diagnostics.
    Doctor,
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
        Command::Tui => graffy_tui::run_home(),
        Command::Run {
            spec,
            prompt,
            tui,
            offline,
            journal,
        } => run_graph(spec, prompt, tui, offline, journal).await,
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
        Command::Doctor => doctor().await,
    }
}

// ---------------------------------------------------------------------------
// Store plumbing
// ---------------------------------------------------------------------------

fn db_path() -> PathBuf {
    if let Ok(dir) = std::env::var("GRAFFY_DATA_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("graffy.db");
    }
    directories::ProjectDirs::from("dev", "graffy", "graffy")
        .map(|dirs| dirs.data_dir().join("graffy.db"))
        .unwrap_or_else(|| PathBuf::from(".graffy/graffy.db"))
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

async fn run_graph(
    spec_arg: String,
    prompt: String,
    tui: bool,
    offline: bool,
    journal: Option<PathBuf>,
) -> anyhow::Result<()> {
    let spec_text = resolve_spec(&spec_arg).await?;
    let spec = GraphSpec::from_toml_str(&spec_text)?;
    let journal_path = journal.unwrap_or_else(|| {
        PathBuf::from("graffy-runs").join(format!("{}.journal", ulid_like_stamp()))
    });
    let invoker = build_invoker(offline);

    let outcome = if tui {
        let Some(outcome) = graffy_tui::run_live(
            spec.clone(),
            spec_text.clone(),
            prompt,
            journal_path.clone(),
            invoker,
        )
        .await?
        else {
            std::process::exit(1);
        };
        outcome
    } else {
        println!(
            "graph '{}' v{} — executing (every step journaled)",
            spec.graph.name, spec.graph.version
        );
        Executor::default()
            .run(
                &spec,
                &spec_text,
                RunInput {
                    prompt,
                    session_id: None,
                },
                &journal_path,
                invoker.as_ref(),
                &AutoApprove,
            )
            .await?
    };

    println!("\nrun     : {}", outcome.run_id);
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

    // Index the run + mirror the journal (best-effort: the journal file is
    // already durable; the store is the queryable index of it).
    match index_run(&spec, &spec_text, &outcome).await {
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
    Ok(())
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
    println!("store: {}", db_path().display());
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

/// Time-sortable journal file stamp (ULID via graffy-core's id types).
fn ulid_like_stamp() -> String {
    graffy_core::id::RunId::generate().to_string()
}
