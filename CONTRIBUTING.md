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

## License

By contributing you agree your work is licensed GPL-3.0-or-later. Third-party inflows must be
GPLv3-compatible and recorded in `NOTICE.md`.
