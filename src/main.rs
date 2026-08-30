//! graffy — graph-native agentic harness.
//!
//! Nothing — no prompt, no skill, no chat turn — executes outside an
//! inspectable, durable, shareable agent graph. See docs/ARCHITECTURE.md.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use graffy_core::exec::{AutoApprove, Executor, ModelInvoker, OfflineEcho, RunInput};
use graffy_core::journal::{JournalReader, summarize, wire};
use graffy_core::spec::GraphSpec;
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
        /// Path to a TOML graph spec (see graphs/).
        spec: PathBuf,
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
    /// List graphs known to this installation.
    List,
    /// Export a graph (TOML spec + provenance) for sharing.
    Export { id: String },
    /// Import a shared graph.
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
        Command::Graph { command } => {
            match command {
                GraphCommand::List => {
                    println!("built-ins shipped with this binary:");
                    for (id, _) in graffy_graphs_builtins() {
                        println!("  {id}");
                    }
                    println!(
                        "\nthe installed-graph registry lands with the Phase 1 libSQL store (M4)."
                    );
                }
                GraphCommand::Export { id } => {
                    println!(
                        "export of '{id}' lands in Phase 1 (M5): TOML spec + provenance + journal excerpt."
                    );
                }
                GraphCommand::Import { path } => {
                    println!("import of '{}' lands in Phase 1 (M5).", path.display());
                }
            }
            Ok(())
        }
        Command::Doctor => doctor(),
    }
}

fn graffy_graphs_builtins() -> [(&'static str, &'static str); 3] {
    graffy_graphs::builtin_specs()
}

fn build_invoker(offline: bool) -> anyhow::Result<Arc<dyn ModelInvoker>> {
    if offline {
        println!("mode: OFFLINE ECHO — deterministic, no real model is consulted\n");
        return Ok(Arc::new(OfflineEcho));
    }
    match RigInvoker::from_env() {
        Ok(invoker) => {
            for (tier, target) in invoker.bound_tiers() {
                println!("tier {tier:<10} -> {target}");
            }
            println!();
            Ok(Arc::new(invoker))
        }
        Err(err) => {
            eprintln!("cannot start a live run: {err}");
            eprintln!("hint: `graffy run --offline …` exercises the engine without a model.");
            std::process::exit(2);
        }
    }
}

async fn run_graph(
    spec_path: PathBuf,
    prompt: String,
    tui: bool,
    offline: bool,
    journal: Option<PathBuf>,
) -> anyhow::Result<()> {
    let spec_text = std::fs::read_to_string(&spec_path)?;
    let spec = GraphSpec::from_toml_str(&spec_text)?;
    let journal_path = journal.unwrap_or_else(|| {
        PathBuf::from("graffy-runs").join(format!("{}.journal", ulid_like_stamp()))
    });
    let invoker = build_invoker(offline)?;

    let outcome = if tui {
        let Some(outcome) =
            graffy_tui::run_live(spec, spec_text, prompt, journal_path.clone(), invoker).await?
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
    if let Some(text) = outcome.final_text {
        println!("\n================ verified response ================\n{text}");
    }
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
            println!("  #{:<4} {}", frame.seq, event_name(frame));
        }
    }
    println!("\ntip: graffy replay {} --tui", journal.display());
    Ok(())
}

fn event_name(frame: &wire::RunEvent) -> &'static str {
    use graffy_core::journal::wire::run_event::Event;
    match &frame.event {
        Some(Event::RunStarted(_)) => "run_started",
        Some(Event::NodeTransition(_)) => "node_transition",
        Some(Event::ModelCall(_)) => "model_call",
        Some(Event::ToolCall(_)) => "tool_call",
        Some(Event::RoutingDecision(_)) => "routing_decision",
        Some(Event::Approval(_)) => "approval",
        Some(Event::Budget(_)) => "budget",
        Some(Event::RunFinished(_)) => "run_finished",
        Some(Event::IuRecorded(_)) => "iu_recorded",
        Some(Event::FailureRaised(_)) => "failure_raised",
        Some(Event::RepairExecuted(_)) => "repair_executed",
        Some(Event::HrdmSampled(_)) => "hrdm_sampled",
        Some(Event::EvidenceRecorded(_)) => "evidence_recorded",
        Some(Event::McwSnapshot(_)) => "mcw_snapshot",
        None => "(empty)",
    }
}

fn doctor() -> anyhow::Result<()> {
    println!("graffy {}", env!("CARGO_PKG_VERSION"));
    println!(
        "proto packages: {}",
        graffy_proto::PROTO_PACKAGES.join(", ")
    );
    if let Some(dirs) = directories::ProjectDirs::from("dev", "graffy", "graffy") {
        println!("config dir: {}", dirs.config_dir().display());
        println!("data dir  : {}", dirs.data_dir().display());
    }
    println!("\nbuilt-in graphs: {}", graffy_graphs_builtins().len());
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
