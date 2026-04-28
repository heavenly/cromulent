use std::path::PathBuf;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::transport::writer::OutputItem;

/// Executes raw bash commands (UI-initiated, not agent tools).
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
    pub async fn execute(
        &self,
        command: &str,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
    ) -> std::io::Result<i32> {
        use crate::app::output::emit_event;
        use crate::protocol::events::ServerEvent;

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncBufReadExt;

        // Read stdout in background
        let tx_stdout = output_tx.clone();
        let stdout_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.is_empty() {
                    emit_event(
                        &tx_stdout,
                        ServerEvent::BashOutput {
                            stdout: line + "\n",
                            stderr: String::new(),
                        },
                    );
                }
            }
        });

        // Read stderr in background
        let tx_stderr = output_tx.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.is_empty() {
                    emit_event(
                        &tx_stderr,
                        ServerEvent::BashOutput {
                            stdout: String::new(),
                            stderr: line + "\n",
                        },
                    );
                }
            }
        });

        let status = child.wait().await?;
        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        let exit_code = status.code().unwrap_or(-1);
        emit_event(
            output_tx,
            ServerEvent::BashDone { exit_code },
        );

        Ok(exit_code)
    }
}
