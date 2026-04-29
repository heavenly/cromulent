use serde::Serialize;

use super::types::{ContentBlock, ModelInfo, ThinkingLevel, UsageInfo};

/// Events emitted by the daemon to stdout (one JSONL line each)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ServerEvent {
    #[serde(rename_all = "camelCase")]
    SessionChanged {
        session_id: String,
        cwd: String,
        model: ModelInfo,
        thinking_level: ThinkingLevel,
    },
    #[serde(rename_all = "camelCase")]
    AgentStart { run_id: String },
    #[serde(rename_all = "camelCase")]
    TurnStart { run_id: String, turn: u32 },
    #[serde(rename_all = "camelCase")]
    TextDelta {
        run_id: String,
        text: String,
        partial: String,
    },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta {
        run_id: String,
        text: String,
        partial: String,
    },
    #[serde(rename_all = "camelCase")]
    ThinkingEnd { run_id: String },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        run_id: String,
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        run_id: String,
        tool_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
    #[serde(rename_all = "camelCase")]
    Ask {
        run_id: String,
        id: String,
        question: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<String>,
        #[serde(default)]
        options: Vec<super::types::AskOption>,
        #[serde(default)]
        allow_multiple: bool,
        #[serde(default = "default_true")]
        allow_freeform: bool,
        #[serde(default)]
        allow_comment: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Error { run_id: String, message: String },
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        run_id: String,
        turn: u32,
        stop_reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<UsageInfo>,
    },
    #[serde(rename_all = "camelCase")]
    AgentEnd { run_id: String, stop_reason: String },
    #[serde(rename_all = "camelCase")]
    BashOutput { stdout: String, stderr: String },
    #[serde(rename_all = "camelCase")]
    BashDone { exit_code: i32 },
}

fn default_true() -> bool {
    true
}
