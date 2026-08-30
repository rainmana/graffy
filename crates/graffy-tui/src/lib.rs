//! Terminal UI (ADR-0002) — ratatui-native, macOS + Linux.
//!
//! Phase 1 milestone M3:
//! * **Run view** — the graph rendered live: node states folding out of the
//!   journal stream, pulses along edges as payloads move (LangConfig's live
//!   canvas, reimagined for the terminal).
//! * **Step inspector** — enter any node during or after a run: inputs,
//!   outputs, Information Units, evidence artifacts, model + routing calls.
//! * **Novice mode** — plain-language line under every state change
//!   ("double-checking claims against sources…"), because graffy must be
//!   usable by people who have never heard of a prompt, a skill, or a graph.

/// Temporary entry point until the Phase 1 TUI lands (M3).
///
/// Async on purpose: the TUI runs inside the binary's tokio runtime so the
/// render loop stays non-blocking while the executor streams tokens and IUs.
pub async fn run_placeholder() -> anyhow::Result<()> {
    println!("graffy tui — the live graph view lands in Phase 1 (see docs/ROADMAP.md).");
    println!("try: graffy run graphs/conversation.default.toml   (spec parser + cycle-guard compiler are live)");
    Ok(())
}
