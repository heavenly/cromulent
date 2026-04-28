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
    /// Return the configured API key by checking:
    /// 1. The env var specified in `api_key_env` (if set).
    /// 2. The env var `{PROVIDER}_API_KEY` in uppercase (convention).
    pub fn resolve_api_key(&self, provider_name: &str) -> Option<String> {
        // Try explicit env var name first
        if let Some(env_var) = &self.api_key_env {
            if let Ok(key) = std::env::var(env_var) {
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }
        // Fall back to convention: {PROVIDER}_API_KEY
        let conventional = format!("{}_API_KEY", provider_name.to_uppercase().replace('-', "_"));
        std::env::var(conventional).ok().filter(|k| !k.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Top-level config file
// ---------------------------------------------------------------------------

/// Configuration file format stored at `~/.config/cromulent/config.json`.
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
/// (`~/.config/cromulent/config.json`).
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
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                base_url: None,
                default_model: Some("gpt-4o".to_string()),
            },
        );
        providers.insert(
            "deepseek".to_string(),
            ProviderAuthConfig {
                api_key_env: Some("DEEPSEEK_API_KEY".to_string()),
                base_url: None,
                default_model: Some("deepseek-chat".to_string()),
            },
        );
        providers.insert(
            "opencode".to_string(),
            ProviderAuthConfig {
                api_key_env: Some("OPENCODE_API_KEY".to_string()),
                base_url: None,
                default_model: None,
            },
        );

        Self {
            providers,
            default_model: Some(ModelInfo {
                provider: "openai".to_string(),
                id: "gpt-4o".to_string(),
                display_name: "GPT-4o".to_string(),
                context_window: 128_000,
                supports_reasoning: false,
                supports_tools: true,
            }),
            thinking_level: Some(ThinkingLevel::Medium),
            max_turns: Some(40),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = AppConfigFile::default();
        assert!(config.providers.contains_key("openai"));
        assert!(config.providers.contains_key("deepseek"));
        assert_eq!(config.max_turns, Some(40));
        assert_eq!(config.thinking_level, Some(ThinkingLevel::Medium));
    }

    #[test]
    fn test_resolve_api_key_from_env() {
        // Set a known env var for testing
        unsafe { std::env::set_var("TEST_CROMULENT_KEY", "sk-test123") };

        let mut providers = HashMap::new();
        providers.insert(
            "test-provider".to_string(),
            ProviderAuthConfig {
                api_key_env: Some("TEST_CROMULENT_KEY".to_string()),
                base_url: None,
                default_model: None,
            },
        );
        let config = AppConfigFile {
            providers,
            default_model: None,
            thinking_level: None,
            max_turns: None,
        };

        let key = config.resolve_api_key("test-provider");
        assert_eq!(key, Some("sk-test123".to_string()));

        unsafe { std::env::remove_var("TEST_CROMULENT_KEY") };
    }

    #[test]
    fn test_resolve_api_key_convention() {
        unsafe { std::env::set_var("MYPROV_API_KEY", "sk-convention") };

        let config = AppConfigFile::default();
        // Add a provider without explicit env var config
        let mut providers = HashMap::new();
        providers.insert(
            "myprov".to_string(),
            ProviderAuthConfig {
                api_key_env: None,
                base_url: None,
                default_model: None,
            },
        );
        let config = AppConfigFile {
            providers,
            ..config
        };

        let key = config.resolve_api_key("myprov");
        assert_eq!(key, Some("sk-convention".to_string()));

        unsafe { std::env::remove_var("MYPROV_API_KEY") };
    }

    #[tokio::test]
    async fn test_load_nonexistent_config_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let config = load_config(&path).await.unwrap();
        // Should return the default config, not error
        assert_eq!(config.max_turns, Some(40));
    }

    #[tokio::test]
    async fn test_load_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let config = AppConfigFile::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        tokio::fs::write(&path, &json).await.unwrap();

        let loaded = load_config(&path).await.unwrap();
        assert_eq!(loaded.max_turns, config.max_turns);
        assert_eq!(loaded.thinking_level, config.thinking_level);
        assert!(loaded.providers.contains_key("openai"));
        assert!(loaded.providers.contains_key("deepseek"));
    }
}
