use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ProviderEvent, ProviderRequest};
use crate::providers::{LlmProvider, ProviderError};

/// Adapter for DeepSeek-compatible Chat Completions API (`deepseek` provider).
///
/// This is a compile-safe skeleton. Full HTTP streaming will be added
/// in a later phase. Currently returns an error if `DEEPSEEK_API_KEY` is
/// not set, otherwise emits a placeholder response for testing.
#[derive(Debug)]
pub struct DeepSeekCompatProvider {
    api_key: Option<String>,
}

impl DeepSeekCompatProvider {
    pub fn new() -> Self {
        let api_key = std::env::var("DEEPSEEK_API_KEY").ok();
        Self { api_key }
    }

    /// Check whether an API key is configured.
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }
}

impl Default for DeepSeekCompatProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for DeepSeekCompatProvider {
    async fn stream(
        &self,
        request: ProviderRequest,
        _cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| ProviderError::ApiKeyMissing("deepseek".to_string()))?;

        // Stub: acknowledge the key exists but don't make real HTTP calls.
        let _ = api_key;
        let _ = request;

        // Emit a placeholder response so the agent loop doesn't hang.
        let (tx, rx) = mpsc::unbounded_channel();

        tx.send(ProviderEvent::TextDelta {
            text: "[DeepSeek provider not yet implemented. This is a placeholder response.]"
                .to_string(),
        })
        .ok();

        tx.send(ProviderEvent::Usage {
            input_tokens: 0,
            output_tokens: 0,
        })
        .ok();

        tx.send(ProviderEvent::Completed).ok();

        Ok(rx)
    }
}
