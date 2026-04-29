use cromulent::app::state::AppConfig;
use cromulent::auth::config::{load_config, AppConfigFile, ProviderAuthConfig};
use cromulent::protocol::types::{ModelInfo, ThinkingLevel};

// -----------------------------------------------------------------------
// AppConfigFile defaults
// -----------------------------------------------------------------------

#[test]
fn test_config_file_default_has_expected_providers() {
    let config = AppConfigFile::default();
    assert!(config.providers.contains_key("openai"));
    assert!(!config.providers.contains_key("deepseek"));
    assert!(config.providers.contains_key("opencode"));
    assert_eq!(config.max_turns, Some(40));
    assert_eq!(config.thinking_level, Some(ThinkingLevel::Medium));
}

#[test]
fn test_config_file_default_model_info() {
    let config = AppConfigFile::default();
    let model = config.default_model.as_ref().unwrap();
    assert_eq!(model.provider, "openai");
    assert_eq!(model.id, "gpt-5.5");
    assert!(model.supports_tools);
}

// -----------------------------------------------------------------------
// AppConfigFile::resolve_api_key
// -----------------------------------------------------------------------

#[test]
fn test_resolve_api_key_from_config_env_var() {
    unsafe { std::env::set_var("TEST_OPENAI_KEY", "sk-test-config") };

    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "test-provider".into(),
        ProviderAuthConfig {
            api_key: None,
            api_key_env: Some("TEST_OPENAI_KEY".into()),
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
    assert_eq!(key, Some("sk-test-config".to_string()));

    unsafe { std::env::remove_var("TEST_OPENAI_KEY") };
}

#[test]
fn test_resolve_api_key_fallback_convention() {
    unsafe { std::env::set_var("CUSTOM_PROVIDER_API_KEY", "sk-convention-fallback") };

    let config = AppConfigFile::default();
    let key = config.resolve_api_key("custom-provider");
    assert_eq!(key, Some("sk-convention-fallback".to_string()));

    unsafe { std::env::remove_var("CUSTOM_PROVIDER_API_KEY") };
}

#[test]
fn test_resolve_api_key_missing_returns_none() {
    unsafe { std::env::remove_var("NONEXISTENT_PROVIDER_API_KEY") };
    let config = AppConfigFile::default();
    let key = config.resolve_api_key("nonexistent-provider");
    assert!(key.is_none());
}

#[test]
fn test_resolve_api_key_config_takes_precedence_over_convention() {
    unsafe {
        std::env::set_var("PRECEDENCE_API_KEY", "sk-convention");
        std::env::set_var("PRECEDENCE_EXPLICIT", "sk-explicit");
    }

    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "precedence".into(),
        ProviderAuthConfig {
            api_key: None,
            api_key_env: Some("PRECEDENCE_EXPLICIT".into()),
            base_url: None,
            default_model: None,
        },
    );
    let config = AppConfigFile {
        providers,
        ..AppConfigFile::default()
    };

    let key = config.resolve_api_key("precedence");
    assert_eq!(key, Some("sk-explicit".to_string()));

    unsafe {
        std::env::remove_var("PRECEDENCE_API_KEY");
        std::env::remove_var("PRECEDENCE_EXPLICIT");
    }
}

#[test]
fn test_resolve_api_key_skips_empty_env_var() {
    unsafe { std::env::set_var("EMPTY_KEY", "") };

    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "empty-test".into(),
        ProviderAuthConfig {
            api_key: None,
            api_key_env: Some("EMPTY_KEY".into()),
            base_url: None,
            default_model: None,
        },
    );
    let config = AppConfigFile {
        providers,
        ..AppConfigFile::default()
    };

    let key = config.resolve_api_key("empty-test");
    assert!(key.is_none());

    unsafe { std::env::remove_var("EMPTY_KEY") };
}

// -----------------------------------------------------------------------
// load_config edge cases
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_load_config_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.json");
    tokio::fs::write(&path, "").await.unwrap();

    let result = load_config(&path).await;
    assert!(result.is_err(), "Empty file should produce an error");
}

#[tokio::test]
async fn test_load_config_invalid_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    tokio::fs::write(&path, "not valid json").await.unwrap();

    let result = load_config(&path).await;
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// ProviderAuthConfig::resolve_api_key
// -----------------------------------------------------------------------

#[test]
fn test_provider_auth_config_resolve_explicit_env() {
    unsafe { std::env::set_var("MY_CUSTOM_KEY", "sk-my-key") };

    let auth = ProviderAuthConfig {
        api_key: None,
        api_key_env: Some("MY_CUSTOM_KEY".into()),
        base_url: None,
        default_model: None,
    };

    let key = auth.resolve_api_key("my-provider");
    assert_eq!(key, Some("sk-my-key".to_string()));

    unsafe { std::env::remove_var("MY_CUSTOM_KEY") };
}

#[test]
fn test_provider_auth_config_resolve_convention() {
    unsafe { std::env::set_var("MY_PROVIDER_API_KEY", "sk-convention-key") };

    let auth = ProviderAuthConfig {
        api_key: None,
        api_key_env: None,
        base_url: None,
        default_model: None,
    };

    let key = auth.resolve_api_key("my-provider");
    assert_eq!(key, Some("sk-convention-key".to_string()));

    unsafe { std::env::remove_var("MY_PROVIDER_API_KEY") };
}

#[test]
fn test_provider_auth_config_resolve_neither() {
    unsafe { std::env::remove_var("NO_PROVIDER_API_KEY") };

    let auth = ProviderAuthConfig {
        api_key: None,
        api_key_env: None,
        base_url: None,
        default_model: None,
    };

    let key = auth.resolve_api_key("no-provider");
    assert!(key.is_none());
}

// -----------------------------------------------------------------------
// AppConfig direct construction and defaults
// -----------------------------------------------------------------------

#[test]
fn test_app_config_default() {
    let config = AppConfig::default();
    assert_eq!(config.max_turns, 40);
    assert_eq!(config.default_model.provider, "openai");
    assert_eq!(config.default_model.id, "gpt-5.5");
    assert_eq!(config.default_thinking, ThinkingLevel::Medium);
    // sessions_dir should be a non-empty path
    assert!(!config.sessions_dir.as_os_str().is_empty());
}

// -----------------------------------------------------------------------
// AppConfigFile with custom values (direct construction)
// -----------------------------------------------------------------------

#[test]
fn test_config_file_with_custom_values() {
    let config = AppConfigFile {
        providers: std::collections::HashMap::new(),
        default_model: Some(ModelInfo {
            provider: "deepseek".into(),
            id: "deepseek-coder".into(),
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        }),
        thinking_level: Some(ThinkingLevel::High),
        max_turns: Some(10),
    };
    assert_eq!(config.max_turns, Some(10));
    assert_eq!(config.default_model.as_ref().unwrap().provider, "deepseek");
    assert_eq!(config.thinking_level, Some(ThinkingLevel::High));
}

#[test]
fn test_config_file_with_none_values_uses_default_through_load() {
    // When all optional fields are None, the Default impl fills them in
    let config = AppConfigFile::default();
    assert_eq!(config.max_turns, Some(40));
    assert!(config.default_model.is_some());
    assert_eq!(config.default_model.as_ref().unwrap().id, "gpt-5.5");
    assert_eq!(config.thinking_level, Some(ThinkingLevel::Medium));
}

// -----------------------------------------------------------------------
// AppConfigFile serde roundtrip
// -----------------------------------------------------------------------

#[test]
fn test_config_file_serde_roundtrip() {
    let config = AppConfigFile::default();
    let json = serde_json::to_string_pretty(&config).unwrap();
    let back: AppConfigFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back.max_turns, config.max_turns);
    assert_eq!(back.thinking_level, config.thinking_level);
    assert!(back.providers.contains_key("openai"));
    assert!(!back.providers.contains_key("deepseek"));
}
