use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::protocol::types::{ModelInfo, ThinkingLevel};
use crate::util::fs::default_config_path;

// ---------------------------------------------------------------------------
// Auth / provider config
// ---------------------------------------------------------------------------

/// Authentication configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthConfig {
    /// API key stored directly in the config file (highest priority).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Environment variable name that holds the API key.
    /// Defaults to `{PROVIDER_UPPERCASE}_API_KEY` if not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,

    /// Optional base URL override for the provider API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Optional default model ID for this provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

impl ProviderAuthConfig {
    /// Return the configured API key by checking, in priority order:
    /// 1. The `apiKey` field in the config file (direct key).
    /// 2. The env var specified in `api_key_env` (if set).
    /// 3. The env var `{PROVIDER}_API_KEY` in uppercase (convention).
    pub fn resolve_api_key(&self, provider_name: &str) -> Option<String> {
        // 1. Direct key in config file
        if let Some(key) = &self.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }
        // 2. Explicit env var name
        if let Some(env_var) = &self.api_key_env {
            if let Ok(key) = std::env::var(env_var) {
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }
        // 3. Fall back to convention: {PROVIDER}_API_KEY
        let conventional = format!("{}_API_KEY", provider_name.to_uppercase().replace('-', "_"));
        std::env::var(conventional).ok().filter(|k| !k.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Top-level config file
// ---------------------------------------------------------------------------

/// Configuration file format stored at `~/.cromulent/config.json`.
///
/// Maps providers to their auth settings and contains optional defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfigFile {
    /// Per-provider auth configuration.
    #[serde(default)]
    pub providers: HashMap<String, ProviderAuthConfig>,

    /// The default model to use when none is specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<ModelInfo>,

    /// The default thinking level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,

    /// Maximum number of agent turns before forced stop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

impl AppConfigFile {
    /// Resolve the API key for a given provider name.
    /// Checks the provider's config for an explicit env var, then falls back
    /// to the conventional `{PROVIDER}_API_KEY` environment variable.
    pub fn resolve_api_key(&self, provider_name: &str) -> Option<String> {
        self.providers
            .get(provider_name)
            .and_then(|cfg| cfg.resolve_api_key(provider_name))
            .or_else(|| {
                let conventional =
                    format!("{}_API_KEY", provider_name.to_uppercase().replace('-', "_"));
                std::env::var(conventional).ok().filter(|k| !k.is_empty())
            })
    }

    /// Convert this config file into an [`AppConfig`] suitable for runtime use.
    /// Fields that are `None` in the file receive sensible defaults.
    pub fn to_app_config(&self) -> crate::app::state::AppConfig {
        let default_model = self
            .default_model
            .clone()
            .unwrap_or_else(default_model_info);
        let default_thinking = self.thinking_level.clone().unwrap_or(ThinkingLevel::Medium);
        let max_turns = self.max_turns.unwrap_or(40);
        crate::app::state::AppConfig {
            max_turns,
            sessions_dir: crate::util::fs::default_sessions_dir(),
            default_model,
            default_thinking,
        }
    }

    /// Merge CLI overrides into a resulting [`AppConfig`].
    ///
    /// Any `Some` value from the CLI will replace the corresponding field.
    pub fn merge_with_cli(
        &self,
        cli_provider: Option<&str>,
        cli_model: Option<&str>,
        cli_thinking: Option<ThinkingLevel>,
        cli_max_turns: Option<u32>,
    ) -> crate::app::state::AppConfig {
        let mut cfg = self.to_app_config();

        if let Some(provider) = cli_provider {
            cfg.default_model.provider = provider.to_string();
        }
        if let Some(model_id) = cli_model {
            cfg.default_model.id = model_id.to_string();
        }
        if let Some(tl) = cli_thinking {
            cfg.default_thinking = tl;
        }
        if let Some(mt) = cli_max_turns {
            cfg.max_turns = mt;
        }

        cfg
    }
}

fn default_model_info() -> ModelInfo {
    ModelInfo {
        provider: "openai".to_string(),
        id: "gpt-5.5".to_string(),
        display_name: "GPT-5.5".to_string(),
        context_window: 200_000,
        supports_reasoning: true,
        supports_tools: true,
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load configuration from a JSON file.
/// Returns the default config if the file does not exist.
pub async fn load_config(path: impl AsRef<Path>) -> std::io::Result<AppConfigFile> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(AppConfigFile::default());
    }
    let content = tokio::fs::read_to_string(path).await?;
    serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Load configuration from the default config path
/// (`~/.cromulent/config.json`).
pub async fn load_default_config() -> std::io::Result<AppConfigFile> {
    load_config(default_config_path()).await
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

impl Default for AppConfigFile {
    fn default() -> Self {
        let mut providers = HashMap::new();
        providers.insert(
            "openai".to_string(),
            ProviderAuthConfig {
                api_key: None,
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                base_url: None,
                default_model: Some("gpt-5.5".to_string()),
            },
        );
        providers.insert(
            "opencode".to_string(),
            ProviderAuthConfig {
                api_key: None,
                api_key_env: Some("OPENCODE_API_KEY".to_string()),
                base_url: None,
                default_model: None,
            },
        );

        Self {
            providers,
            default_model: Some(ModelInfo {
                provider: "openai".to_string(),
                id: "gpt-5.5".to_string(),
                display_name: "GPT-5.5".to_string(),
                context_window: 200_000,
                supports_reasoning: true,
                supports_tools: true,
            }),
            thinking_level: Some(ThinkingLevel::Medium),
            max_turns: Some(40),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/inline/auth_config_tests.rs"]
mod tests;
