use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::process::bash_runner::BashRunner;
use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Executes a shell command using the shared BashRunner.
/// Output is streamed via ServerEvent::BashOutput and ServerEvent::BashDone.
/// The command is cancellable — killing the child process if cancelled.
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".to_string(),
            description: "Execute a shell command. The command runs in a subprocess; stdout and stderr are streamed incrementally. Use for running scripts, builds, git operations, or quick file inspection. Prefer read/find/grep for non-shell file operations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: no timeout)"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolContext,
        arguments: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let cmd_str = arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("Missing required 'command' argument".into())
            })?;

        let timeout_secs = arguments.get("timeout").and_then(|v| v.as_u64());

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let runner = BashRunner::new(ctx.cwd.clone());
        let timeout = timeout_secs.map(Duration::from_secs);

        let result = runner
            .execute_for_tool(cmd_str, &ctx.event_tx, cancel, timeout)
            .await;

        if result.cancelled {
            return Err(ToolError::Cancelled);
        }

        let stdout_trimmed = result.stdout.trim().to_string();
        let stderr_trimmed = result.stderr.trim().to_string();
        let exit_code = result.exit_code;

        let mut text = String::new();
        if result.timed_out {
            text.push_str(&format!(
                "Command timed out after {}s.\n",
                timeout_secs.unwrap_or(0)
            ));
        }
        if !stdout_trimmed.is_empty() {
            text.push_str(&format!("STDOUT:\n{stdout_trimmed}\n"));
        }
        if !stderr_trimmed.is_empty() {
            text.push_str(&format!("STDERR:\n{stderr_trimmed}\n"));
        }
        if stdout_trimmed.is_empty() && stderr_trimmed.is_empty() && !result.timed_out {
            text.push_str("(no output)\n");
        }
        text.push_str(&format!("Exit code: {exit_code}"));

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text }],
            is_error: exit_code != 0 || result.timed_out,
            metadata: Some(serde_json::json!({
                "exitCode": exit_code,
                "stdout": stdout_trimmed,
                "stderr": stderr_trimmed,
                "timedOut": result.timed_out,
                "cancelled": result.cancelled,
            })),
        })
    }
}
