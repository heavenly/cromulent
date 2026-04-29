use std::collections::HashMap;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ToolContext, ToolDefinition, ToolResult};
use crate::tools::ask_user::AskUserTool;
use crate::tools::bash::BashTool;
use crate::tools::find::FindTool;
use crate::tools::grep::GrepTool;
use crate::tools::hashline::edit::HashlineEditTool;
use crate::tools::read::ReadTool;
use crate::tools::write::WriteTool;
use crate::tools::ToolError;

/// A single executable tool known to the agent.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the tool's definition (name, description, input schema) for LLM provider handshake.
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given context, arguments, and cancellation token.
    async fn execute(
        &self,
        ctx: ToolContext,
        arguments: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolError>;
}

/// Thread-safe handle that owns all registered tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Build the default registry with all required v1 tools.
    pub fn default() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(ReadTool));
        reg.register(Box::new(WriteTool));
        reg.register(Box::new(HashlineEditTool));
        reg.register(Box::new(GrepTool));
        reg.register(Box::new(FindTool));
        reg.register(Box::new(BashTool));
        reg.register(Box::new(AskUserTool));
        reg
    }

    /// Register a tool by name (from its definition).
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.definition().name.clone();
        self.tools.insert(name, tool);
    }

    /// Execute a named tool.
    pub async fn execute(
        &self,
        name: &str,
        ctx: ToolContext,
        arguments: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolError> {
        match self.tools.get(name) {
            Some(tool) => tool.execute(ctx, arguments, cancel).await,
            None => Err(ToolError::NotFound(format!("Unknown tool: {name}"))),
        }
    }

    /// Returns all registered tool definitions.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        f.debug_struct("ToolRegistry")
            .field("tools", &names)
            .finish()
    }
}
