use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::protocol::responses::CommandResponse;

/// Items that can be written to stdout via the TransportWriter
#[derive(Debug)]
pub enum OutputItem {
    Event(serde_json::Value),
    Response(CommandResponse),
}

/// The only component allowed to write to stdout.
/// Serializes both server events and command responses as JSONL.
pub struct TransportWriter {
    rx: mpsc::UnboundedReceiver<OutputItem>,
}

impl TransportWriter {
    pub fn new(rx: mpsc::UnboundedReceiver<OutputItem>) -> Self {
        Self { rx }
    }

    /// Start the writer loop. Runs forever until the channel is closed.
    pub async fn run(&mut self) {
        let mut stdout = tokio::io::stdout();
        while let Some(item) = self.rx.recv().await {
            let json = match item {
                OutputItem::Event(value) => serde_json::to_string(&value),
                OutputItem::Response(resp) => serde_json::to_string(&resp),
            };

            match json {
                Ok(line) => {
                    if let Err(e) = stdout.write_all(line.as_bytes()).await {
                        tracing::error!("Failed to write to stdout: {e}");
                        break;
                    }
                    if let Err(e) = stdout.write_all(b"\n").await {
                        tracing::error!("Failed to write newline: {e}");
                        break;
                    }
                    if let Err(e) = stdout.flush().await {
                        tracing::error!("Failed to flush stdout: {e}");
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to serialize output: {e}");
                }
            }
        }
    }
}
