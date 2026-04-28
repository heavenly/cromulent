use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::providers::{LlmProvider, ProviderError};
use crate::protocol::types::{ProviderEvent, ProviderRequest};

/// OpenAI Responses API provider adapter.
pub struct OpenAiResponsesProvider;

impl OpenAiResponsesProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    async fn stream(
        &self,
        _request: ProviderRequest,
        _cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(ProviderEvent::Completed);
        Ok(rx)
    }
}
