use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::output::emit_event;
use crate::protocol::events::ServerEvent;
use crate::transport::writer::OutputItem;

/// Result of a completed bash execution.
pub struct BashResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub cancelled: bool,
    pub timed_out: bool,
}

/// Executes bash commands — used by both the raw-bash handler and the agent tool.
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

    /// Execute a command and stream output. Used by the raw `bash` command handler.
    /// Returns the exit code on success, or an IO error.
    pub async fn execute_with_cancel(
        &self,
        command: &str,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
        cancel: CancellationToken,
    ) -> io::Result<i32> {
        let result = self.run_command(command, output_tx, cancel, None).await;
        result.map(|r| r.exit_code)
    }

    /// Execute a command for the agent tool — captures full output, supports timeout.
    /// Returns the full BashResult with accumulated stdout/stderr.
    pub async fn execute_for_tool(
        &self,
        command: &str,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
        cancel: CancellationToken,
        timeout: Option<Duration>,
    ) -> BashResult {
        self.run_command(command, output_tx, cancel, timeout)
            .await
            .unwrap_or_else(|e| BashResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Failed to execute: {e}"),
                cancelled: false,
                timed_out: false,
            })
    }

    /// Core subprocess execution with concurrent stdout/stderr reads.
    /// Both streams are read concurrently via `tokio::spawn` to avoid deadlocks
    /// and ensure correct interleaving.
    async fn run_command(
        &self,
        command: &str,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
        cancel: CancellationToken,
        timeout: Option<Duration>,
    ) -> io::Result<BashResult> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Shared accumulators so both readers can push lines concurrently
        let output = std::sync::Arc::new(std::sync::Mutex::new((String::new(), String::new())));

        // Read stdout in background
        let tx_out = output_tx.clone();
        let out_accum = output.clone();
        let cancel_out = cancel.clone();
        let stdout_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    result = reader.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                if !line.is_empty() {
                                    emit_event(
                                        &tx_out,
                                        ServerEvent::BashOutput {
                                            stdout: line.clone() + "\n",
                                            stderr: String::new(),
                                        },
                                    );
                                    let mut guard = out_accum.lock().unwrap();
                                    guard.0.push_str(&line);
                                    guard.0.push('\n');
                                }
                            }
                            _ => break,
                        }
                    }
                    _ = cancel_out.cancelled() => break,
                }
            }
        });

        // Read stderr in background
        let tx_err = output_tx.clone();
        let err_accum = output.clone();
        let cancel_err = cancel.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            loop {
                tokio::select! {
                    result = reader.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                if !line.is_empty() {
                                    emit_event(
                                        &tx_err,
                                        ServerEvent::BashOutput {
                                            stdout: String::new(),
                                            stderr: line.clone() + "\n",
                                        },
                                    );
                                    let mut guard = err_accum.lock().unwrap();
                                    guard.1.push_str(&line);
                                    guard.1.push('\n');
                                }
                            }
                            _ => break,
                        }
                    }
                    _ = cancel_err.cancelled() => break,
                }
            }
        });

        // Race between process completion, cancellation, and optional timeout
        let (exit_code, cancelled, timed_out) = if let Some(timeout_dur) = timeout {
            tokio::select! {
                status = child.wait() => {
                    match status {
                        Ok(s) => (s.code().unwrap_or(-1), false, false),
                        Err(_) => (-1, false, false),
                    }
                }
                _ = cancel.cancelled() => {
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                    (-1, true, false)
                }
                _ = tokio::time::sleep(timeout_dur) => {
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                    (-1, false, true)
                }
            }
        } else {
            tokio::select! {
                status = child.wait() => {
                    match status {
                        Ok(s) => (s.code().unwrap_or(-1), false, false),
                        Err(_) => (-1, false, false),
                    }
                }
                _ = cancel.cancelled() => {
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                    (-1, true, false)
                }
            }
        };

        // Ensure background readers finish draining
        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        // Emit BashDone
        emit_event(output_tx, ServerEvent::BashDone { exit_code });

        // Collect accumulated output
        let (stdout, stderr) = output.lock().unwrap().clone();

        Ok(BashResult {
            exit_code,
            stdout,
            stderr,
            cancelled,
            timed_out,
        })
    }
}
