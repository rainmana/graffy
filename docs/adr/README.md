# Architecture Decision Records

Locked decisions, in order. An ADR is superseded only by a newer ADR that names it.

| # | Decision | Status |
| --- | --- | --- |
| [0001](0001-rust-single-process.md) | Rust workspace; single-process, single-binary runtime | Accepted |
| [0002](0002-terminal-native-ratatui.md) | Terminal-native UI (Ratatui); macOS + Linux; GitHub Actions distribution; no Tauri/GUI | Accepted |
| [0003](0003-protobuf-runtime-toml-specs.md) | Protobuf (prost/protox) for runtime + journal; TOML for graph specs | Accepted |
| [0004](0004-mcw-first-class-types.md) | MCW Framework modeled as first-class protobuf types | Accepted |
| [0005](0005-ecosystem-rig-rigs-graphflow.md) | rig-core providers; rigs conversation nodes; custom graph-flow-patterned executor; no-raw-execution invariant | Accepted |
| [0006](0006-libsql-storage.md) | Embedded libSQL (with vectors) for all local persistence | Accepted |
| [0007](0007-graph-sharing-durability.md) | Durable sharing: TOML specs in git + hash-pinned protobuf journal bundles | Accepted |
| [0008](0008-evidence-policy.md) | Evidence-backed execution and the epistemic floor | Accepted |
| [0009](0009-licensing-attribution.md) | GPLv3 project licensing and third-party inflow rules | Accepted |
