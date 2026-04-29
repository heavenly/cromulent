use std::collections::HashMap;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ModelInfo, ProviderEvent, ProviderRequest};

mod fake;
mod openai_compat;
mod openai_responses;
pub(crate) mod retry;

pub use fake::FakeProvider;
pub use openai_compat::OpenAiCompatProvider;
pub use openai_responses::OpenAiResponsesProvider;

// ---------------------------------------------------------------------------
// ProviderError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("Provider not found: {0}")]
    NotFound(String),

    #[error("API key not configured for provider `{0}`")]
    ApiKeyMissing(String),

    #[error("Request failed: {message}")]
    RequestFailed {
        message: String,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Stream ended unexpectedly")]
    StreamEnded,

    #[error("Request was cancelled")]
    Cancelled,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ProviderError {
    pub fn request_failed(message: impl Into<String>) -> Self {
        Self::RequestFailed {
            message: message.into(),
            source: None,
        }
    }
}

// ---------------------------------------------------------------------------
// LlmProvider trait
// ---------------------------------------------------------------------------

/// A normalized LLM provider adapter.
///
/// Implementations translate provider-specific streaming (OpenAI Responses API,
/// OpenAI compatible, etc.) into the internal [`ProviderEvent`] stream.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Open a streaming request to the provider.
    ///
    /// Returns a receiver that yields normalized [`ProviderEvent`]s until
    /// `Completed` or `Error` is received, the channel is dropped, or
    /// `cancel` is triggered.
    async fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError>;
}

// ---------------------------------------------------------------------------
// ProviderManager
// ---------------------------------------------------------------------------

/// Manages registered provider adapters and resolves one by provider name.
pub struct ProviderManager {
    providers: HashMap<String, Box<dyn LlmProvider>>,
}

impl Default for ProviderManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderManager {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a provider under a name (e.g. `"openai"`, `"fake"`).
    pub fn register(&mut self, name: &str, provider: Box<dyn LlmProvider>) {
        self.providers.insert(name.to_string(), provider);
    }

    /// Resolve a provider by model info.
    pub fn get(&self, model: &ModelInfo) -> Result<&dyn LlmProvider, ProviderError> {
        self.providers
            .get(&model.provider)
            .map(|p| p.as_ref())
            .ok_or_else(|| ProviderError::NotFound(model.provider.clone()))
    }

    /// Check whether a provider name is registered.
    pub fn has_provider(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    /// List registered provider names.
    pub fn provider_names(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    /// Build the default set of providers (fake, openai).
    pub fn default() -> Self {
        let mut mgr = Self::new();
        mgr.register("fake", Box::new(FakeProvider::default()));
        mgr.register("openai", Box::new(OpenAiResponsesProvider::new()));
        mgr
    }

    /// Build the default set of providers, resolving API keys from the
    /// given config file (falls back to env vars when config has no entry).
    /// Also registers any additional providers defined in the config's
    /// `providers` map as [`OpenAiCompatProvider`] instances.
    pub fn default_with_config(config: &crate::auth::config::AppConfigFile) -> Self {
        let mut mgr = Self::new();
        mgr.register("fake", Box::new(FakeProvider::default()));

        let openai_key = config.resolve_api_key("openai");
        mgr.register(
            "openai",
            Box::new(OpenAiResponsesProvider::with_api_key(openai_key)),
        );

        // Register any custom providers defined in config.json
        mgr.register_custom_from_app_config(config);

        mgr
    }

    /// Register custom providers from the `providers` map in the app config.
    /// Skips built-in names (`openai`, `fake`) that already have
    /// dedicated adapters.
    pub fn register_custom_from_app_config(&mut self, config: &crate::auth::config::AppConfigFile) {
        let builtins: &[&str] = &["openai", "fake"];
        for (name, auth) in &config.providers {
            if builtins.contains(&name.as_str()) {
                continue;
            }
            if self.has_provider(name) {
                continue;
            }
            let api_key = auth.resolve_api_key(name);
            let base_url = auth
                .base_url
                .clone()
                .unwrap_or_else(|| format!("https://api.{name}.com/v1/chat/completions"));
            self.register(
                name,
                Box::new(OpenAiCompatProvider::new(name.clone(), api_key, base_url)),
            );
        }
    }

    /// Register custom providers loaded from `~/.cromulent/providers.json`.
    pub fn register_custom_from_providers_json(
        &mut self,
        config: &crate::auth::providers_config::ProvidersConfigFile,
    ) {
        for (name, auth) in &config.providers {
            if self.has_provider(name) {
                continue;
            }
            let api_key = auth.resolve_api_key(name);
            let base_url = auth
                .base_url
                .clone()
                .unwrap_or_else(|| format!("https://api.{name}.com/v1/chat/completions"));
            self.register(
                name,
                Box::new(OpenAiCompatProvider::new(name.clone(), api_key, base_url)),
            );
        }
    }
}

impl std::fmt::Debug for ProviderManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderManager")
            .field("providers", &self.provider_names())
            .finish()
    }
}
