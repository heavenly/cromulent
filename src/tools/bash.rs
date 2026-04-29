use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::protocol::events::ServerEvent;
use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;
use crate::transport::writer::OutputItem;

/// Executes a shell command using tokio::process::Command.
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

        let _timeout_secs = arguments.get("timeout").and_then(|v| v.as_i64());

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(cmd_str)
            .current_dir(&ctx.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ToolError::Other(format!("Failed to spawn command: {e}")))?;

        let stdout_handle = child.stdout.take().unwrap();
        let stderr_handle = child.stderr.take().unwrap();

        let mut stdout_buf = String::new();
        let mut stderr_buf = String::new();
        let mut stdout_reader = tokio::io::BufReader::new(stdout_handle);
        let mut stderr_reader = tokio::io::BufReader::new(stderr_handle);

        // Stream stdout/stderr line-by-line
        let mut line_buf = Vec::new();
        let mut partial_stdout = String::new();
        let mut partial_stderr = String::new();

        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    // Kill the child process
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ToolError::Cancelled);
                }

                stdout_result = read_line(&mut stdout_reader, &mut line_buf) => {
                    match stdout_result {
                        Ok(Some(line)) => {
                            partial_stdout.push_str(&line);
                            partial_stdout.push('\n');
                            stdout_buf.push_str(&line);
                            stdout_buf.push('\n');

                            // Emit incremental output
                            let ev = ServerEvent::BashOutput {
                                stdout: line + "\n",
                                stderr: String::new(),
                            };
                            let _ = ctx.event_tx.send(OutputItem::Event(serde_json::to_value(ev).expect("BashOutput serialization failed")));
                        }
                        Ok(None) => {
                            // stdout done, break to check stderr
                            break;
                        }
                        Err(e) => {
                            return Err(ToolError::Other(format!("stdout read error: {e}")));
                        }
                    }
                }
            }
        }

        // Read remaining stderr
        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ToolError::Cancelled);
                }

                stderr_result = read_line(&mut stderr_reader, &mut line_buf) => {
                    match stderr_result {
                        Ok(Some(line)) => {
                            partial_stderr.push_str(&line);
                            partial_stderr.push('\n');
                            stderr_buf.push_str(&line);
                            stderr_buf.push('\n');

                            let ev = ServerEvent::BashOutput {
                                stdout: String::new(),
                                stderr: line + "\n",
                            };
                            let _ = ctx.event_tx.send(OutputItem::Event(serde_json::to_value(ev).expect("BashOutput serialization failed")));
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return Err(ToolError::Other(format!("stderr read error: {e}")));
                        }
                    }
                }
            }
        }

        // Wait for process to exit
        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::Other(format!("Failed to wait for command: {e}")))?;
        let exit_code = status.code().unwrap_or(-1);

        // Emit bash_done
        let ev = ServerEvent::BashDone { exit_code };
        let _ = ctx.event_tx.send(OutputItem::Event(
            serde_json::to_value(ev).expect("BashDone serialization failed"),
        ));

        let stdout_trimmed = stdout_buf.trim().to_string();
        let stderr_trimmed = stderr_buf.trim().to_string();

        let mut text = String::new();
        if !stdout_trimmed.is_empty() {
            text.push_str(&format!("STDOUT:\n{stdout_trimmed}\n"));
        }
        if !stderr_trimmed.is_empty() {
            text.push_str(&format!("STDERR:\n{stderr_trimmed}\n"));
        }
        if stdout_trimmed.is_empty() && stderr_trimmed.is_empty() {
            text.push_str("(no output)\n");
        }
        text.push_str(&format!("Exit code: {exit_code}"));

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text }],
            is_error: exit_code != 0,
            metadata: Some(serde_json::json!({
                "exitCode": exit_code,
                "stdout": stdout_trimmed,
                "stderr": stderr_trimmed,
            })),
        })
    }
}

/// Read one line from a BufReader into the provided buffer.
/// Returns `Ok(Some(line))` if a line was read, `Ok(None)` on EOF.
async fn read_line<R: tokio::io::AsyncBufReadExt + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    buf.clear();
    let n = reader.read_until(b'\n', buf).await?;
    if n == 0 {
        return Ok(None); // EOF
    }
    // Remove trailing newline
    if buf.last() == Some(&b'\n') {
        buf.pop();
        // Also handle \r\n
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    let line = String::from_utf8_lossy(buf).to_string();
    Ok(Some(line))
}
