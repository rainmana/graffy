<div align="center">

# graffy

**A graph-native agentic harness. No prompt, no skill, no chat turn ever executes outside an inspectable, durable, shareable agent graph.**

[![CI](https://github.com/rainmana/graffy/actions/workflows/ci.yml/badge.svg)](https://github.com/rainmana/graffy/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

*Pre-alpha — Phase 0/1. The skeleton is real; the muscles are being attached. See [ROADMAP](docs/ROADMAP.md).*

</div>

## The bet

Most harnesses execute prompts and skills "raw": text goes in, tokens come out, and whatever
happened in between is a vibe. graffy takes the opposite bet:

- **Everything is a graph.** User prompts, imported skills, memory recall, summarization,
  peer review, even "plain chat" — each runs as a typed, budgeted, journaled graph. Graphs are
  durable objects you can version in git, share as files, inspect mid-flight, and replay after.
- **Claims need receipts.** Agents never execute on inference ("guessing") alone. Statements
  carry evidence artifacts with explicit epistemic levels (L0 definitional → L3 validated).
  Running a DnD campaign instead of a research paper? The receipts still exist — they just stay
  out of the interface (`policy.evidence.mode = "trace-only"`).
- **Coordination is measured, not assumed.** The [Meta-Context Window (MCW) Framework](https://github.com/rainmana/mcw-framework)
  is built into the type system: Information Units, the six coordination failure modes, the five
  repair operations, and H/R/D/M observables are first-class protobuf types in every run journal.
- **Compaction never eats meaning.** Context-window management is itself a graph with
  IU-preservation checks — never a raw "summarize this" call (Overcompression is a named,
  detectable failure mode here).
- **Quality failures route up.** When a review node rejects content, smart routing escalates to
  a stronger model, a different provider, or a local fine-tune — it doesn't silently bounce work
  back to the model that produced it.

## Quick taste

```console
$ cargo run -- run graphs/conversation.default.toml --prompt "hello" --offline --tui
```

…and watch the pipeline light up node by node — intake, ground, draft, verify, respond —
with a live journal feed, an MCW counter strip, and a plain-language line explaining each
step. `Tab` opens the step inspector (every IU, evidence artifact, model call, and routing
decision, straight from the journal). Afterwards:

```console
$ cargo run -- replay graffy-runs/<run>.journal --tui   # inspect any past run
$ cargo run -- run graphs/reasoning.sequential-thinking.toml --prompt "…" --offline
```

That TOML file *is* a durable graph object — read it, diff it, commit it, send it to a friend.

## Status

| Piece | State |
| --- | --- |
| Cargo workspace, ADRs, CI, release pipeline | ✅ this repo |
| MCW protobuf schema (`src/protos/mcw.proto`) | ✅ IUs, 6 failure modes, 5 repair ops, H/R/D/M |
| Run journal schema (`src/protos/journal.proto`) | ✅ append-only, replayable |
| TOML graph specs + cycle-guard compiler | ✅ parsing + validation |
| Executor: guarded cycles, budgets, escalation routing (M2) | ✅ tested |
| Providers via rig: Anthropic, OpenAI, Ollama, Venice, OpenRouter (M2) | ✅ tier bindings |
| Live Ratatui run view + step inspector + novice mode (M3) | ✅ `--tui` |
| Reasoning templates: sequential thinking, decision framework | ✅ `graphs/` |
| libSQL store: graph registry, run history, queryable journal mirror (M4) | ✅ `graffy.db` |
| Graph export/import as validated TOML (M5 core) | ✅ `graffy graph` |
| Interactive TUI approvals, provenance bundles | 🚧 Phase 1 wrap-up |
| MCP tools-as-nodes, skill → graph conversion | 🔜 Phase 2 |
| libSQL memory (vectors, temporal KG), MCW detectors live | 🔜 Phase 3 |
| Benchmarks, surveys, shareable eval bundles | 🔜 Phase 4 |

## New to all this?

You don't need to know what an "agentic graph" is to use graffy. Short version: instead of
asking one AI to answer in one shot, graffy walks your request through a small pipeline —
understand → research → draft → double-check → respond — and shows you every step. That's why
answers take a bit longer, and why you can actually trust (and audit) what comes back.

## Docs

- [ARCHITECTURE](docs/ARCHITECTURE.md) — crates, data flow, invariants
- [ROADMAP](docs/ROADMAP.md) — phases and milestones
- [ADRs](docs/adr/README.md) — every locked decision, with reasoning
- [NOTICE](NOTICE.md) — third-party attributions

## Building

Rust stable (via [rustup](https://rustup.rs)) is the only prerequisite — protobufs compile with
a pure-Rust toolchain (no `protoc`), and the database is embedded (no server).

```console
$ cargo build --workspace
$ cargo test  --workspace
```

## License

[GPL-3.0-or-later](LICENSE). Third-party concepts and dependencies are credited in
[NOTICE.md](NOTICE.md) — notably the Apache-2.0 [MCW Framework](https://github.com/rainmana/mcw-framework)
and the MIT-licensed [rig](https://github.com/0xPlaygrounds/rig), rigs, Hermes, MemPalace,
Clear-Thought, and LangConfig projects that inspired parts of this design.
