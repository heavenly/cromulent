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
