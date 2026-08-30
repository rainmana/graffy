# ADR-0002: Terminal-native UI with Ratatui; macOS + Linux; no Tauri

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

Earlier drafts kept a future GUI (Tauri) on the table. The Phase 1 lock-down pivots completely
away from that: graffy is a pure terminal application.

## Decision

- The only interface is a Ratatui TUI (+ plain CLI subcommands) in the single binary.
- Target platforms: macOS and Linux. Distribution: GitHub Actions builds per-tag release
  binaries (x86_64 + aarch64 for both OSes).
- No Tauri, no web view, no GUI framework — removed from the roadmap entirely.
- The TUI must serve novices: plain-language explanations of node states ("double-checking
  claims against sources…"), progressive disclosure of graph detail.

## Consequences

- One rendering stack to master; deep investment in Ratatui (live graph view, inspector,
  dashboards, survey forms) instead of split UI effort.
- Anything that fundamentally needs pixels (rich diagrams) is expressed as terminal-friendly
  structure instead; journal bundles remain exportable for external tooling.
- Windows support, if demanded, is a future ADR (likely via WSL2 guidance first).
