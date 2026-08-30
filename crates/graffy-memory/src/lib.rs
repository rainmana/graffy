//! Memory & persistence (ADR-0006) — embedded libSQL (Turso lineage).
//!
//! One database file carries:
//! * the **episodic log** — verbatim, never summarized at write time
//!   (MemPalace-inspired; summaries are separate, IU-preserving artifacts),
//! * **vector search** over embeddings via libSQL's native `F32_BLOB`
//!   columns and `vector_top_k` (embeddings local via fastembed in Phase 3,
//!   or cloud),
//! * the **temporal knowledge graph** — entity/relation triples with
//!   validity windows, so "what did we believe on Tuesday?" is answerable,
//! * **session persistence** and the run-journal mirror for query,
//! * the **graph registry** — installed durable graph objects + provenance.
//!
//! Phase 3 implements retrieval; Phase 1 (M4) ships the schema so nothing is
//! retrofitted.

pub use libsql;
