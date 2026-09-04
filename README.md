<div align="center">

# graffy

**A terminal-native runtime for inspectable, durable agent graphs.**

Every prompt, imported skill, tool call, and approval runs through a compiled TOML graph and produces an append-only run journal.

[![CI](https://github.com/rainmana/graffy/actions/workflows/ci.yml/badge.svg)](https://github.com/rainmana/graffy/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

*Pre-alpha (source-tree version `0.1.0-alpha.6`; tagged releases may trail it). The graph runtime, journals, TUI, providers, MCP facades, graphification, persistence, retries, and metrics work; several larger MCW, memory, agent, and evaluation features remain in development.*

</div>

## Why graffy?

Most agent harnesses treat the prompt as the program and leave orchestration hidden inside application code. graffy makes the orchestration a first-class artifact:

- **Graphs are data.** Nodes, edges, guards, budgets, evidence policy, and model-routing tiers live in readable TOML files that can be diffed, versioned, exported, and imported.
- **Runs are inspectable records.** Every run writes length-delimited protobuf events to an append-only `.journal` file and mirrors completed runs into embedded libSQL. `replay` folds those recorded events for inspection; it does not re-execute the graph.
- **Loops are bounded.** The compiler rejects unguarded cycles; the executor enforces per-node visit caps and graph-level token, time, and USD ceilings.
- **Quality gates route visibly.** `verify` nodes return `PASS` or `REVISE`; revisions emit failure signals and move subsequent work up the graph's routing ladder.
- **Tools are graph nodes.** MCP tools are discovered over stdio and wrapped in generated facade graphs. Destructive or unannotated tools receive a human approval gate by default.
- **Human decisions are journaled.** Approval, rejection, and approve-with-edit outcomes include the decision source. EOF and abandoned approvals reject rather than silently release work.
- **MCW concepts are typed.** Information Units, evidence artifacts, the six coordination failure modes, five repair operations, and H/R/D/M samples are represented in the protobuf schema. The current runtime records IUs and evidence, detects judge-labelled failures and visit-cap exhaustion, and journals retry repair episodes.

## Quick start

### Install a release binary

Tagged releases contain one `graffy` binary plus the license, notice, and README:

- Linux x86_64, static musl
- Linux aarch64, static musl
- macOS Apple Silicon (`aarch64`)

Download the archive for your platform from [GitHub Releases](https://github.com/rainmana/graffy/releases), extract it, and place `graffy` on your `PATH`.

macOS releases are currently unsigned and not notarized. Intel macOS and Windows binaries are not currently produced.

### Build or install from source

Rust stable is the only build prerequisite. Protobuf compilation uses the pure-Rust `protox` toolchain, so a system `protoc` installation is not required.

```console
$ git clone https://github.com/rainmana/graffy
$ cd graffy
$ cargo install --path .
```

Or build the whole workspace without installing:

```console
$ cargo build --workspace
```

### Initialize and run without a model

```console
$ graffy init
$ graffy graph list
$ graffy run graffy.builtin.conversation --prompt "hello" --offline
```

`--offline` uses a deterministic echo invoker. With this tool-free built-in it exercises the real compiler, executor, journal, registry, and optional TUI without making a model or network call; it is a demo/test mode, not a useful model response. It disables model calls, not MCP execution: a graph containing `tool.invoke` nodes still connects to and invokes its registered servers.

The run prints its journal path. Inspect it in plain text or in the TUI:

```console
$ graffy replay ~/.graffy/runs/<run-id>.journal --events
$ graffy replay ~/.graffy/runs/<run-id>.journal --tui
```

Running `graffy` with no command opens the journal-browser TUI. `graffy tui` does the same explicitly.

## Live model configuration

graffy binds graph-level capability tiers to provider/model pairs through environment variables. It does not hardcode model names.

```text
GRAFFY_MODEL_FAST=provider:model
GRAFFY_MODEL_BALANCED=provider:model
GRAFFY_MODEL_FRONTIER=provider:model
```

Supported providers:

| Provider prefix | Required configuration |
| --- | --- |
| `anthropic` | `ANTHROPIC_API_KEY` |
| `openai` | `OPENAI_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |
| `venice` | `VENICE_API_KEY` |
| `ollama` | No key; optional `OLLAMA_API_BASE_URL` |

A graph only requires the tiers it actually reaches, but the built-ins can use `fast`, `balanced`, and `frontier` as work escalates. Bind all three for predictable live runs:

```console
$ export GRAFFY_MODEL_FAST=ollama:<fast-model>
$ export GRAFFY_MODEL_BALANCED=ollama:<balanced-model>
$ export GRAFFY_MODEL_FRONTIER=ollama:<frontier-model>
$ graffy doctor
$ graffy run graffy.builtin.conversation --prompt "Explain the tradeoffs in this design" --tui
```

Replace the placeholders with model identifiers available from your provider. `graffy doctor` reports tier bindings and whether credential variables are present without printing their values.

> [!NOTE]
> Provider token counts are recorded when the provider returns them. USD pricing tables are not implemented yet, so live model calls deliberately record `cost_usd = 0.0`; USD budget and cost output should not yet be treated as real spend accounting.

## Built-in graphs

Built-ins are ordinary TOML specs. They are automatically seeded into the registry and can be exported, edited, and re-imported.

| Graph ID | Pipeline | Purpose |
| --- | --- | --- |
| `graffy.builtin.conversation` | intake → ground → draft → verify → respond | Minimum governed conversation path with a guarded draft-revision loop |
| `graffy.builtin.conversation.gated` | conversation floor → human approval → respond | Prevents release until a human approves or edits the result |
| `graffy.builtin.reasoning.sequential-thinking` | plan → think → critique → synthesize | Structured reasoning with a guarded critique loop |
| `graffy.builtin.reasoning.decision-framework` | frame → evaluate → challenge → decide | Options, criteria, weighted evaluation, challenge, and recommendation |

Run a built-in by registry ID or run a TOML file directly:

```console
$ graffy run graffy.builtin.reasoning.sequential-thinking --prompt "Design a falsifiable experiment" --offline
$ graffy run graphs/reasoning.decision-framework.toml --prompt "Choose a journal format" --offline
```

## Command reference

Global option: `-v` enables debug logs and `-vv` enables trace logs. `RUST_LOG` can override the default tracing filter.

| Command | What it does | Important options |
| --- | --- | --- |
| `graffy` / `graffy tui` | Opens the journal-browser TUI | — |
| `graffy init` | Creates the graffy home, run directory, database, and built-in graph records | — |
| `graffy doctor` | Shows version, data paths, store counts, tier bindings, and credential presence | — |
| `graffy run <SPEC> --prompt <TEXT>` | Runs a TOML path or registered graph ID | `--offline`, `--tui`, `--journal <PATH>`, `--retry <N|auto>` |
| `graffy replay <JOURNAL>` | Folds a journal into a run summary | `--events`, `--tui` |
| `graffy runs` | Lists indexed runs | `--limit <N>`; default `10` |
| `graffy metrics` | Folds journals into per-run and aggregate MCW/runtime metrics | `--dir <PATH>`, `--json` |
| `graffy graph list` | Lists registered graphs | — |
| `graffy graph export <ID>` | Writes a registered graph as shareable TOML | `--out <PATH>`; default `<id>.toml` |
| `graffy graph import <PATH>` | Parses, compiles, validates, hashes, and registers a TOML graph | — |
| `graffy graphify <PATH>` | Converts a skill or prompt file into a governed graph | `--name <NAME>`, `--prompt`, `--mode auto|guided|collaborative` |
| `graffy mcp add <NAME> --stdio <COMMAND>` | Discovers a stdio MCP server and optionally generates one facade graph per tool | `--role`, `--evidence-level`, `--knowledge`, `--skip-interview`, `--no-facades` |
| `graffy mcp list` | Lists registered MCP transport bindings and metadata | — |

Run `graffy <command> --help` for the exact syntax supported by the installed version.

### Retries with repair feedback

```console
$ graffy run graffy.builtin.conversation --prompt "..." --retry 2
$ graffy run graffy.builtin.conversation --prompt "..." --retry auto
```

`--retry N` permits `N` extra attempts. `--retry auto` permits up to three extra attempts. Failed attempts only retry when graffy can harvest a failure signal; the judge's critique is injected into the next attempt as a correction IU, and all attempts share one session ID while keeping separate journal files.

Retries are currently unavailable with `--tui`. Run in plain mode, then replay any attempt in the TUI.

### Metrics

```console
$ graffy metrics
$ graffy metrics --json > metrics.json
$ graffy metrics --dir /path/to/journals
```

The metrics fold reports recorded run outcomes, tokens, model/tool calls, IUs, evidence levels, failure modes, repairs, visit-cap hits, escalation outcomes, and H/R/D/M sample counts. It does not estimate missing values.

## Graphify skills and prompts

Graphification turns text into a five-node governed graph:

```text
intake → ground → apply → verify → respond
                   ▲         │
                   └─ revise ┘
```

A Markdown file with YAML-style frontmatter or a `#` heading is treated as a skill. Use `--prompt` to force raw-prompt handling.

```console
$ graffy graphify ./SKILL.md
$ graffy graphify ./prompt.txt --prompt --name daily-standup
$ graffy graphify ./SKILL.md --mode guided
$ graffy graphify ./SKILL.md --mode collaborative
```

Modes:

- `auto` generates, validates, and registers the graph immediately.
- `guided` opens a review surface before registration; non-interactive use prints the TOML and asks for confirmation. EOF rejects.
- `collaborative` adds node-description editing, model-tier cycling, and system-knowledge editing through `$EDITOR`. It requires an interactive terminal.

Generated artifacts record the actual involvement mode in their authorship metadata.

## MCP tools as graph facades

graffy currently supports MCP servers over **stdio**.

```console
$ graffy mcp add filesystem --stdio "npx -y @modelcontextprotocol/server-filesystem /path/to/share"
$ graffy mcp list
$ graffy graph list
```

During registration graffy:

1. starts the server and performs the MCP handshake;
2. discovers tools, annotations, and available no-required-argument prompts;
3. seeds each tool as `evidence` or `effector`;
4. records the logical server-to-transport binding in the local store;
5. generates and validates a facade graph unless `--no-facades` is set.

Facade shape:

```text
intake → prepare → [approve] → invoke → digest → respond
```

Read-only tools are treated as evidence tools. Destructive tools are effectors. Unannotated tools default conservatively to effectors unless `--role evidence` is explicitly supplied. Effector facades include the human approval node.

Useful registration options:

```console
$ graffy mcp add <name> --stdio "<command>" --role evidence
$ graffy mcp add <name> --stdio "<command>" --evidence-level L2
$ graffy mcp add <name> --stdio "<command>" --knowledge "How and when this server should be used"
$ graffy mcp add <name> --stdio "<command>" --skip-interview
$ graffy mcp add <name> --stdio "<command>" --no-facades
```

When the server provides no prompt-based usage knowledge and registration is interactive, graffy asks a three-question usage interview. The answers can refine the default role and become context for each generated facade's argument-preparation node.

Only complete MCP tool responses containing text or structured JSON are handled today. Streamable HTTP transport, elicitation/input-required responses, and MCP tasks are not yet implemented.

## TUI controls

### Run and replay views

| Key | Action |
| --- | --- |
| `Tab` | Switch between run overview and step inspector |
| `↑` / `↓` | Select a node |
| `j` / `k` or `PageDown` / `PageUp` | Scroll detail |
| `q` / `Esc` | Quit |

During an approval: `a` approves, `r` rejects, and `e` begins approve-with-edit. `Enter` submits an edit; `Esc` cancels editing. Quitting while approval is pending records a rejection. In the current runtime, an approval edit is preserved as the approval record's note; it does not rewrite the draft or create a correction IU.

### Graphify review

`a` or `Enter` accepts after a fresh compiler check; `r`, `q`, or `Esc` rejects; `n` renames; `↑`/`↓` selects nodes; `j`/`k` scrolls; `?` opens help. Collaborative mode additionally uses `d` to edit a node description, `t` to cycle a model tier, and `s` to edit system knowledge.

## Data and persistence

The default home is `~/.graffy`:

```text
~/.graffy/
├── graffy.db     # embedded libSQL registry and queryable indexes
└── runs/         # canonical append-only .journal files
```

Override it with either variable; `GRAFFY_DATA_DIR` has precedence and is retained for compatibility:

```console
$ export GRAFFY_HOME=/path/to/graffy-home
# or
$ export GRAFFY_DATA_DIR=/path/to/graffy-home
```

The database currently stores graph records, run/session indexes, mirrored journal events, and MCP server bindings. The journal file remains the canonical run record.

## Graph format and runtime

A graph spec contains metadata, policies, nodes, and directed edges:

```toml
[graph]
id = "example.reviewed-answer"
name = "Reviewed Answer"
version = "0.1.0"

[policy.evidence]
mode = "strict"
min_level = "L1"

[policy.budget]
max_tokens = 200000
max_usd = 1.0
max_seconds = 300

[policy.routing]
ladder = ["fast", "balanced", "frontier"]
on_quality_fail = "escalate"

[[node]]
id = "intake"
kind = "intake"

[[node]]
id = "draft"
kind = "model"
model_tier = "balanced"

[[edge]]
from = "intake"
to = "draft"
```

Implemented node kinds are `intake`, `research`, `model`, `verify`, `respond`, `approval`, and `tool.invoke`. Unknown kinds fail loudly at runtime. Back-edges must be guarded; guards currently support truthy facts, `==`, `!=`, and `&&`.

Normal executor runs can journal the graph ID/version, graffy version, graph-spec SHA-256, session ID, evidence policy, node transitions, IUs, evidence, model and tool calls, routing decisions, approvals, budget events, failure signals, repair actions, and final status. The journal stores the spec hash rather than the full spec, and model requests and tool arguments are hash-represented rather than preserved as plaintext. Failed or truncated execution can leave a partial journal.

## Current boundaries

The project is intentionally pre-alpha. The main implementation boundaries are:

- The generic `research` node is currently a structural no-op; it does not independently browse, query memory, or call MCP. Concrete MCP access works through generated `tool.invoke` facade graphs.
- The `intake` node records the complete prompt as one L1 goal IU; it does not yet perform the decomposition described by the built-in graph prose.
- Evidence policy and evidence levels are recorded and included in judge context, but the current `verify` node is model-judged rather than a complete deterministic claim-to-artifact validator.
- `policy.routing.on_quality_fail` is serialized but not currently consulted. A `REVISE` result tier-escalates its passing successor when that successor's base tier appears in the ladder; a different provider or model is only used if the environment bindings make it so.
- Broader MCW streaming detectors, durable learned lessons, automatic compaction graphs, and H/R/D/M raters are Phase 3 work. Current detection covers judge-labelled review failures and visit-cap exhaustion.
- The database is a graph/run/journal/MCP registry today. Episodic FTS memory, vector retrieval, temporal knowledge graphs, and layered recall are not implemented yet.
- Multi-persona `agent.conversation` execution and benchmark/evaluation runners are scaffolded, not active node kinds.
- A completed graph run can report `Failed`, `Cancelled`, or retry exhaustion while the CLI process still exits successfully; inspect the printed run status or journal rather than relying only on the shell exit code. Setup, parsing, provider, transport, and other command errors still return an error.
- Approval edits are attribution-bearing notes today; they do not rewrite the approved draft.
- Cost output remains zero until provider pricing tables land.
- MCP is stdio-only, and its stored argument tail is space-separated; complex shell quoting is not preserved as a shell would preserve it.
- Supplying an existing `--journal` path truncates that file before the new run; journals are append-only only after creation.
- The TUI may show overlapping text under some Windows Terminal/WSL resize and glyph-width combinations; see [Known Issues](docs/KNOWN-ISSUES.md).

See [the roadmap](docs/ROADMAP.md) and [open GitHub issues](https://github.com/rainmana/graffy/issues) for planned work.

## Development

Required gates (Rust stable plus `python3`, which the hermetic MCP fixture uses):

```console
$ cargo fmt --all --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo test --workspace
```

CI runs formatting and Clippy on Linux, then builds and tests the workspace on Linux and macOS. The MCP crate also contains an ignored live-server integration test:

```console
$ cargo test -p graffy-mcp -- --ignored
```

Project guidance:

- [AGENT.md](AGENT.md) — invariant, architecture seams, development discipline, and current project state
- [Architecture](docs/ARCHITECTURE.md) — design and crate map
- [ADRs](docs/adr/README.md) — architectural decisions and rationale
- [Roadmap](docs/ROADMAP.md) — phases and tabled design breadcrumbs
- [Known issues](docs/KNOWN-ISSUES.md) — observed field issues
- [Contributing](CONTRIBUTING.md) — contribution requirements
- [Changelog](CHANGELOG.md) — release history
- [NOTICE](NOTICE.md) — third-party lineage and attribution

## License

[GPL-3.0-or-later](LICENSE).

Third-party concepts and dependencies are credited in [NOTICE.md](NOTICE.md), including the Apache-2.0 [Meta-Context Window Framework](https://github.com/rainmana/mcw-framework) by W. Alec Akin and the MIT-licensed projects that informed parts of graffy's design.
