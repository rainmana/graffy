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
}
