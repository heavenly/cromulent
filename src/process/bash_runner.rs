use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::transport::writer::OutputItem;

/// Executes raw bash commands (UI-initiated, not agent tools).
#[derive(Clone)]
pub struct BashRunner {
    cwd: PathBuf,
}

impl BashRunner {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    /// Update the working directory.
    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
    }

    /// Execute a command, streaming output via the event channel.
    /// Delegates to `execute_with_cancel` with a no-op cancellation token.
    pub async fn execute(
        &self,
        command: &str,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
    ) -> std::io::Result<i32> {
        self.execute_with_cancel(command, output_tx, CancellationToken::new())
            .await
    }

    /// Execute a command with cancellation support.
    /// When the token is cancelled, the child process is killed and an error is returned.
    pub async fn execute_with_cancel(
        &self,
        command: &str,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
        cancel: CancellationToken,
    ) -> std::io::Result<i32> {
        use crate::app::output::emit_event;
        use crate::protocol::events::ServerEvent;

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Read stdout in background with cancellation
        let tx_stdout = output_tx.clone();
        let cancel_stdout = cancel.clone();
        let stdout_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    result = reader.next_line() => {
                        match result {
                            Ok(Some(line)) if !line.is_empty() => {
                                emit_event(
                                    &tx_stdout,
                                    ServerEvent::BashOutput {
                                        stdout: line + "\n",
                                        stderr: String::new(),
                                    },
                                );
                            }
                            _ => break,
                        }
                    }
                    _ = cancel_stdout.cancelled() => break,
                }
            }
        });

        // Read stderr in background with cancellation
        let tx_stderr = output_tx.clone();
        let cancel_stderr = cancel.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            loop {
                tokio::select! {
                    result = reader.next_line() => {
                        match result {
                            Ok(Some(line)) if !line.is_empty() => {
                                emit_event(
                                    &tx_stderr,
                                    ServerEvent::BashOutput {
                                        stdout: String::new(),
                                        stderr: line + "\n",
                                    },
                                );
                            }
                            _ => break,
                        }
                    }
                    _ = cancel_stderr.cancelled() => break,
                }
            }
        });

        // Race between process completion and cancellation
        let result = tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) => {
                        let exit_code = s.code().unwrap_or(-1);
                        emit_event(output_tx, ServerEvent::BashDone { exit_code });
                        Ok(exit_code)
                    }
                    Err(e) => {
                        emit_event(output_tx, ServerEvent::BashDone { exit_code: -1 });
                        Err(e)
                    }
                }
            }
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                // Wait briefly for the kill to take effect
                let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                emit_event(output_tx, ServerEvent::BashDone { exit_code: -1 });
                Err(std::io::Error::other(
                    "Command cancelled",
                ))
            }
        };

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        result
    }
}
