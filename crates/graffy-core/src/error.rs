//! Core error types.

use thiserror::Error;

/// Errors reading or writing graph specs.
#[derive(Debug, Error)]
pub enum SpecError {
    #[error("failed to read graph spec: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid graph spec TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize graph spec: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Errors compiling a spec into an executable topology.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("duplicate node id '{0}'")]
    DuplicateNode(String),
    #[error("edge references unknown node '{0}'")]
    UnknownNode(String),
    #[error(
        "graph contains an unguarded cycle — back-edges must carry `when` guards (and budgets apply at runtime)"
    )]
    UnguardedCycle,
    #[error("graph has no entry node (every node has guarded or unguarded predecessors)")]
    NoEntryNode,
}

/// Errors from the append-only run journal.
#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal io: {0}")]
    Io(#[from] std::io::Error),
    #[error("journal frame decode: {0}")]
    Decode(#[from] graffy_proto::prost::DecodeError),
}

/// Errors from a model invoker (provider layer or offline stub).
#[derive(Debug, Error)]
pub enum ModelError {
    #[error("no model bound for tier '{tier}' — {hint}")]
    UnboundTier { tier: String, hint: String },
    #[error("provider error: {0}")]
    Provider(String),
}

/// Errors from a tool invoker (the MCP plane, or a mock in tests).
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("no tool invoker configured: {0}")]
    Unavailable(String),
    #[error("tool call failed: {0}")]
    Call(String),
}

/// Errors during graph execution.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("no behavior registered for node kind '{0}'")]
    UnknownNodeKind(String),
    #[error("guard expression '{expr}' is malformed: {reason}")]
    BadGuard { expr: String, reason: String },
    #[error("node '{0}' failed: {1}")]
    NodeFailed(String, String),
}
