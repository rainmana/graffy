//! MCP integration — Phase 2 (see docs/ROADMAP.md).
//!
//! Plan: an rmcp-backed client manager (stdio + streamable HTTP servers).
//! Every tool a connected MCP server exposes becomes an invocable graph node
//! (`kind = "tool"`, origin = MCP); results are recorded as evidence
//! artifacts. Later, graffy itself can serve MCP — exposing *graphs* as
//! tools to other clients.
//!
//! The rmcp dependency is version-pinned in the workspace manifest and wired
//! here when Phase 2 begins.
