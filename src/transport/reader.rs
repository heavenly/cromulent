use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::protocol::commands::{ClientCommand, ClientCommandEnvelope};

/// Reads JSONL lines from stdin and sends parsed commands via a channel
pub struct TransportReader {
    cmd_tx: mpsc::UnboundedSender<ClientCommand>,
}

impl TransportReader {
    pub fn new(cmd_tx: mpsc::UnboundedSender<ClientCommand>) -> Self {
        Self { cmd_tx }
    }

    /// Start reading from stdin in a background task.
    /// Returns a handle that can be awaited for graceful shutdown.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let reader = BufReader::new(stdin);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() || trimmed.starts_with("//") {
                    continue;
                }

                match serde_json::from_str::<ClientCommandEnvelope>(&trimmed) {
                    Ok(envelope) => {
                        if self.cmd_tx.send(envelope.command).is_err() {
                            // Receiver dropped, shutting down
                            break;
                        }
                    }
                    Err(e) => {
                        // Structured parse error — send as an invalid command
                        let _ = self.cmd_tx.send(ClientCommand::Shutdown {
                            id: None,
                        });
                        tracing::error!("Failed to parse command: {e}");
                        break;
                    }
                }
            }
        })
    }
}
