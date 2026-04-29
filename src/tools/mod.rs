pub mod ask_user;
pub mod bash;
pub mod find;
pub mod grep;
pub mod hashline;
pub mod read;
pub mod registry;
pub mod write;

pub use ask_user::AskUserTool;
pub use bash::BashTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use hashline::edit::HashlineEditTool;
pub use read::ReadTool;
pub use write::WriteTool;

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
