# Contributing to graffy

Pre-alpha — expect churn. That said:

## Ground rules

- **Decisions live in ADRs.** Anything architectural goes through `docs/adr/` — read the index
  before proposing a redesign. New ADRs supersede old ones explicitly; nothing changes silently.
- **The invariant is non-negotiable.** No code path may send a user prompt or skill to a model
  outside a compiled, journaled graph. PRs that shortcut this get closed with a link to ADR-0005.
- **Claims need receipts here, too.** PRs that change behavior should cite evidence — a failing
  test, a journal excerpt, a benchmark bundle.

## Mechanics

- Toolchain: Rust stable (`rust-toolchain.toml` handles it).
- Gates: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` must pass (CI enforces all three).
- Protobufs compile via pure-Rust `protox` — do not introduce a system `protoc` dependency.
- Commit style: imperative subject, body explains *why*.
- **PRs wait for Codex.** Codex auto-reviews every PR. Green CI is necessary,
  not sufficient: wait for its comments/approval, reply to each as the agent
  (sign the reply so it reads as the agent, not @rainmana), then either fix and
  commit or explain why the finding is not applicable, and resolve the thread.
  Merge only once Codex has no open threads and has approved. See AGENT.md.

## License

By contributing you agree your work is licensed Apache-2.0. Third-party inflows must be
compatible with Apache-2.0 (no GPL/AGPL/LGPL-only code) and recorded in `NOTICE.md`; see
[ADR-0013](docs/adr/0013-apache-2-relicensing.md).
