use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use cromulent::app::state::{AppConfig, AppState, RunState};
use cromulent::protocol::types::{ModelInfo, ThinkingLevel};
use cromulent::session::store::{LoadedSessionState, SessionHeader};

fn default_config() -> AppConfig {
    AppConfig::default()
}

fn sample_session() -> LoadedSessionState {
    let header = SessionHeader::new(
        "ses_cancel_test".into(),
        "/tmp".into(),
        ModelInfo {
            provider: "openai".into(),
            id: "gpt-5.5".into(),
            display_name: "GPT-5.5".into(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        },
        ThinkingLevel::Medium,
    );
    LoadedSessionState {
        header,
        messages: Vec::new(),
    }
}

// -----------------------------------------------------------------------
// CancellationToken basics
// -----------------------------------------------------------------------

#[test]
fn test_cancellation_token_not_cancelled_by_default() {
    let token = CancellationToken::new();
    assert!(!token.is_cancelled());
}

#[test]
fn test_cancellation_token_cancel() {
    let token = CancellationToken::new();
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn test_cancellation_token_child_cancelled_with_parent() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    parent.cancel();
    assert!(child.is_cancelled());
}

#[test]
fn test_cancellation_token_child_independent_when_not_cancelled() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    assert!(!child.is_cancelled());
    assert!(!parent.is_cancelled());
}

#[test]
fn test_cancellation_token_clone() {
    let token = CancellationToken::new();
    let cloned = token.clone();
    token.cancel();
    assert!(cloned.is_cancelled());
}

// -----------------------------------------------------------------------
// RunState transitions
// -----------------------------------------------------------------------

#[test]
fn test_run_state_initial_idle() {
    let state = AppState::new(
        sample_session(),
        default_config(),
        default_config().default_model,
        default_config().default_thinking,
        PathBuf::from("/tmp"),
    );
    assert!(matches!(state.run_state, RunState::Idle));
    assert!(!state.is_running());
    assert!(state.run_id().is_none());
    assert!(state.cancel_token().is_none());
}

#[test]
fn test_run_state_transition_to_running() {
    let mut state = AppState::new(
        sample_session(),
        default_config(),
        default_config().default_model,
        default_config().default_thinking,
        PathBuf::from("/tmp"),
    );

    let cancel = CancellationToken::new();
    let run_id = "run_test_1".to_string();

    state.run_state = RunState::Running {
        run_id: run_id.clone(),
        cancel: cancel.clone(),
        started_at: chrono::Utc::now(),
    };

    assert!(state.is_running());
    assert_eq!(state.run_id(), Some("run_test_1"));
    assert!(state.cancel_token().is_some());

    // Cancel should work
    let token = state.cancel_token().unwrap();
    assert!(!token.is_cancelled());
}

#[test]
fn test_run_state_cancel_then_idle() {
    let mut state = AppState::new(
        sample_session(),
        default_config(),
        default_config().default_model,
        default_config().default_thinking,
        PathBuf::from("/tmp"),
    );

    let cancel = CancellationToken::new();
    state.run_state = RunState::Running {
        run_id: "run_test_2".to_string(),
        cancel: cancel.clone(),
        started_at: chrono::Utc::now(),
    };

    assert!(state.is_running());

    // Cancel the token
    state.run_state = RunState::Idle;
    assert!(!state.is_running());
    assert!(state.run_id().is_none());
    assert!(state.cancel_token().is_none());
}

// -----------------------------------------------------------------------
// AppState baseline
// -----------------------------------------------------------------------

#[test]
fn test_app_state_defaults() {
    let model = ModelInfo {
        provider: "openai".into(),
        id: "gpt-5.5".into(),
        display_name: "GPT-5.5".into(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    };
    let session = sample_session();
    let state = AppState::new(
        session,
        default_config(),
        model.clone(),
        ThinkingLevel::Medium,
        PathBuf::from("/workspace"),
    );

    assert_eq!(state.model.id, "gpt-5.5");
    assert_eq!(state.thinking_level, ThinkingLevel::Medium);
    assert_eq!(state.cwd, PathBuf::from("/workspace"));
}

// -----------------------------------------------------------------------
// Propagation: CancellationToken can be passed and checked in spawned tasks
// -----------------------------------------------------------------------

#[test]
fn test_cancellation_stops_spawned_work() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            loop {
                if cancel_clone.is_cancelled() {
                    return "cancelled";
                }
                tokio::task::yield_now().await;
            }
        });

        // Yield to let the task start
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        cancel.cancel();
        let result = handle.await.unwrap();
        assert_eq!(result, "cancelled");
    });
}

#[test]
fn test_cancellation_does_not_affect_other_tokens() {
    let cancel_a = CancellationToken::new();
    let cancel_b = CancellationToken::new();

    cancel_a.cancel();

    assert!(cancel_a.is_cancelled());
    assert!(!cancel_b.is_cancelled());
}

#[test]
fn test_cancellation_of_same_token_twice_is_idempotent() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    cancel.cancel(); // second cancel should be a no-op
    assert!(cancel.is_cancelled());
}
