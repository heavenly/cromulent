use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::ProviderEvent;
use crate::providers::{LlmProvider, OpenAiCompatProvider, ProviderError};

/// Adapter for DeepSeek-compatible Chat Completions API (`deepseek` provider).
///
/// Thin wrapper around [`OpenAiCompatProvider`] that pre-configures the
/// DeepSeek base URL and reads `DEEPSEEK_API_KEY` from the environment.
#[derive(Debug, Clone)]
pub struct DeepSeekCompatProvider {
    inner: OpenAiCompatProvider,
}

impl DeepSeekCompatProvider {
    pub fn new() -> Self {
        let api_key = std::env::var("DEEPSEEK_API_KEY").ok();
        let base_url = std::env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/chat/completions".to_string());
        Self {
            inner: OpenAiCompatProvider::new("deepseek", api_key, base_url),
        }
    }

    /// Construct with an explicit API key, useful for tests and embedded callers.
    pub fn with_api_key(api_key: Option<String>) -> Self {
        let base_url = std::env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/chat/completions".to_string());
        Self {
            inner: OpenAiCompatProvider::new("deepseek", api_key, base_url),
        }
    }

    /// Override the endpoint for tests or proxies.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let api_key = std::env::var("DEEPSEEK_API_KEY").ok();
        self.inner = OpenAiCompatProvider::new("deepseek", api_key, base_url);
        self
    }

    /// Check whether an API key is configured.
    pub fn is_configured(&self) -> bool {
        self.inner.is_configured()
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
        request: crate::protocol::types::ProviderRequest,
        cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError> {
        self.inner.stream(request, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_api_key_constructor() {
        let provider = DeepSeekCompatProvider::with_api_key(Some("key".to_string()));
        assert!(provider.is_configured());
        let provider = DeepSeekCompatProvider::with_api_key(None);
        assert!(!provider.is_configured());
    }
}
