use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::auth::config::ProviderAuthConfig;
use crate::util::fs::default_cromulent_dir;

/// User-defined custom providers loaded from `~/.cromulent/providers.json`.
///
/// Each entry maps a provider name (e.g. `"ollama"`, `"groq"`) to its
/// OpenAI-compatible Chat Completions configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct ProvidersConfigFile {
    #[serde(default)]
    pub providers: HashMap<String, ProviderAuthConfig>,
}

/// Get the default path for the custom providers config file.
pub fn default_providers_config_path() -> std::path::PathBuf {
    default_cromulent_dir().join("providers.json")
}

/// Load custom providers from a JSON file.
/// Returns the default (empty) config if the file does not exist.
pub async fn load_providers_config(path: impl AsRef<Path>) -> std::io::Result<ProvidersConfigFile> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(ProvidersConfigFile::default());
    }
    let content = tokio::fs::read_to_string(path).await?;
    serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Load custom providers from the default path (`~/.cromulent/providers.json`).
pub async fn load_default_providers_config() -> std::io::Result<ProvidersConfigFile> {
    load_providers_config(default_providers_config_path()).await
}
