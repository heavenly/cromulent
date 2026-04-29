use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Codex-style credential types
// ---------------------------------------------------------------------------

/// Cached credentials obtained through the Codex auth flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCredentials {
    /// The access token used to authenticate API requests.
    pub access_token: String,

    /// The refresh token used to obtain a new access token.
    pub refresh_token: String,

    /// ISO 8601 timestamp indicating when the access token expires.
    pub expires_at: String,

    /// Optional token type (e.g., "Bearer").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,

    /// Optional scope granted with this token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Wrapper for cached credential file contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialCache {
    /// The provider name these credentials are for (e.g., "codex", "openai").
    pub provider: String,

    /// The cached credentials.
    pub credentials: CodexCredentials,

    /// Schema version for forward compatibility.
    pub schema_version: u32,
}

impl CredentialCache {
    pub fn new(provider: impl Into<String>, credentials: CodexCredentials) -> Self {
        Self {
            provider: provider.into(),
            credentials,
            schema_version: 1,
        }
    }
}

/// Default path for cached Codex credentials.
pub fn default_credentials_path() -> PathBuf {
    crate::util::fs::default_config_dir()
        .join("auth")
        .join("codex.json")
}

// ---------------------------------------------------------------------------
// Credential persistence
// ---------------------------------------------------------------------------

/// Load cached credentials from a JSON file.
///
/// Returns `Ok(None)` if the file does not exist.
pub async fn load_cached_credentials(
    path: impl AsRef<Path>,
) -> std::io::Result<Option<CredentialCache>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let content = tokio::fs::read_to_string(path).await?;
    let cache: CredentialCache = serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if cache.schema_version != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Unsupported credential schema version: {}",
                cache.schema_version
            ),
        ));
    }

    Ok(Some(cache))
}

/// Save cached credentials to a JSON file.
///
/// Creates parent directories if they don't exist.
pub async fn save_cached_credentials(
    path: impl AsRef<Path>,
    cache: &CredentialCache,
) -> std::io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(cache).map_err(|e| std::io::Error::other(e))?;
    tokio::fs::write(path.as_ref(), &json).await
}

// ---------------------------------------------------------------------------
// Refresh stub
// ---------------------------------------------------------------------------

/// Refresh errors for the Codex auth flow.
#[derive(Debug, Error)]
pub enum CodexAuthError {
    /// The refresh flow is not yet implemented.
    #[error("Codex credential refresh is not yet implemented. Provide a fresh API key or token manually.")]
    RefreshNotImplemented,

    /// An I/O error occurred while loading or saving credentials.
    #[error("credential I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The cached credentials have expired and refresh is unavailable.
    #[error("credentials expired at {expires_at} and refresh is not implemented")]
    Expired {
        /// ISO 8601 timestamp when the credentials expired.
        expires_at: String,
    },
}

/// Refresh cached Codex credentials.
///
/// Currently a placeholder that returns a clear error indicating refresh
/// is not yet implemented. This will be wired to the actual Codex OAuth
/// refresh flow in a future phase.
pub async fn refresh_credentials(
    _credentials: &CodexCredentials,
) -> Result<CodexCredentials, CodexAuthError> {
    Err(CodexAuthError::RefreshNotImplemented)
}

/// Check whether the cached credentials are expired based on the current time.
///
/// Returns `Ok(true)` if the token is expired, `Ok(false)` if still valid,
/// or `Err` if the `expires_at` field cannot be parsed.
pub fn is_expired(credentials: &CodexCredentials) -> Result<bool, CodexAuthError> {
    let expires_at =
        chrono::DateTime::parse_from_rfc3339(&credentials.expires_at).map_err(|_| {
            CodexAuthError::Expired {
                expires_at: credentials.expires_at.clone(),
            }
        })?;
    let now = chrono::Utc::now();
    Ok(now > expires_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_credentials() -> CodexCredentials {
        CodexCredentials {
            access_token: "at_sample_token".to_string(),
            refresh_token: "rt_sample_refresh".to_string(),
            expires_at: "2099-12-31T23:59:59Z".to_string(), // far future
            token_type: Some("Bearer".to_string()),
            scope: Some("read write".to_string()),
        }
    }

    #[test]
    fn test_credential_cache_new() {
        let creds = sample_credentials();
        let cache = CredentialCache::new("codex", creds.clone());
        assert_eq!(cache.provider, "codex");
        assert_eq!(cache.schema_version, 1);
        assert_eq!(cache.credentials.access_token, "at_sample_token");
    }

    #[test]
    fn test_is_expired_false() {
        let creds = sample_credentials();
        assert!(!is_expired(&creds).unwrap());
    }

    #[test]
    fn test_is_expired_true() {
        let creds = CodexCredentials {
            expires_at: "2020-01-01T00:00:00Z".to_string(),
            ..sample_credentials()
        };
        assert!(is_expired(&creds).unwrap());
    }

    #[tokio::test]
    async fn test_credential_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth").join("codex.json");

        let creds = sample_credentials();
        let cache = CredentialCache::new("codex", creds.clone());
        save_cached_credentials(&path, &cache).await.unwrap();

        let loaded = load_cached_credentials(&path).await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.provider, "codex");
        assert_eq!(loaded.credentials.access_token, "at_sample_token");
        assert_eq!(loaded.credentials.refresh_token, "rt_sample_refresh");
    }

    #[tokio::test]
    async fn test_load_nonexistent_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let loaded = load_cached_credentials(&path).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_refresh_placeholder_returns_error() {
        let creds = sample_credentials();
        let result = refresh_credentials(&creds).await;
        assert!(result.is_err());
        match result {
            Err(CodexAuthError::RefreshNotImplemented) => {} // expected
            _ => panic!("expected RefreshNotImplemented"),
        }
    }
}
