# CLAUDE.md

Read **AGENT.md** first — it is the canonical agent briefing for this
project (invariant, architecture seams, discipline, owner's working style,
paid-for lessons). Everything there applies to you.

The five rules that prevent the most damage, restated:

1. Nothing executes as a raw prompt — every execution path goes through a
   compiled, journaled graph. Do not add shortcuts, not even for tests
   (tests use `OfflineEcho` through the real executor).
2. Gates before any push: `cargo fmt --all --check` &&
   `cargo clippy --workspace --all-targets -- -D warnings` &&
   `cargo test --workspace`.
3. Verify dependency APIs against the published crate source
   (static.crates.io tarballs), never memory. rig-core's lib is `rig_core`;
   rmcp's params are non-exhaustive builders; responses are enums.
4. Honest values only: no guessed prices, no hardcoded model names, no
   silent fallbacks. Loud, specific failures.
5. Decisions → `docs/adr/`; deferred ideas → ROADMAP "Tabled" with design
   breadcrumbs; observations → `docs/KNOWN-ISSUES.md`; license inflows
   (GPLv3-compatible only) → `NOTICE.md`.

Owner: W. Alec Akin (@rainmana). Teach as you work, credit his ideas, give
him exact PowerShell-safe commands (no `&&`) for anything touching tags,
workflows, or releases.
