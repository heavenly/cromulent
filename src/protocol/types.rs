use serde::{Deserialize, Serialize};

use crate::transport::writer::OutputItem;

// ---------------------------------------------------------------------------
// Core protocol types shared across commands, events, and the agent loop
// ---------------------------------------------------------------------------

/// Usage statistics for an LLM turn
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Thinking / reasoning level
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ThinkingLevel {
    Low,
    Medium,
    High,
}

/// Identifies a provider + model pair
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub provider: String,
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default = "default_supports_tools")]
    pub supports_tools: bool,
}

fn default_context_window() -> u32 {
    128_000
}

fn default_supports_tools() -> bool {
    true
}

/// One entry in the transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub timestamp: String,
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A content block within a message (multi-modal support)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ContentBlock {
    #[serde(rename_all = "camelCase")]
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        tool_call_id: String,
        content: Vec<ContentBlock>,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
}

/// Tool definition sent to the LLM provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Provider-normalized event stream (internal, not wire format)
#[derive(Debug, Clone)]
pub enum ProviderEvent {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ThinkingEnd,
    ToolCallStarted {
        id: String,
        name: String,
    },
    ToolCallArgumentsDelta {
        id: String,
        delta: String,
    },
    ToolCallCompleted {
        id: String,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    Completed,
    Error {
        message: String,
    },
}

/// Request payload sent to a provider
#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: ModelInfo,
    pub system_prompt: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
    pub thinking_level: ThinkingLevel,
    pub cwd: std::path::PathBuf,
}

/// Simplified LLM message format for provider requests
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmMessage {
    pub role: String,
    pub content: Vec<LlmContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum LlmContentBlock {
    #[serde(rename_all = "camelCase")]
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
    },
}

/// The ask_user payload
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskPayload {
    pub id: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default)]
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub allow_multiple: bool,
    #[serde(default = "default_true")]
    pub allow_freeform: bool,
    #[serde(default)]
    pub allow_comment: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskOption {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// User's response to an ask
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserResponse {
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Tool execution context
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: std::path::PathBuf,
    pub run_id: String,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<OutputItem>,
    pub ask_manager: crate::tools::ask_user::AskManagerHandle,
}

/// Result from tool execution
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub metadata: Option<serde_json::Value>,
}
