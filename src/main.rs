//! graffy — graph-native agentic harness.
//!
//! Nothing — no prompt, no skill, no chat turn — executes outside an
//! inspectable, durable, shareable agent graph. See docs/ARCHITECTURE.md.

use clap::{Parser, Subcommand};

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
    /// Launch the terminal UI (default when no subcommand is given).
    Tui,
    /// Execute a TOML graph spec.
    Run {
        /// Path to a TOML graph spec (see graphs/conversation.default.toml).
        spec: std::path::PathBuf,
        /// User prompt handed to the graph's intake node.
        #[arg(long)]
        prompt: Option<String>,
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
    Import { path: std::path::PathBuf },
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
        Command::Tui => graffy_tui::run_placeholder().await,
        Command::Run { spec, prompt } => {
            tracing::info!(spec = %spec.display(), ?prompt, "executor lands in Phase 1 (M2)");
            let parsed = graffy_core::spec::GraphSpec::from_toml_path(&spec)?;
            let compiled = graffy_core::graph::CompiledGraph::compile(&parsed)?;
            println!(
                "parsed + compiled graph '{}' v{} — {} nodes / {} edges (cycle guards verified)",
                parsed.graph.name,
                parsed.graph.version,
                compiled.topology.node_count(),
                compiled.topology.edge_count(),
            );
            println!("execution itself arrives with the Phase 1 executor — see docs/ROADMAP.md");
            Ok(())
        }
        Command::Graph { command } => {
            match command {
                GraphCommand::List => {
                    println!("graph registry lands with the Phase 1 libSQL store (M4).");
                }
                GraphCommand::Export { id } => {
                    println!("export of '{id}' lands in Phase 1 (M5): TOML spec + provenance + journal excerpt.");
                }
                GraphCommand::Import { path } => {
                    println!("import of '{}' lands in Phase 1 (M5).", path.display());
                }
            }
            Ok(())
        }
        Command::Doctor => {
            println!("graffy {}", env!("CARGO_PKG_VERSION"));
            println!("proto packages: {}", graffy_proto::PROTO_PACKAGES.join(", "));
            if let Some(dirs) = directories::ProjectDirs::from("dev", "graffy", "graffy") {
                println!("config dir: {}", dirs.config_dir().display());
                println!("data dir:   {}", dirs.data_dir().display());
            }
            Ok(())
        }
    }
}
