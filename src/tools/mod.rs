pub mod registry;
pub mod bash;
pub mod read;
pub mod write;
pub mod edit;
pub mod grep;
pub mod find;
pub mod ask_user;

use thiserror::Error;

/// Structured errors from tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Execution cancelled")]
    Cancelled,

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Tool not found: {0}")]
    NotFound(String),

    #[error("Edit failed: {0}")]
    EditFailed(String),

    #[error("Ask error: {0}")]
    AskError(String),

    #[error("{0}")]
    Other(String),
}
