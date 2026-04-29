use serde::Serialize;

use super::types::{ModelInfo, ThinkingLevel};

/// Synchronous response to a client command
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResponse {
    pub id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl CommandResponse {
    pub fn ok(id: Option<String>) -> Self {
        Self {
            id,
            success: true,
            error: None,
            data: None,
        }
    }

    pub fn ok_with_data(id: Option<String>, data: serde_json::Value) -> Self {
        Self {
            id,
            success: true,
            error: None,
            data: Some(data),
        }
    }

    pub fn err(id: Option<String>, error: impl Into<String>) -> Self {
        Self {
            id,
            success: false,
            error: Some(error.into()),
            data: None,
        }
    }
}

/// State snapshot returned by get_state
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub model: ModelInfo,
    pub thinking_level: ThinkingLevel,
    pub session_id: String,
    pub cwd: String,
    pub message_count: usize,
    pub is_streaming: bool,
    pub run_id: Option<String>,
}
