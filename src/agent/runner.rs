use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::prompt::{build_system_prompt, PromptContext};
use crate::agent::transcript;
use crate::app::output::emit_event;
use crate::protocol::events::ServerEvent;
use crate::protocol::types::{
    ContentBlock, LlmContentBlock, LlmMessage, Message, ModelInfo, ProviderEvent, ProviderRequest,
    ThinkingLevel, ToolContext, ToolDefinition, UsageInfo,
};
use crate::providers::{LlmProvider, ProviderManager};
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

impl Default for AgentRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRunner {
    pub fn new() -> Self {
        Self
    }

    /// Execute one full prompt run.
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
        provider_manager: &Arc<ProviderManager>,
        ask_manager: &AskManagerHandle,
        cancel: CancellationToken,
        max_turns: u32,
        run_id: String,
    ) -> RunResult {
        let run_id = Arc::from(run_id);

        // --- Build system prompt and tool defs once -------------------------------
        let tool_defs = tool_registry.definitions_arc();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let system_prompt = build_system_prompt(&PromptContext {
            cwd: cwd.to_string_lossy().to_string(),
            date,
            tools: (*tool_defs).clone(),
        });

        // --- Resolve provider ----------------------------------------------------
        let provider = match provider_manager.get(&model) {
            Ok(p) => p,
            Err(e) => {
                emit_event_with_agent_start(output_tx, &run_id);
                emit_event(
                    output_tx,
                    ServerEvent::Error {
                        run_id: run_id.to_string(),
                        message: format!("Provider error: {e}"),
                    },
                );
                emit_agent_end(output_tx, &run_id, "error");
                return RunResult {
                    messages,
                    stop_reason: "error".to_string(),
                };
            }
        };

        emit_event_with_agent_start(output_tx, &run_id);

        // --- Pre-compute LLM-formatted messages once ------------------------------
        let mut llm_messages = transcript::messages_to_llm(&messages);
        let mut converted_count = messages.len();

        let ctx = RunContext {
            run_id: &run_id,
            system_prompt: &system_prompt,
            tool_defs: &tool_defs,
            model: &model,
            thinking_level: &thinking_level,
            cwd: &cwd,
            session_store,
            session_id,
            tool_registry,
            output_tx,
            ask_manager,
            provider,
        };

        let mut stop_reason = "completed".to_string();

        for turn in 1..=max_turns {
            if cancel.is_cancelled() {
                stop_reason = "aborted".to_string();
                break;
            }

            emit_event(
                output_tx,
                ServerEvent::TurnStart {
                    run_id: run_id.to_string(),
                    turn,
                },
            );

            // Convert any new messages since last turn (appended by run_turn)
            if converted_count < messages.len() {
                let new_llm = transcript::messages_to_llm(&messages[converted_count..]);
                llm_messages.extend(new_llm);
                converted_count = messages.len();
            }

            match self
                .run_turn(
                    &ctx,
                    &mut messages,
                    &llm_messages,
                    turn,
                    &stop_reason,
                    &cancel,
                )
                .await
            {
                TurnOutcome::Continue => continue,
                TurnOutcome::Stop { reason } => {
                    stop_reason = reason;
                    break;
                }
            }
        }

        // --- Emit agent_end -----------------------------------------------------
        if cancel.is_cancelled() {
            stop_reason = "aborted".to_string();
        }

        emit_agent_end(output_tx, &run_id, &stop_reason);

        RunResult {
            messages,
            stop_reason,
        }
    }

    /// Execute one turn of the agent loop.
    /// `llm_messages` is the pre-computed LLM-form transcript (incrementally built).
    async fn run_turn(
        &self,
        ctx: &RunContext<'_>,
        messages: &mut Vec<Message>,
        llm_messages: &[LlmMessage],
        turn: u32,
        _stop_reason: &str,
        cancel: &CancellationToken,
    ) -> TurnOutcome {
        // Apply transcript compaction to stay within context window limits.
        // Full transcript is preserved on disk; only the LLM request is compacted.
        let compact_messages = crate::agent::compaction::compact(llm_messages);

        let request = ProviderRequest {
            model: ctx.model.clone(),
            system_prompt: ctx.system_prompt.to_string(),
            messages: compact_messages,
            tools: (**ctx.tool_defs).clone(),
            thinking_level: ctx.thinking_level.clone(),
        };

        let mut rx = match ctx.provider.stream(request, cancel.clone()).await {
            Ok(rx) => rx,
            Err(e) => {
                emit_event(
                    ctx.output_tx,
                    ServerEvent::Error {
                        run_id: ctx.run_id.to_string(),
                        message: format!("Provider stream failed: {e}"),
                    },
                );
                emit_event(
                    ctx.output_tx,
                    ServerEvent::TurnEnd {
                        run_id: ctx.run_id.to_string(),
                        turn,
                        stop_reason: "error".to_string(),
                        usage: None,
                    },
                );
                return TurnOutcome::Stop {
                    reason: "error".to_string(),
                };
            }
        };

        let stream_result = self.consume_provider_events(ctx, &mut rx, cancel).await;

        // Check for cancellation during streaming
        if cancel.is_cancelled() {
            emit_event(
                ctx.output_tx,
                ServerEvent::TurnEnd {
                    run_id: ctx.run_id.to_string(),
                    turn,
                    stop_reason: "aborted".to_string(),
                    usage: None,
                },
            );
            return TurnOutcome::Stop {
                reason: "aborted".to_string(),
            };
        }

        // Handle provider error
        if let Some(ref err_msg) = stream_result.error {
            emit_event(
                ctx.output_tx,
                ServerEvent::Error {
                    run_id: ctx.run_id.to_string(),
                    message: err_msg.clone(),
                },
            );

            let has_content = stream_result.has_content();
            if has_content {
                let assistant_msg = transcript::new_assistant_message_with_thinking(
                    stream_result.text,
                    stream_result.thinking,
                    stream_result.tool_calls,
                );
                if let Err(e) = ctx
                    .session_store
                    .append_message(ctx.session_id, &assistant_msg)
                    .await
                {
                    tracing::error!("Failed to persist assistant message: {e}");
                }
                messages.push(assistant_msg);
            }

            emit_event(
                ctx.output_tx,
                ServerEvent::TurnEnd {
                    run_id: ctx.run_id.to_string(),
                    turn,
                    stop_reason: "error".to_string(),
                    usage: stream_result.usage,
                },
            );
            return TurnOutcome::Stop {
                reason: "error".to_string(),
            };
        }

        // Build and persist assistant message
        let assistant_msg = transcript::new_assistant_message_with_thinking(
            stream_result.text.clone(),
            stream_result.thinking.clone(),
            stream_result.tool_calls.clone(),
        );

        if stream_result.has_content() {
            if let Err(e) = ctx
                .session_store
                .append_message(ctx.session_id, &assistant_msg)
                .await
            {
                tracing::error!("Failed to persist assistant message: {e}");
            }
            messages.push(assistant_msg);
        }

        // Execute tool calls if any
        if !stream_result.tool_calls.is_empty() {
            self.execute_tool_calls(ctx, &stream_result.tool_calls, messages, cancel)
                .await;

            emit_event(
                ctx.output_tx,
                ServerEvent::TurnEnd {
                    run_id: ctx.run_id.to_string(),
                    turn,
                    stop_reason: "tool_calls".to_string(),
                    usage: stream_result.usage,
                },
            );
            return TurnOutcome::Continue;
        }

        // No tool calls — turn completed
        let reason = if cancel.is_cancelled() {
            "aborted"
        } else {
            "completed"
        };

        emit_event(
            ctx.output_tx,
            ServerEvent::TurnEnd {
                run_id: ctx.run_id.to_string(),
                turn,
                stop_reason: reason.to_string(),
                usage: stream_result.usage,
            },
        );

        TurnOutcome::Stop {
            reason: reason.to_string(),
        }
    }

    /// Consume provider events from the stream receiver.
    async fn consume_provider_events(
        &self,
        ctx: &RunContext<'_>,
        rx: &mut mpsc::UnboundedReceiver<ProviderEvent>,
        cancel: &CancellationToken,
    ) -> StreamResult {
        let mut result = StreamResult::default();
        let mut tool_call_buffers: HashMap<String, (String, String)> = HashMap::new();
        // Maps call_id -> (name, accumulated_args_json)

        while let Some(event) = rx.recv().await {
            if cancel.is_cancelled() {
                // Drain but stop processing
                continue;
            }

            match event {
                ProviderEvent::TextDelta { text } => {
                    let current = result.text.get_or_insert_with(String::new);
                    current.push_str(&text);
                    emit_event(
                        ctx.output_tx,
                        ServerEvent::TextDelta {
                            run_id: ctx.run_id.to_string(),
                            text: text.clone(),
                            partial: current.clone(),
                        },
                    );
                }

                ProviderEvent::ThinkingDelta { text } => {
                    let current = result.thinking.get_or_insert_with(String::new);
                    current.push_str(&text);
                    emit_event(
                        ctx.output_tx,
                        ServerEvent::ThinkingDelta {
                            run_id: ctx.run_id.to_string(),
                            text: text.clone(),
                            partial: current.clone(),
                        },
                    );
                }

                ProviderEvent::ThinkingEnd => {
                    emit_event(
                        ctx.output_tx,
                        ServerEvent::ThinkingEnd {
                            run_id: ctx.run_id.to_string(),
                        },
                    );
                }

                ProviderEvent::ToolCallStarted { id, name } => {
                    tool_call_buffers.insert(id, (name, String::new()));
                }

                ProviderEvent::ToolCallArgumentsDelta { id, delta } => {
                    if let Some((_name, args)) = tool_call_buffers.get_mut(&id) {
                        args.push_str(&delta);
                    }
                }

                ProviderEvent::ToolCallCompleted { id } => {
                    if let Some((name, args_json)) = tool_call_buffers.remove(&id) {
                        let args: serde_json::Value =
                            serde_json::from_str(&args_json).unwrap_or(serde_json::Value::Null);

                        emit_event(
                            ctx.output_tx,
                            ServerEvent::ToolCall {
                                run_id: ctx.run_id.to_string(),
                                id: id.clone(),
                                name: name.clone(),
                                arguments: args.clone(),
                            },
                        );

                        result.tool_calls.push(LlmContentBlock::ToolCall {
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
                    result.usage = Some(UsageInfo {
                        input_tokens,
                        output_tokens,
                    });
                }

                ProviderEvent::Completed => break,

                ProviderEvent::Error { message } => {
                    result.error = Some(message);
                    break;
                }
            }
        }

        result
    }

    /// Execute tool calls and append tool result messages.
    /// Tool results are batched into a single persistence write.
    async fn execute_tool_calls(
        &self,
        ctx: &RunContext<'_>,
        tool_calls: &[LlmContentBlock],
        messages: &mut Vec<Message>,
        cancel: &CancellationToken,
    ) {
        let mut tool_msgs: Vec<Message> = Vec::with_capacity(tool_calls.len());

        for tc in tool_calls {
            if let LlmContentBlock::ToolCall {
                id,
                name,
                arguments,
            } = tc
            {
                let tool_ctx = ToolContext {
                    cwd: ctx.cwd.clone(),
                    run_id: ctx.run_id.to_string(),
                    event_tx: ctx.output_tx.clone(),
                    ask_manager: ctx.ask_manager.clone(),
                };

                let result = ctx
                    .tool_registry
                    .execute(name, tool_ctx, arguments.clone(), cancel.clone())
                    .await;

                let (content_text, is_error, result_metadata) = match result {
                    Ok(tool_result) => {
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

                        emit_event(
                            ctx.output_tx,
                            ServerEvent::ToolResult {
                                run_id: ctx.run_id.to_string(),
                                tool_call_id: id.clone(),
                                content: vec![ContentBlock::Text { text: text.clone() }],
                                is_error: is_err,
                                metadata: metadata.clone(),
                            },
                        );

                        (text, is_err, metadata)
                    }
                    Err(e) => {
                        let err_text = format!("Tool execution error: {e}");

                        emit_event(
                            ctx.output_tx,
                            ServerEvent::ToolResult {
                                run_id: ctx.run_id.to_string(),
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

                let tool_msg = transcript::new_tool_result_message(
                    id,
                    name,
                    content_text,
                    is_error,
                    result_metadata,
                );
                tool_msgs.push(tool_msg);
            }
        }

        // Batch-persist all tool results in one write
        if !tool_msgs.is_empty() {
            if let Err(e) = ctx
                .session_store
                .append_messages(ctx.session_id, &tool_msgs)
                .await
            {
                tracing::error!("Failed to persist tool results: {e}");
            }
            messages.extend(tool_msgs);
        }
    }
}

// ---------------------------------------------------------------------------
// Helper types
// ---------------------------------------------------------------------------

/// Bundles immutable shared state for a run to avoid argument sprawl.
struct RunContext<'a> {
    run_id: &'a Arc<str>,
    system_prompt: &'a str,
    tool_defs: &'a Arc<Vec<ToolDefinition>>,
    model: &'a ModelInfo,
    thinking_level: &'a ThinkingLevel,
    cwd: &'a PathBuf,
    session_store: &'a SessionStore,
    session_id: &'a str,
    tool_registry: &'a ToolRegistry,
    output_tx: &'a mpsc::UnboundedSender<OutputItem>,
    ask_manager: &'a AskManagerHandle,
    provider: &'a dyn LlmProvider,
}

/// Collected output from consuming a provider event stream.
#[derive(Default)]
struct StreamResult {
    text: Option<String>,
    thinking: Option<String>,
    tool_calls: Vec<LlmContentBlock>,
    usage: Option<UsageInfo>,
    error: Option<String>,
}

impl StreamResult {
    fn has_content(&self) -> bool {
        self.text.as_ref().is_some_and(|s| !s.is_empty())
            || self.thinking.as_ref().is_some_and(|s| !s.is_empty())
            || !self.tool_calls.is_empty()
    }
}

enum TurnOutcome {
    Continue,
    Stop { reason: String },
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

fn emit_event_with_agent_start(tx: &mpsc::UnboundedSender<OutputItem>, run_id: &Arc<str>) {
    emit_event(
        tx,
        ServerEvent::AgentStart {
            run_id: run_id.to_string(),
        },
    );
}

fn emit_agent_end(tx: &mpsc::UnboundedSender<OutputItem>, run_id: &Arc<str>, stop_reason: &str) {
    emit_event(
        tx,
        ServerEvent::AgentEnd {
            run_id: run_id.to_string(),
            stop_reason: stop_reason.to_string(),
        },
    );
}
