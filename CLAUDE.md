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
   (Apache-2.0-compatible only) → `NOTICE.md`.
6. **PRs wait for Codex auto-review.** Green CI is not "done". Reply to every
   Codex comment as the agent (sign it — it is not @rainmana), fix and commit or
   justify why the finding does not apply, resolve each thread, and merge only
   once Codex is clear. Work in the AGENT.md "What to work on next" order.

Owner: W. Alec Akin (@rainmana). Teach as you work, credit his ideas, give
him exact PowerShell-safe commands (no `&&`) for anything touching tags,
workflows, or releases.
