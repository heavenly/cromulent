use super::*;

#[test]
fn test_default_config_is_valid() {
    let config = AppConfigFile::default();
    assert!(config.providers.contains_key("openai"));
    assert!(!config.providers.contains_key("deepseek"));
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
            api_key: None,
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
            api_key: None,
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

#[test]
fn test_resolve_api_key_direct() {
    // apiKey stored directly in config takes highest priority
    let mut providers = HashMap::new();
    providers.insert(
        "myprov".to_string(),
        ProviderAuthConfig {
            api_key: Some("sk-direct-key".to_string()),
            api_key_env: Some("IGNORED_ENV".to_string()),
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

    let key = config.resolve_api_key("myprov");
    assert_eq!(key, Some("sk-direct-key".to_string()));
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
    assert!(!loaded.providers.contains_key("deepseek"));
}

#[test]
fn test_to_app_config() {
    use crate::app::state::AppConfig;
    use crate::protocol::types::ThinkingLevel;

    let config_file = AppConfigFile::default();
    let app_config: AppConfig = config_file.to_app_config();

    assert_eq!(app_config.max_turns, 40);
    assert_eq!(app_config.default_thinking, ThinkingLevel::Medium);
    assert_eq!(app_config.default_model.id, "gpt-5.5");
    assert_eq!(app_config.default_model.provider, "openai");
}

#[test]
fn test_merge_with_cli_overrides_model() {
    let config_file = AppConfigFile::default();
    let merged = config_file.merge_with_cli(
        Some("opencode"),
        Some("opencode-reasoner"),
        Some(ThinkingLevel::High),
        Some(100),
    );

    assert_eq!(merged.default_model.provider, "opencode");
    assert_eq!(merged.default_model.id, "opencode-reasoner");
    assert_eq!(merged.default_thinking, ThinkingLevel::High);
    assert_eq!(merged.max_turns, 100);
}

#[test]
fn test_merge_with_cli_partial_overrides() {
    let config_file = AppConfigFile::default();
    // Only override provider; model/thinking/max_turns should stay at defaults
    let merged = config_file.merge_with_cli(Some("opencode"), None, None, None);

    assert_eq!(merged.default_model.provider, "opencode");
    // id should remain from default
    assert_eq!(merged.default_model.id, "gpt-5.5");
    assert_eq!(merged.default_thinking, ThinkingLevel::Medium);
    assert_eq!(merged.max_turns, 40);
}

#[test]
fn test_merge_with_cli_no_overrides() {
    let config_file = AppConfigFile::default();
    let merged = config_file.merge_with_cli(None, None, None, None);

    assert_eq!(merged.default_model.provider, "openai");
    assert_eq!(merged.default_model.id, "gpt-5.5");
    assert_eq!(merged.default_thinking, ThinkingLevel::Medium);
    assert_eq!(merged.max_turns, 40);
}
