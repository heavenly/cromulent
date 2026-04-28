use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ModelInfo, ThinkingLevel};
use crate::session::store::LoadedSessionState;

/// Global application configuration loaded from config file + CLI args
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub max_turns: u32,
    pub sessions_dir: PathBuf,
    pub default_model: ModelInfo,
    pub default_thinking: ThinkingLevel,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_turns: 40,
            sessions_dir: crate::util::fs::default_sessions_dir(),
            default_model: ModelInfo {
                provider: "openai".to_string(),
                id: "gpt-5.5".to_string(),
                display_name: "GPT-5.5".to_string(),
                context_window: 200_000,
                supports_reasoning: true,
                supports_tools: true,
            },
            default_thinking: ThinkingLevel::Medium,
        }
    }
}

/// Tracks whether a run is active
#[derive(Debug, Clone)]
pub enum RunState {
    Idle,
    Running {
        run_id: String,
        cancel: CancellationToken,
        started_at: chrono::DateTime<chrono::Utc>,
    },
}

/// The single authoritative mutable state of the application.
/// Only AppRuntime may mutate this.
#[derive(Debug)]
pub struct AppState {
    pub current_session: LoadedSessionState,
    pub model: ModelInfo,
    pub thinking_level: ThinkingLevel,
    pub cwd: PathBuf,
    pub run_state: RunState,
    pub config: AppConfig,
}

impl AppState {
    pub fn new(
        session: LoadedSessionState,
        config: AppConfig,
        model: ModelInfo,
        thinking: ThinkingLevel,
        cwd: PathBuf,
    ) -> Self {
        Self {
            current_session: session,
            model,
            thinking_level: thinking,
            cwd,
            run_state: RunState::Idle,
            config,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run_state, RunState::Running { .. })
    }

    pub fn run_id(&self) -> Option<&str> {
        match &self.run_state {
            RunState::Running { run_id, .. } => Some(run_id.as_str()),
            RunState::Idle => None,
        }
    }

    pub fn cancel_token(&self) -> Option<CancellationToken> {
        match &self.run_state {
            RunState::Running { cancel, .. } => Some(cancel.clone()),
            RunState::Idle => None,
        }
    }
}

/// Thread-safe wrapper around AppState
pub type SharedAppState = Arc<Mutex<AppState>>;
