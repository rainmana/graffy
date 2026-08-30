# NOTICE — third-party attributions

graffy is licensed **GPL-3.0-or-later** (see [LICENSE](LICENSE)). The following projects flow
into graffy as dependencies, ported concepts, or design references. All inbound licenses are
GPLv3-compatible (verified 2026-08-29 via crates.io / GitHub license metadata).

| Source | License | What graffy takes |
| --- | --- | --- |
| [Meta-Context Window (MCW) Framework](https://github.com/rainmana/mcw-framework) — W. Alec Akin | Apache-2.0 | Canonical vocabulary modeled as first-class types in `src/protos/mcw.proto`: Information Units and their five-stage lifecycle, the six failure modes, the five repair operations, H/R/D/M observables, and the evidence-level epistemic floor. |
| [rig / rig-core](https://github.com/0xPlaygrounds/rig) — 0xPlaygrounds | MIT | Provider layer dependency (Anthropic, OpenAI + compatibles, Ollama). |
| [rigs](https://github.com/M4n5ter/rigs) — M4n5ter | MIT | Multi-agent orchestration dependency for conversation nodes. |
| [graph-flow](https://crates.io/crates/graph-flow) | MIT | Design pattern reference for the typed-task execution loop with guarded cycles. No code vendored; graffy's executor is custom so MCW instrumentation stays first-class. |
| [Hermes Agent](https://github.com/NousResearch/hermes-agent) — Nous Research | MIT | Design reference: closed learning loop, capability versioning + rollback, skills as procedural memory. |
| [MemPalace](https://github.com/MemPalace/mempalace) | MIT | Design reference: layered wake-up memory (L0–L3), verbatim-first storage, temporal knowledge graph with validity windows. |
| Clear-Thought MCP server lineage (waldzellai et al.) | MIT | Catalog of reasoning patterns (sequential thinking, mental models, decision frameworks, OODA, collaborative reasoning…) reimplemented as graffy graph templates. |
| [LangConfig](https://github.com/LangConfig/langconfig) | MIT | Design reference: node-type taxonomy, portable workflow configs, live execution + replay UX, seeded template library. |

If code (not just concepts) is vendored from any of these projects, it keeps its upstream
copyright headers and gains an explicit entry here.
