pub mod registry;
pub mod bash;
pub mod read;
pub mod write;
pub mod edit;
pub mod grep;
pub mod find;
pub mod ask_user;

pub use read::ReadTool;
pub use write::WriteTool;
pub use edit::EditTool;
pub use grep::GrepTool;
pub use find::FindTool;
pub use bash::BashTool;
pub use ask_user::AskUserTool;

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
