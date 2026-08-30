# ADR-0006: Embedded libSQL (with vector search) for all local persistence

- Status: **Accepted**
- Date: 2026-08-29
- Deciders: W. Alec Akin (@rainmana)

## Context

The Phase 1 lock-down explicitly commits to the `libsql` crate with vector support for local
embedded memory and session persistence. Verified: libsql 0.9.x (MIT), native vector search
(`F32_BLOB` columns, `vector_top_k`) — no extension loading, no server.

## Decision

One embedded libSQL database file (default under the platform data dir) carries:

- the verbatim episodic log (+ FTS) — never summarized at write time,
- vector search over embeddings (fastembed locally in Phase 3, or cloud embedding APIs),
- the temporal knowledge graph (triples with validity windows),
- session persistence and the queryable mirror of run journals,
- the graph registry (installed durable graphs + provenance + spec hashes).

Turso sync/cloud is a possible later opt-in (same crate), never a requirement.

## Consequences

- Zero-infrastructure installs; backup = copy one file (plus journal files).
- Schema is versioned with embedded migrations from v1 (Phase 1 M4) so later phases never
  retrofit storage.
- MemPalace-style layered wake-up (identity → essentials → scoped recall → deep search) is a
  query discipline over this store, implemented in Phase 3.
