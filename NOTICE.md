# NOTICE — third-party attributions

graffy is licensed **Apache-2.0** (see [LICENSE](LICENSE); relicensed from GPL-3.0-or-later on
2026-11-14, [ADR-0013](docs/adr/0013-apache-2-relicensing.md)). The following projects flow
into graffy as dependencies, ported concepts, or design references. The sources below are
Apache-2.0 or MIT (verified 2026-08-29 via crates.io / GitHub license metadata). The full
dependency tree was re-checked on 2026-11-16 from `cargo metadata`: 445 third-party packages,
none GPL/AGPL-only; one, `option-ext`, is MPL-2.0 (file-level copyleft, used unmodified as a
dependency); every other package offers a permissive license.

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
| [reqwest](https://github.com/seanmonstar/reqwest) | MIT OR Apache-2.0 | Direct HTTP client for the decision-model router (`graffy-providers`); already in the tree through rig-core at the same version. |
| TypeSafe AI "System One" API (Jev) and [Strands Decider](https://github.com/strands-labs/strands-decider) (Apache-2.0) | — | Wire shape only (`POST /v1/systemone`): graffy's adapter is written against their published request/response schemas. No code is vendored and neither is bundled; the user points graffy at a service they run or subscribe to. |

If code (not just concepts) is vendored from any of these projects, it keeps its upstream
copyright headers and gains an explicit entry here.
