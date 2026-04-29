use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::runner::AgentRunner;
use crate::app::output::{emit_event, respond};
use crate::app::state::{RunState, SharedAppState};
use crate::auth::config::AppConfigFile;
use crate::process::bash_runner::BashRunner;
use crate::protocol::events::ServerEvent;
use crate::protocol::responses::{CommandResponse, StateSnapshot};
use crate::protocol::types::{AskUserResponse, ModelInfo, ThinkingLevel};
use crate::providers::ProviderManager;
use crate::session::store::{SessionHeader, SessionStore};
use crate::tools::ask_user::AskManagerHandle;
use crate::tools::registry::ToolRegistry;
use crate::transport::writer::OutputItem;
use crate::util::ids::{generate_run_id, generate_session_id};
use crate::util::time::now_iso;

/// The single authoritative runtime that owns state and orchestrates commands.
pub struct AppRuntime {
    pub state: SharedAppState,
    pub output_tx: mpsc::UnboundedSender<OutputItem>,
    pub session_store: SessionStore,
    pub tool_registry: Arc<ToolRegistry>,
    pub bash_runner: std::sync::Mutex<BashRunner>,
    pub bash_cancel: std::sync::Mutex<Option<CancellationToken>>,
    pub ask_manager: AskManagerHandle,
    pub config: AppConfigFile,
}

impl AppRuntime {
    pub fn new(
        state: SharedAppState,
        output_tx: mpsc::UnboundedSender<OutputItem>,
        session_store: SessionStore,
        tool_registry: ToolRegistry,
        bash_runner: BashRunner,
        ask_manager: AskManagerHandle,
        config: AppConfigFile,
    ) -> Self {
        Self {
            state,
            output_tx,
            session_store,
            tool_registry: Arc::new(tool_registry),
            bash_runner: std::sync::Mutex::new(bash_runner),
            bash_cancel: std::sync::Mutex::new(None),
            ask_manager,
            config,
        }
    }

    // -----------------------------------------------------------------------
    // Command handlers
    // -----------------------------------------------------------------------

    /// Handle a `prompt` command.
    /// Validates idle, appends the user message, responds immediately, then
    /// spawns the provider-neutral agent loop in the background.
    pub async fn handle_prompt(&self, id: Option<String>, message: String) {
        let run_id = generate_run_id();
        let cancel = CancellationToken::new();
        let user_msg = crate::agent::transcript::new_user_message(message);

        let (messages, model, thinking_level, cwd, session_id, max_turns) = {
            let mut state = self.state.lock().await;
            if state.is_running() {
                respond(
                    &self.output_tx,
                    CommandResponse::err(
                        id,
                        "A run is already in progress. Abort first or wait for it to complete.",
                    ),
                );
                return;
            }

            state.run_state = RunState::Running {
                run_id: run_id.clone(),
                cancel: cancel.clone(),
                started_at: chrono::Utc::now(),
            };

            state.current_session.messages.push(user_msg.clone());
            (
                state.current_session.messages.clone(),
                state.model.clone(),
                state.thinking_level.clone(),
                state.cwd.clone(),
                state.current_session.header.session_id.clone(),
                state.config.max_turns,
            )
        };

        if let Err(e) = self
            .session_store
            .append_message(&session_id, &user_msg)
            .await
        {
            tracing::error!("Failed to persist user message: {e}");
        }

        respond(
            &self.output_tx,
            CommandResponse::ok_with_data(id, serde_json::json!({ "runId": run_id })),
        );

        let state = self.state.clone();
        let output_tx = self.output_tx.clone();
        let session_store = self.session_store.clone();
        let tool_registry = self.tool_registry.clone();
        let ask_manager = self.ask_manager.clone();
        let config = self.config.clone();
        tokio::spawn(async move {
            let runner = AgentRunner::new();
            let provider_manager = ProviderManager::default_with_config(&config);
            let result = runner
                .run_prompt(
                    messages,
                    model,
                    thinking_level,
                    cwd,
                    &session_store,
                    &session_id,
                    &tool_registry,
                    &output_tx,
                    &provider_manager,
                    &ask_manager,
                    cancel,
                    max_turns,
                    run_id.clone(),
                )
                .await;

            let mut state = state.lock().await;
            state.current_session.messages = result.messages;
            if matches!(&state.run_state, RunState::Running { run_id: active, .. } if active == &run_id)
            {
                state.run_state = RunState::Idle;
            }
        });
    }

    /// Handle an `abort` command.
    pub async fn handle_abort(&self, id: Option<String>) {
        if let Some(cancel) = self.bash_cancel.lock().unwrap().take() {
            cancel.cancel();
        }

        let (run_id, cancel) = {
            let state = self.state.lock().await;
            match &state.run_state {
                RunState::Running { run_id, cancel, .. } => {
                    (Some(run_id.clone()), Some(cancel.clone()))
                }
                RunState::Idle => (None, None),
            }
        };

        if let Some(cancel) = cancel {
            cancel.cancel();
            // Emit abort lifecycle events
            if let Some(run_id) = &run_id {
                emit_event(
                    &self.output_tx,
                    ServerEvent::Error {
                        run_id: run_id.clone(),
                        message: "Run aborted by user".to_string(),
                    },
                );
                emit_event(
                    &self.output_tx,
                    ServerEvent::AgentEnd {
                        run_id: run_id.clone(),
                        stop_reason: "aborted".to_string(),
                    },
                );
            }
            self.ask_manager.cancel_all().await;
            let mut state = self.state.lock().await;
            state.run_state = RunState::Idle;
        }

        respond(&self.output_tx, CommandResponse::ok(id));
    }

    /// Handle a `user_response` command — resolve a pending ask.
    pub async fn handle_user_response(
        &self,
        id: Option<String>,
        ask_id: String,
        response: AskUserResponse,
    ) {
        match self.ask_manager.resolve(&ask_id, response).await {
            Ok(()) => respond(&self.output_tx, CommandResponse::ok(id)),
            Err(e) => respond(&self.output_tx, CommandResponse::err(id, e)),
        }
    }

    /// Handle `set_model` — allowed only when idle.
    pub async fn handle_set_model(&self, id: Option<String>, provider: String, model_id: String) {
        let is_running = {
            let state = self.state.lock().await;
            state.is_running()
        };

        if is_running {
            respond(
                &self.output_tx,
                CommandResponse::err(
                    id,
                    "Cannot change model while a run is active. Abort first.",
                ),
            );
            return;
        }

        let new_model = ModelInfo {
            provider,
            id: model_id,
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        };

        let session_id;
        let cwd;
        let thinking_level;
        {
            let mut state = self.state.lock().await;
            state.model = new_model.clone();
            state.current_session.header.model = new_model.clone();
            state.current_session.header.updated = now_iso();
            session_id = state.current_session.header.session_id.clone();
            cwd = state.cwd.to_string_lossy().to_string();
            thinking_level = state.thinking_level.clone();

            // Persist header update
            if let Err(e) = self
                .session_store
                .update_header(&state.current_session.header)
                .await
            {
                tracing::error!("Failed to update session header: {e}");
            }
        }

        emit_event(
            &self.output_tx,
            ServerEvent::SessionChanged {
                session_id,
                cwd,
                model: new_model,
                thinking_level,
            },
        );

        respond(&self.output_tx, CommandResponse::ok(id));
    }

    /// Handle `set_thinking` — allowed only when idle.
    pub async fn handle_set_thinking(&self, id: Option<String>, level: ThinkingLevel) {
        let is_running = {
            let state = self.state.lock().await;
            state.is_running()
        };

        if is_running {
            respond(
                &self.output_tx,
                CommandResponse::err(
                    id,
                    "Cannot change thinking level while a run is active. Abort first.",
                ),
            );
            return;
        }

        let session_id;
        let cwd;
        let model;
        {
            let mut state = self.state.lock().await;
            state.thinking_level = level.clone();
            state.current_session.header.thinking_level = level.clone();
            state.current_session.header.updated = now_iso();
            session_id = state.current_session.header.session_id.clone();
            cwd = state.cwd.to_string_lossy().to_string();
            model = state.model.clone();

            if let Err(e) = self
                .session_store
                .update_header(&state.current_session.header)
                .await
            {
                tracing::error!("Failed to update session header: {e}");
            }
        }

        emit_event(
            &self.output_tx,
            ServerEvent::SessionChanged {
                session_id,
                cwd,
                model,
                thinking_level: level,
            },
        );

        respond(&self.output_tx, CommandResponse::ok(id));
    }

    /// Handle `cycle_model` — cycle through a small built-in model list.
    pub async fn handle_cycle_model(&self, id: Option<String>) {
        let is_running = {
            let state = self.state.lock().await;
            state.is_running()
        };

        if is_running {
            respond(
                &self.output_tx,
                CommandResponse::err(id, "Cannot cycle model while a run is active. Abort first."),
            );
            return;
        }

        let models = vec![
            ModelInfo {
                provider: "fake".to_string(),
                id: "default".to_string(),
                display_name: "Fake".to_string(),
                context_window: 128_000,
                supports_reasoning: false,
                supports_tools: true,
            },
            ModelInfo {
                provider: "openai".to_string(),
                id: "gpt-5.5".to_string(),
                display_name: "GPT-5.5".to_string(),
                context_window: 200_000,
                supports_reasoning: true,
                supports_tools: true,
            },
            ModelInfo {
                provider: "openai".to_string(),
                id: "gpt-5-codex".to_string(),
                display_name: "GPT-5 Codex".to_string(),
                context_window: 200_000,
                supports_reasoning: true,
                supports_tools: true,
            },
            ModelInfo {
                provider: "deepseek".to_string(),
                id: "deepseek-v4-flash".to_string(),
                display_name: "DeepSeek V4 Flash".to_string(),
                context_window: 128_000,
                supports_reasoning: true,
                supports_tools: true,
            },
        ];

        let (new_model, session_id, cwd, thinking_level, header) = {
            let mut state = self.state.lock().await;
            let current_idx = models
                .iter()
                .position(|m| m.provider == state.model.provider && m.id == state.model.id)
                .unwrap_or(0);
            let next = models[(current_idx + 1) % models.len()].clone();
            state.model = next.clone();
            state.current_session.header.model = next.clone();
            state.current_session.header.updated = now_iso();
            (
                next,
                state.current_session.header.session_id.clone(),
                state.cwd.to_string_lossy().to_string(),
                state.thinking_level.clone(),
                state.current_session.header.clone(),
            )
        };

        if let Err(e) = self.session_store.update_header(&header).await {
            respond(
                &self.output_tx,
                CommandResponse::err(id, format!("Failed to update session header: {e}")),
            );
            return;
        }

        emit_event(
            &self.output_tx,
            ServerEvent::SessionChanged {
                session_id,
                cwd,
                model: new_model.clone(),
                thinking_level,
            },
        );

        respond(
            &self.output_tx,
            CommandResponse::ok_with_data(id, serde_json::json!({ "model": new_model })),
        );
    }

    /// Handle `list_sessions`.
    pub async fn handle_list_sessions(&self, id: Option<String>) {
        match self.session_store.list_session_headers().await {
            Ok(sessions) => {
                let sessions: Vec<serde_json::Value> = sessions
                    .into_iter()
                    .map(|(sid, updated)| {
                        serde_json::json!({"sessionId": sid, "updated": updated})
                    })
                    .collect();
                let data = serde_json::json!({ "sessions": sessions });
                respond(&self.output_tx, CommandResponse::ok_with_data(id, data));
            }
            Err(e) => {
                respond(
                    &self.output_tx,
                    CommandResponse::err(id, format!("Failed to list sessions: {e}")),
                );
            }
        }
    }

    /// Handle `load_session` — allowed only when idle.
    pub async fn handle_load_session(&self, id: Option<String>, session_id: String) {
        let is_running = {
            let state = self.state.lock().await;
            state.is_running()
        };

        if is_running {
            respond(
                &self.output_tx,
                CommandResponse::err(
                    id,
                    "Cannot load session while a run is active. Abort first.",
                ),
            );
            return;
        }

        match self.session_store.load_session(&session_id).await {
            Ok(loaded) => {
                // If the current session is trivial (auto-created, no messages),
                // delete it to avoid accumulating empty session files.
                {
                    let state = self.state.lock().await;
                    if state.current_session.messages.is_empty() {
                        let old_id = state.current_session.header.session_id.clone();
                        drop(state);
                        if let Err(e) = self.session_store.delete_session(&old_id).await {
                            tracing::warn!(session_id = %old_id, error = %e, "Failed to delete trivial session");
                        }
                    }
                }
                let model;
                let thinking_level;
                let cwd;
                let sid;
                {
                    let mut state = self.state.lock().await;
                    state.current_session = loaded;
                    state.model = state.current_session.header.model.clone();
                    state.thinking_level = state.current_session.header.thinking_level.clone();
                    state.cwd = std::path::PathBuf::from(&state.current_session.header.cwd);
                    self.bash_runner.lock().unwrap().set_cwd(state.cwd.clone());
                    model = state.model.clone();
                    thinking_level = state.thinking_level.clone();
                    cwd = state.cwd.to_string_lossy().to_string();
                    sid = state.current_session.header.session_id.clone();
                }

                emit_event(
                    &self.output_tx,
                    ServerEvent::SessionChanged {
                        session_id: sid,
                        cwd,
                        model,
                        thinking_level,
                    },
                );

                respond(&self.output_tx, CommandResponse::ok(id));
            }
            Err(e) => {
                respond(
                    &self.output_tx,
                    CommandResponse::err(id, format!("Failed to load session: {e}")),
                );
            }
        }
    }

    /// Handle `new_session` — allowed only when idle.
    pub async fn handle_new_session(&self, id: Option<String>) {
        let (is_running, _old_session_id, old_model, old_thinking, old_cwd) = {
            let state = self.state.lock().await;
            (
                state.is_running(),
                state.current_session.header.session_id.clone(),
                state.model.clone(),
                state.thinking_level.clone(),
                state.cwd.to_string_lossy().to_string(),
            )
        };

        if is_running {
            respond(
                &self.output_tx,
                CommandResponse::err(
                    id,
                    "Cannot create new session while a run is active. Abort first.",
                ),
            );
            return;
        }

        let new_session_id = generate_session_id();
        let new_header = SessionHeader::new(
            new_session_id.clone(),
            old_cwd.clone(),
            old_model.clone(),
            old_thinking.clone(),
        );

        if let Err(e) = self.session_store.create_session(&new_header).await {
            respond(
                &self.output_tx,
                CommandResponse::err(id, format!("Failed to create session: {e}")),
            );
            return;
        }

        let new_loaded = crate::session::store::LoadedSessionState {
            header: new_header,
            messages: Vec::new(),
        };

        {
            let mut state = self.state.lock().await;
            state.current_session = new_loaded;
        }

        emit_event(
            &self.output_tx,
            ServerEvent::SessionChanged {
                session_id: new_session_id.clone(),
                cwd: old_cwd.clone(),
                model: old_model.clone(),
                thinking_level: old_thinking.clone(),
            },
        );

        respond(
            &self.output_tx,
            CommandResponse::ok_with_data(id, serde_json::json!({ "sessionId": new_session_id })),
        );
    }

    /// Handle `fork_session` — allowed only when idle.
    pub async fn handle_fork_session(&self, id: Option<String>, entry_id: String) {
        let (source_session_id, model, thinking, cwd) = {
            let state = self.state.lock().await;
            if state.is_running() {
                respond(
                    &self.output_tx,
                    CommandResponse::err(
                        id,
                        "Cannot fork session while a run is active. Abort first.",
                    ),
                );
                return;
            }
            (
                state.current_session.header.session_id.clone(),
                state.model.clone(),
                state.thinking_level.clone(),
                state.cwd.to_string_lossy().to_string(),
            )
        };
        let new_session_id = generate_session_id();
        let new_header = SessionHeader::new(
            new_session_id.clone(),
            cwd.clone(),
            model.clone(),
            thinking.clone(),
        );

        match self
            .session_store
            .fork_session(&source_session_id, &entry_id, &new_header)
            .await
        {
            Ok(loaded) => {
                {
                    let mut state = self.state.lock().await;
                    state.current_session = loaded;
                }

                emit_event(
                    &self.output_tx,
                    ServerEvent::SessionChanged {
                        session_id: new_session_id.clone(),
                        cwd,
                        model,
                        thinking_level: thinking,
                    },
                );

                respond(
                    &self.output_tx,
                    CommandResponse::ok_with_data(
                        id,
                        serde_json::json!({ "sessionId": new_session_id }),
                    ),
                );
            }
            Err(e) => {
                respond(
                    &self.output_tx,
                    CommandResponse::err(id, format!("Failed to fork session: {e}")),
                );
            }
        }
    }

    /// Handle `get_state`.
    pub async fn handle_get_state(&self, id: Option<String>) {
        let state = self.state.lock().await;
        let snapshot = StateSnapshot {
            model: state.model.clone(),
            thinking_level: state.thinking_level.clone(),
            session_id: state.current_session.header.session_id.clone(),
            cwd: state.cwd.to_string_lossy().to_string(),
            message_count: state.current_session.messages.len(),
            is_streaming: state.is_running(),
            run_id: state.run_id().map(|s| s.to_string()),
        };
        drop(state);

        respond(
            &self.output_tx,
            CommandResponse::ok_with_data(
                id,
                serde_json::to_value(snapshot).expect("StateSnapshot serialization failed"),
            ),
        );
    }

    /// Handle `get_messages`.
    pub async fn handle_get_messages(&self, id: Option<String>) {
        let state = self.state.lock().await;
        let messages = state.current_session.messages.clone();
        drop(state);

        let data = serde_json::json!({ "messages": messages });
        respond(&self.output_tx, CommandResponse::ok_with_data(id, data));
    }

    /// Handle `export_session`.
    pub async fn handle_export_session(&self, id: Option<String>, output_path: String) {
        let state = self.state.lock().await;
        let header = state.current_session.header.clone();
        let messages = state.current_session.messages.clone();
        drop(state);

        let export = serde_json::json!({
            "schemaVersion": 1,
            "header": header,
            "messages": messages,
        });

        let path = std::path::Path::new(&output_path);
        match tokio::fs::write(path, serde_json::to_string_pretty(&export).unwrap()).await {
            Ok(()) => respond(&self.output_tx, CommandResponse::ok(id)),
            Err(e) => respond(
                &self.output_tx,
                CommandResponse::err(id, format!("Failed to export session: {e}")),
            ),
        }
    }

    /// Handle `bash` command — spawn a raw shell process and stream output.
    pub async fn handle_bash(&self, id: Option<String>, command: String) {
        let cancel = CancellationToken::new();
        {
            let mut slot = self.bash_cancel.lock().unwrap();
            if slot.is_some() {
                respond(
                    &self.output_tx,
                    CommandResponse::err(id, "A raw bash command is already running."),
                );
                return;
            }
            *slot = Some(cancel.clone());
        }

        let result = {
            let runner = self.bash_runner.lock().unwrap().clone();
            runner
                .execute_with_cancel(&command, &self.output_tx, cancel)
                .await
        };

        self.bash_cancel.lock().unwrap().take();

        match result {
            Ok(exit_code) => {
                respond(
                    &self.output_tx,
                    CommandResponse::ok_with_data(id, serde_json::json!({ "exitCode": exit_code })),
                );
            }
            Err(e) => {
                respond(
                    &self.output_tx,
                    CommandResponse::err(id, format!("Bash execution failed: {e}")),
                );
            }
        }
    }

    /// Handle an unrecognized command type.
    pub async fn handle_invalid_command(&self) {
        respond(
            &self.output_tx,
            CommandResponse::err(None, "Unrecognized command type"),
        );
    }

    /// Handle `shutdown`.
    pub async fn handle_shutdown(&self, id: Option<String>) {
        // Cancel any active run
        {
            let state = self.state.lock().await;
            if let Some(cancel) = state.cancel_token() {
                cancel.cancel();
            }
        }

        respond(&self.output_tx, CommandResponse::ok(id));
    }
}
