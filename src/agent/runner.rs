use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::prompt::{build_system_prompt, PromptContext};
use crate::agent::transcript;
use crate::app::output::emit_event;
use crate::protocol::events::ServerEvent;
use crate::protocol::types::{
    ContentBlock, LlmContentBlock, Message, ModelInfo, ProviderEvent, ProviderRequest,
    ThinkingLevel, ToolContext, ToolDefinition, UsageInfo,
};
use crate::providers::ProviderManager;
use crate::session::store::SessionStore;
use crate::tools::ask_user::AskManagerHandle;
use crate::tools::registry::ToolRegistry;
use crate::transport::writer::OutputItem;

/// Result returned by [`AgentRunner::run_prompt`].
#[derive(Debug)]
pub struct RunResult {
    /// Messages appended during this run (user message + assistant + tool results).
    pub messages: Vec<Message>,
    /// Reason the run stopped.
    pub stop_reason: String,
}

/// Runs the agent turn loop.
///
/// Owns the turn loop that:
/// 1. Builds a provider request from the transcript.
/// 2. Streams provider events and converts them to UI-friendly `ServerEvent`s.
/// 3. Executes tools when the provider emits tool calls.
/// 4. Appends messages to the transcript and persists them.
/// 5. Loops until the provider stops, the turn limit is reached, or cancellation.
pub struct AgentRunner;

impl AgentRunner {
    pub fn new() -> Self {
        Self
    }

    /// Execute one full prompt run.
    ///
    /// # Parameters
    /// - `messages`: current transcript messages (before appending the user message).
    /// - `model`: the model info for this run.
    /// - `thinking_level`: the thinking level.
    /// - `cwd`: working directory.
    /// - `session_store`: for persisting messages.
    /// - `session_id`: the active session ID.
    /// - `tool_registry`: registered tool definitions and executors.
    /// - `output_tx`: channel for emitting events and command responses.
    /// - `provider_manager`: resolves the LLM provider from `model.provider`.
    /// - `ask_manager`: handles blocking `ask_user` interactions.
    /// - `cancel`: cancellation token shared with the caller.
    /// - `max_turns`: maximum number of turns before forcing stop.
    pub async fn run_prompt(
        &self,
        mut messages: Vec<Message>,
        model: ModelInfo,
        thinking_level: ThinkingLevel,
        cwd: PathBuf,
        session_store: &SessionStore,
        session_id: &str,
        tool_registry: &ToolRegistry,
        output_tx: &mpsc::UnboundedSender<OutputItem>,
        provider_manager: &ProviderManager,
        ask_manager: &AskManagerHandle,
        cancel: CancellationToken,
        max_turns: u32,
        run_id: String,
    ) -> RunResult {
        // --- 1. Append user message -------------------------------------------------
        // The caller should have already appended the user message. We just ensure
        // it's present. For now we trust the caller — no double-append.

        // --- 2. Build system prompt and tool defs -----------------------------------
        let tool_defs: Vec<ToolDefinition> = tool_registry.definitions().to_vec();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let system_prompt = build_system_prompt(&PromptContext {
            cwd: cwd.to_string_lossy().to_string(),
            date,
            tools: tool_defs.clone(),
        });

        // --- 3. Resolve provider ---------------------------------------------------
        let provider = match provider_manager.get(&model) {
            Ok(p) => p,
            Err(e) => {
                emit_event(
                    output_tx,
                    ServerEvent::AgentStart {
                        run_id: run_id.clone(),
                    },
                );
                emit_event(
                    output_tx,
                    ServerEvent::Error {
                        run_id: run_id.clone(),
                        message: format!("Provider error: {e}"),
                    },
                );
                emit_event(
                    output_tx,
                    ServerEvent::AgentEnd {
                        run_id: run_id.clone(),
                        stop_reason: "error".to_string(),
                    },
                );
                return RunResult {
                    messages,
                    stop_reason: "error".to_string(),
                };
            }
        };

        // --- 4. Emit agent_start ---------------------------------------------------
        emit_event(
            output_tx,
            ServerEvent::AgentStart {
                run_id: run_id.clone(),
            },
        );

        // --- 5. Turn loop ----------------------------------------------------------
        let mut stop_reason = "completed".to_string();

        for turn in 1..=max_turns {
            // Check cancellation before each turn
            if cancel.is_cancelled() {
                stop_reason = "aborted".to_string();
                break;
            }

            emit_event(
                output_tx,
                ServerEvent::TurnStart {
                    run_id: run_id.clone(),
                    turn,
                },
            );

            // Convert messages to LlmMessage format (exclude system messages)
            let llm_messages = transcript::messages_to_llm(&messages);

            let request = ProviderRequest {
                model: model.clone(),
                system_prompt: system_prompt.clone(),
                messages: llm_messages,
                tools: tool_defs.clone(),
                thinking_level: thinking_level.clone(),
                cwd: cwd.clone(),
            };

            // Stream from provider
            let mut rx = match provider.stream(request, cancel.clone()).await {
                Ok(rx) => rx,
                Err(e) => {
                    emit_event(
                        output_tx,
                        ServerEvent::Error {
                            run_id: run_id.clone(),
                            message: format!("Provider stream failed: {e}"),
                        },
                    );
                    stop_reason = "error".to_string();
                    emit_event(
                        output_tx,
                        ServerEvent::TurnEnd {
                            run_id: run_id.clone(),
                            turn,
                            stop_reason: stop_reason.clone(),
                            usage: None,
                        },
                    );
                    break;
                }
            };

            // --- 5a. Consume provider events ---------------------------------------
            let mut assistant_text: Option<String> = None;
            let mut assistant_thinking: Option<String> = None;
            let mut assistant_tool_calls: Vec<LlmContentBlock> = Vec::new();
            let mut usage: Option<UsageInfo> = None;
            let mut turn_error: Option<String> = None;

            // For accumulating tool call arguments across deltas
            let mut tool_call_buffers: std::collections::HashMap<String, (String, String, String)> =
                std::collections::HashMap::new();
            // Maps call_id -> (name, accumulated_args_json)

            while let Some(event) = rx.recv().await {
                if cancel.is_cancelled() {
                    // Drain but stop processing
                    continue;
                }

                match event {
                    ProviderEvent::TextDelta { text } => {
                        let current = assistant_text.get_or_insert_with(String::new);
                        current.push_str(&text);
                        emit_event(
                            output_tx,
                            ServerEvent::TextDelta {
                                run_id: run_id.clone(),
                                text: text.clone(),
                                partial: current.clone(),
                            },
                        );
                    }

                    ProviderEvent::ThinkingDelta { text } => {
                        let current = assistant_thinking.get_or_insert_with(String::new);
                        current.push_str(&text);
                        emit_event(
                            output_tx,
                            ServerEvent::ThinkingDelta {
                                run_id: run_id.clone(),
                                text: text.clone(),
                                partial: current.clone(),
                            },
                        );
                    }

                    ProviderEvent::ThinkingEnd => {
                        emit_event(
                            output_tx,
                            ServerEvent::ThinkingEnd {
                                run_id: run_id.clone(),
                            },
                        );
                    }

                    ProviderEvent::ToolCallStarted { id, name } => {
                        tool_call_buffers.insert(id.clone(), (name, String::new(), id.clone()));
                        // Don't emit a separate ToolCall event yet — wait for
                        // completion when we have the full arguments
                    }

                    ProviderEvent::ToolCallArgumentsDelta { id, delta } => {
                        if let Some(entry) = tool_call_buffers.get_mut(&id) {
                            entry.1.push_str(&delta);
                        }
                    }

                    ProviderEvent::ToolCallCompleted { id } => {
                        if let Some((name, args_json, _)) = tool_call_buffers.remove(&id) {
                            // Parse accumulated JSON arguments
                            let args: serde_json::Value =
                                serde_json::from_str(&args_json).unwrap_or(serde_json::Value::Null);

                            // Emit the tool call event with final arguments
                            emit_event(
                                output_tx,
                                ServerEvent::ToolCall {
                                    run_id: run_id.clone(),
                                    id: id.clone(),
                                    name: name.clone(),
                                    arguments: args.clone(),
                                },
                            );

                            assistant_tool_calls.push(LlmContentBlock::ToolCall {
                                id,
                                name,
                                arguments: args,
                            });
                        }
                    }

                    ProviderEvent::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        usage = Some(UsageInfo {
                            input_tokens,
                            output_tokens,
                        });
                    }

                    ProviderEvent::Completed => {
                        // Provider is done — stop consuming
                        break;
                    }

                    ProviderEvent::Error { message } => {
                        turn_error = Some(message);
                        break;
                    }
                }
            }

            // If cancelled during streaming
            if cancel.is_cancelled() {
                stop_reason = "aborted".to_string();
                emit_event(
                    output_tx,
                    ServerEvent::TurnEnd {
                        run_id: run_id.clone(),
                        turn,
                        stop_reason: stop_reason.clone(),
                        usage: None,
                    },
                );
                break;
            }

            // Handle provider errors
            if let Some(err_msg) = turn_error {
                emit_event(
                    output_tx,
                    ServerEvent::Error {
                        run_id: run_id.clone(),
                        message: err_msg.clone(),
                    },
                );
                // Even on error, we try to persist what we have
                let has_content = assistant_text.as_ref().is_some_and(|s| !s.is_empty())
                    || assistant_thinking.as_ref().is_some_and(|s| !s.is_empty())
                    || !assistant_tool_calls.is_empty();
                if has_content {
                    let assistant_msg = transcript::new_assistant_message_with_thinking(
                        assistant_text,
                        assistant_thinking,
                        std::mem::take(&mut assistant_tool_calls),
                    );
                    let _ = session_store
                        .append_message(session_id, &assistant_msg)
                        .await;
                    messages.push(assistant_msg);
                }
                stop_reason = "error".to_string();
                emit_event(
                    output_tx,
                    ServerEvent::TurnEnd {
                        run_id: run_id.clone(),
                        turn,
                        stop_reason: stop_reason.clone(),
                        usage,
                    },
                );
                break;
            }

            // --- 5b. Build and persist assistant message ---------------------------
            let assistant_msg = transcript::new_assistant_message_with_thinking(
                assistant_text.clone(),
                assistant_thinking.clone(),
                assistant_tool_calls.clone(),
            );

            let has_tool_calls = !assistant_tool_calls.is_empty();
            let has_text = assistant_text.as_ref().is_some_and(|s| !s.is_empty());
            let has_thinking = assistant_thinking.as_ref().is_some_and(|s| !s.is_empty());

            // Only persist assistant message if there's content, thinking, or tool calls
            if has_text || has_thinking || has_tool_calls {
                let _ = session_store
                    .append_message(session_id, &assistant_msg)
                    .await;
                messages.push(assistant_msg);
            }

            // --- 5c. Execute tool calls if any ------------------------------------
            if !assistant_tool_calls.is_empty() {
                for tc in &assistant_tool_calls {
                    match tc {
                        LlmContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            // Build tool context
                            let tool_ctx = ToolContext {
                                cwd: cwd.clone(),
                                run_id: run_id.clone(),
                                event_tx: output_tx.clone(),
                                ask_manager: ask_manager.clone(),
                            };

                            // Execute the tool
                            let result = tool_registry
                                .execute(name, tool_ctx, arguments.clone(), cancel.clone())
                                .await;

                            let (content_text, is_error, result_metadata) = match result {
                                Ok(tool_result) => {
                                    // Flatten content to text for the transcript
                                    let text: String = tool_result
                                        .content
                                        .iter()
                                        .filter_map(|b| match b {
                                            ContentBlock::Text { text } => Some(text.as_str()),
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");

                                    let is_err = tool_result.is_error;
                                    let metadata = tool_result.metadata.clone();

                                    // Emit tool result event
                                    emit_event(
                                        output_tx,
                                        ServerEvent::ToolResult {
                                            run_id: run_id.clone(),
                                            tool_call_id: id.clone(),
                                            content: vec![ContentBlock::Text {
                                                text: text.clone(),
                                            }],
                                            is_error: is_err,
                                            metadata: metadata.clone(),
                                        },
                                    );

                                    (text, is_err, metadata)
                                }
                                Err(e) => {
                                    let err_text = format!("Tool execution error: {e}");

                                    emit_event(
                                        output_tx,
                                        ServerEvent::ToolResult {
                                            run_id: run_id.clone(),
                                            tool_call_id: id.clone(),
                                            content: vec![ContentBlock::Text {
                                                text: err_text.clone(),
                                            }],
                                            is_error: true,
                                            metadata: None,
                                        },
                                    );

                                    (err_text, true, None)
                                }
                            };

                            // Create and persist tool result message
                            let tool_msg = transcript::new_tool_result_message(
                                id,
                                name,
                                content_text,
                                is_error,
                                result_metadata,
                            );
                            let _ = session_store.append_message(session_id, &tool_msg).await;
                            messages.push(tool_msg);
                        }
                        _ => {}
                    }
                }

                // Turn ends with tool calls — continue to next turn
                emit_event(
                    output_tx,
                    ServerEvent::TurnEnd {
                        run_id: run_id.clone(),
                        turn,
                        stop_reason: "tool_calls".to_string(),
                        usage,
                    },
                );

                // Continue to next turn
                continue;
            }

            // --- 5d. No tool calls — turn completed --------------------------------
            let turn_stop_reason = if cancel.is_cancelled() {
                "aborted"
            } else {
                "completed"
            };

            emit_event(
                output_tx,
                ServerEvent::TurnEnd {
                    run_id: run_id.clone(),
                    turn,
                    stop_reason: turn_stop_reason.to_string(),
                    usage,
                },
            );

            stop_reason = turn_stop_reason.to_string();
            break;
        }

        // --- 6. Emit agent_end ----------------------------------------------------
        if cancel.is_cancelled() {
            stop_reason = "aborted".to_string();
        }

        emit_event(
            output_tx,
            ServerEvent::AgentEnd {
                run_id: run_id.clone(),
                stop_reason: stop_reason.clone(),
            },
        );

        RunResult {
            messages,
            stop_reason,
        }
    }
}
