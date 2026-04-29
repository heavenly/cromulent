use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ProviderEvent, ProviderRequest};
use crate::providers::{LlmProvider, ProviderError};

/// Fake provider for testing.
///
/// By default it emits a single text delta and completes.
pub struct FakeProvider {
    /// Optional scripted events to emit.
    pub script: Option<Vec<ProviderEvent>>,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self { script: None }
    }
}

impl FakeProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a provider that emits the given events in sequence.
    pub fn scripted(events: Vec<ProviderEvent>) -> Self {
        Self {
            script: Some(events),
        }
    }
}

#[async_trait]
impl LlmProvider for FakeProvider {
    async fn stream(
        &self,
        _request: ProviderRequest,
        _cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError> {
        let (tx, rx) = mpsc::unbounded_channel();

        if let Some(script) = self.script.clone() {
            tokio::spawn(async move {
                for event in script {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            });
        } else {
            let _ = tx.send(ProviderEvent::TextDelta {
                text: "Fake provider response.".to_string(),
            });
            let _ = tx.send(ProviderEvent::Completed);
        }

        Ok(rx)
    }
}
