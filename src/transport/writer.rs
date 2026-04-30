use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::time::{self, Duration};

use crate::protocol::responses::CommandResponse;

/// Items that can be written to stdout via the TransportWriter
#[derive(Debug)]
pub enum OutputItem {
    Event(serde_json::Value),
    Response(CommandResponse),
}

/// The only component allowed to write to stdout.
/// Serializes both server events and command responses as JSONL.
/// Uses a BufWriter to reduce syscall overhead, flushing periodically.
pub struct TransportWriter {
    rx: mpsc::UnboundedReceiver<OutputItem>,
}

impl TransportWriter {
    pub fn new(rx: mpsc::UnboundedReceiver<OutputItem>) -> Self {
        Self { rx }
    }

    /// Start the writer loop. Runs forever until the channel is closed.
    /// Writes are buffered and flushed on an interval or when the buffer fills.
    pub async fn run(&mut self) {
        let stdout = tokio::io::stdout();
        let mut writer = tokio::io::BufWriter::with_capacity(8192, stdout);
        let mut flush_interval = time::interval(Duration::from_millis(20));
        // Don't tick immediately — first write goes to buffer
        flush_interval.reset();

        loop {
            tokio::select! {
                item = self.rx.recv() => {
                    match item {
                        Some(out) => {
                            let json = match out {
                                OutputItem::Event(value) => serde_json::to_string(&value),
                                OutputItem::Response(resp) => serde_json::to_string(&resp),
                            };

                            match json {
                                Ok(line) => {
                                    if let Err(e) = writer.write_all(line.as_bytes()).await {
                                        tracing::error!("Failed to write to stdout: {e}");
                                        break;
                                    }
                                    if let Err(e) = writer.write_all(b"\n").await {
                                        tracing::error!("Failed to write newline: {e}");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to serialize output: {e}");
                                }
                            }
                        }
                        None => {
                            // Channel closed — flush and exit
                            let _ = writer.flush().await;
                            break;
                        }
                    }
                }
                _ = flush_interval.tick() => {
                    // Periodic flush to keep streaming responsive
                    if let Err(e) = writer.flush().await {
                        tracing::error!("Failed to flush stdout: {e}");
                        break;
                    }
                }
            }
        }
    }
}
