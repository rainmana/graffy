# ADR-0009: GPLv3 licensing and third-party inflow rules

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

graffy is GPL-3.0-or-later (chosen at repo creation). Inspiration and dependencies come from
Apache-2.0 and MIT projects; license compatibility was verified 2026-08-29 (crates.io + GitHub
metadata): MCW Framework Apache-2.0; rig-core, rigs, graph-flow, libsql, ratatui, Hermes,
MemPalace, LangConfig all MIT; prost/prost-build Apache-2.0; rmcp Apache-2.0; fastembed
Apache-2.0.

## Decision

- Apache-2.0 and MIT code and concepts may flow **into** graffy (one-way compatible with
  GPLv3). Every inflow — dependency, ported concept, or vendored code — is recorded in
  `NOTICE.md`; vendored code keeps upstream headers.
- graffy's own code carries `GPL-3.0-or-later` SPDX headers where headers are used.
- GPL-incompatible sources (or unlicensed snippets) do not enter the tree, full stop.
- Copyleft note for users: shipping modified graffy binaries triggers GPLv3 source obligations;
  TOML graph specs users author are their own data, not derivative works of graffy.

## Consequences

- Hermes/MemPalace/LangConfig/Clear-Thought patterns are usable as deeply as we like, with
  attribution. The MCW Framework's canonical vocabulary is embedded with attribution.
- Contributions are accepted under GPL-3.0-or-later (see CONTRIBUTING.md).
