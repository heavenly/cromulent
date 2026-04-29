use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use cromulent::protocol::types::{
    LlmContentBlock, LlmMessage, ModelInfo, ProviderEvent, ProviderRequest, ThinkingLevel,
};
use cromulent::providers::{
    FakeProvider, LlmProvider, OpenAiCompatProvider, OpenAiResponsesProvider, ProviderError,
    ProviderManager,
};

fn dummy_request() -> ProviderRequest {
    ProviderRequest {
        model: ModelInfo {
            provider: "test".into(),
            id: "test-model".into(),
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        },
        system_prompt: "You are a test assistant.".into(),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: vec![LlmContentBlock::Text {
                text: "Hello".into(),
            }],
        }],
        tools: vec![],
        thinking_level: ThinkingLevel::Medium,
    }
}

// -----------------------------------------------------------------------
// ProviderManager basics
// -----------------------------------------------------------------------

#[test]
fn test_provider_manager_default_has_providers() {
    let mgr = ProviderManager::default();
    // Default is empty — use default_with_config() for configured providers
    assert!(mgr.provider_names().is_empty());
}

#[test]
fn test_provider_manager_empty_new() {
    let mgr = ProviderManager::new();
    assert!(mgr.provider_names().is_empty());
    assert!(!mgr.has_provider("openai"));
}

#[test]
fn test_provider_manager_register_and_get() {
    let mut mgr = ProviderManager::new();
    mgr.register("custom", Box::new(FakeProvider::new()));
    assert!(mgr.has_provider("custom"));
    let model = ModelInfo {
        provider: "custom".into(),
        id: "custom-model".into(),
        display_name: String::new(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    };
    assert!(mgr.get(&model).is_ok());
}

#[test]
fn test_provider_manager_get_not_found() {
    let mgr = ProviderManager::default();
    let model = ModelInfo {
        provider: "nonexistent".into(),
        id: "nope".into(),
        display_name: String::new(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    };
    let err = mgr.get(&model);
    match err {
        Err(ProviderError::NotFound(p)) => assert_eq!(p, "nonexistent"),
        Err(other) => panic!("Expected NotFound, got: {other}"),
        Ok(_) => panic!("Expected NotFound, got Ok"),
    }
}

// -----------------------------------------------------------------------
// OpenAiResponsesProvider: API key missing
// -----------------------------------------------------------------------

#[test]
fn test_openai_api_key_missing() {
    // Sync test to avoid parallel env var races
    let provider = OpenAiResponsesProvider::with_api_key(None);
    assert!(!provider.is_configured());

    let cancel = CancellationToken::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(provider.stream(dummy_request(), cancel));
    match result {
        Err(ProviderError::ApiKeyMissing(name)) => assert_eq!(name, "openai"),
        other => panic!("Expected ApiKeyMissing, got: {other:?}"),
    }
}

#[test]
fn test_openai_api_key_present_does_not_stream_without_network() {
    let (provider, cancel) = (
        OpenAiResponsesProvider::with_api_key(Some("sk-test-openai-key".to_string())),
        CancellationToken::new(),
    );
    assert!(provider.is_configured());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut rx = rt
        .block_on(provider.stream(dummy_request(), cancel))
        .unwrap();
    let event = rt.block_on(rx.recv());
    assert!(
        event.is_some(),
        "Expected some event from the provider stream"
    );
    assert!(
        matches!(&event, Some(ProviderEvent::Error { .. })),
        "Expected Error event (no network), got: {event:?}"
    );
}

// -----------------------------------------------------------------------
// OpenAiCompatProvider: API key missing
// -----------------------------------------------------------------------

#[test]
fn test_openai_compat_api_key_missing() {
    let provider = OpenAiCompatProvider::new(
        "deepseek",
        None,
        "https://api.deepseek.com/chat/completions",
    );
    assert!(!provider.is_configured());

    let cancel = CancellationToken::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(provider.stream(dummy_request(), cancel));
    match result {
        Err(ProviderError::ApiKeyMissing(name)) => assert_eq!(name, "deepseek"),
        other => panic!("Expected ApiKeyMissing, got: {other:?}"),
    }
}

// -----------------------------------------------------------------------
// OpenAiCompatProvider: API key present (no network)
// -----------------------------------------------------------------------

#[test]
fn test_openai_compat_api_key_present() {
    let (provider, cancel) = (
        OpenAiCompatProvider::new(
            "deepseek",
            Some("sk-test-deepseek-key".to_string()),
            "https://api.deepseek.com/chat/completions",
        ),
        CancellationToken::new(),
    );
    assert!(provider.is_configured());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut rx = rt
        .block_on(provider.stream(dummy_request(), cancel))
        .unwrap();
    let event = rt.block_on(rx.recv());
    assert!(
        matches!(event, Some(ProviderEvent::Error { .. })),
        "Expected Error event (no network), got: {event:?}"
    );
    let event2 = rt.block_on(rx.recv());
    assert!(matches!(event2, Some(ProviderEvent::Completed)));
}
// -----------------------------------------------------------------------
// FakeProvider: default (no script)
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_fake_provider_default() {
    let provider = FakeProvider::default();
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(dummy_request(), cancel).await.unwrap();

    let event1 = rx.recv().await;
    assert!(matches!(event1, Some(ProviderEvent::TextDelta { .. })));
    if let Some(ProviderEvent::TextDelta { text }) = event1 {
        assert_eq!(text, "Fake provider response.");
    }

    let event2 = rx.recv().await;
    assert!(matches!(event2, Some(ProviderEvent::Completed)));
}

// -----------------------------------------------------------------------
// FakeProvider: scripted text sequence
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_fake_provider_scripted_text_sequence() {
    let events = vec![
        ProviderEvent::TextDelta {
            text: "Hello ".into(),
        },
        ProviderEvent::TextDelta {
            text: "World".into(),
        },
        ProviderEvent::Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
        ProviderEvent::Completed,
    ];
    let provider = FakeProvider::scripted(events.clone());
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(dummy_request(), cancel).await.unwrap();

    // Collect all events
    let mut received = Vec::new();
    while let Some(event) = rx.recv().await {
        if matches!(event, ProviderEvent::Completed) {
            received.push(event);
            break;
        }
        received.push(event);
    }

    assert_eq!(received.len(), events.len());
    assert!(matches!(&received[0], ProviderEvent::TextDelta { text } if text == "Hello "));
    assert!(matches!(&received[1], ProviderEvent::TextDelta { text } if text == "World"));
    assert!(matches!(
        &received[2],
        ProviderEvent::Usage {
            input_tokens: 10,
            output_tokens: 5
        }
    ));
    assert!(matches!(&received[3], ProviderEvent::Completed));
}

// -----------------------------------------------------------------------
// FakeProvider: scripted tool call sequence
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_fake_provider_scripted_tool_call() {
    let events = vec![
        ProviderEvent::TextDelta {
            text: "I'll read the file.".into(),
        },
        ProviderEvent::ToolCallStarted {
            id: "call_1".into(),
            name: "read".into(),
        },
        ProviderEvent::ToolCallArgumentsDelta {
            id: "call_1".into(),
            delta: r#"{"path":""#.into(),
        },
        ProviderEvent::ToolCallArgumentsDelta {
            id: "call_1".into(),
            delta: r#""src/main.rs"}"#.into(),
        },
        ProviderEvent::ToolCallCompleted {
            id: "call_1".into(),
        },
        ProviderEvent::Usage {
            input_tokens: 25,
            output_tokens: 10,
        },
        ProviderEvent::Completed,
    ];
    let provider = FakeProvider::scripted(events.clone());
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(dummy_request(), cancel).await.unwrap();

    let mut received = Vec::new();
    while let Some(event) = rx.recv().await {
        let is_completed = matches!(&event, ProviderEvent::Completed);
        received.push(event.clone());
        // To avoid comparing ToolCallArgumentsDelta directly, just count them
        if is_completed {
            break;
        }
    }

    assert_eq!(received.len(), events.len());
    assert!(
        matches!(&received[1], ProviderEvent::ToolCallStarted { id, name } if id == "call_1" && name == "read")
    );
    assert!(
        matches!(&received[2], ProviderEvent::ToolCallArgumentsDelta { id, .. } if id == "call_1")
    );
    assert!(
        matches!(&received[3], ProviderEvent::ToolCallArgumentsDelta { id, .. } if id == "call_1")
    );
    assert!(matches!(&received[4], ProviderEvent::ToolCallCompleted { id } if id == "call_1"));
}

// -----------------------------------------------------------------------
// FakeProvider: scripted error sequence
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_fake_provider_scripted_error() {
    let events = vec![
        ProviderEvent::TextDelta {
            text: "Starting...".into(),
        },
        ProviderEvent::Error {
            message: "Something went wrong".into(),
        },
    ];
    let provider = FakeProvider::scripted(events);
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(dummy_request(), cancel).await.unwrap();

    // Default scripted doesn't send Completed after Error, it just stops
    let event1 = rx.recv().await;
    assert!(matches!(&event1, Some(ProviderEvent::TextDelta { text }) if text == "Starting..."));

    let event2 = rx.recv().await;
    assert!(
        matches!(&event2, Some(ProviderEvent::Error { message }) if message == "Something went wrong")
    );
}

// -----------------------------------------------------------------------
// FakeProvider: empty script
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_fake_provider_scripted_empty() {
    let provider = FakeProvider::scripted(vec![]);
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(dummy_request(), cancel).await.unwrap();

    // Empty script: receiver immediately returns None (channel closed)
    let event = rx.recv().await;
    assert!(
        event.is_none(),
        "Expected no events for empty script, got: {event:?}"
    );
}

// -----------------------------------------------------------------------
// ProviderManager resolves to real providers
// -----------------------------------------------------------------------

#[test]
fn test_provider_manager_resolves_openai() {
    let mgr = {
        unsafe { std::env::set_var("OPENAI_API_KEY", "sk-test") };
        let mut m = ProviderManager::new();
        m.register("openai", Box::new(OpenAiResponsesProvider::new()));
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        m
    };
    let model = ModelInfo {
        provider: "openai".into(),
        id: "gpt-5.5".into(),
        display_name: String::new(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    };
    let provider = mgr.get(&model).unwrap();
    let cancel = CancellationToken::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut rx = rt
        .block_on(provider.stream(dummy_request(), cancel))
        .unwrap();
    let event = rt.block_on(rx.recv());
    assert!(
        event.is_some(),
        "Expected some event from the provider stream"
    );
    assert!(
        matches!(&event, Some(ProviderEvent::Error { .. })),
        "Expected Error event (no network), got: {event:?}"
    );
}

#[tokio::test]
async fn test_provider_manager_resolves_fake() {
    let mut mgr = ProviderManager::new();
    mgr.register("fake", Box::new(FakeProvider::default()));
    let mgr = mgr;
    let model = ModelInfo {
        provider: "fake".into(),
        id: "fake".into(),
        display_name: String::new(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    };
    let provider = mgr.get(&model).unwrap();
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(dummy_request(), cancel).await.unwrap();
    assert!(matches!(
        rx.recv().await,
        Some(ProviderEvent::TextDelta { .. })
    ));
}

// -----------------------------------------------------------------------
// ProviderError Display
// -----------------------------------------------------------------------

#[test]
fn test_provider_error_display() {
    assert_eq!(
        format!("{}", ProviderError::NotFound("foo".into())),
        "Provider not found: foo"
    );
    assert_eq!(
        format!("{}", ProviderError::ApiKeyMissing("openai".into())),
        "API key not configured for provider `openai`"
    );
    assert_eq!(
        format!("{}", ProviderError::Cancelled),
        "Request was cancelled"
    );
}
