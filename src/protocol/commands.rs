use serde::Deserialize;

use super::types::{AskUserResponse, ThinkingLevel};

/// Incoming command from the client over stdin
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ClientCommand {
    #[serde(rename_all = "camelCase")]
    Prompt { id: Option<String>, message: String },
    #[serde(rename_all = "camelCase")]
    Abort { id: Option<String> },
    #[serde(rename_all = "camelCase")]
    UserResponse {
        id: Option<String>,
        ask_id: String,
        response: AskUserResponse,
    },
    #[serde(rename_all = "camelCase")]
    SetModel {
        id: Option<String>,
        provider: String,
        model_id: String,
    },
    #[serde(rename_all = "camelCase")]
    SetThinking {
        id: Option<String>,
        level: ThinkingLevel,
    },
    #[serde(rename_all = "camelCase")]
    CycleModel {
        id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Bash {
        id: Option<String>,
        command: String,
    },
    #[serde(rename_all = "camelCase")]
    ListSessions {
        id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    LoadSession {
        id: Option<String>,
        session_id: String,
    },
    #[serde(rename_all = "camelCase")]
    NewSession {
        id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ForkSession {
        id: Option<String>,
        entry_id: String,
    },
    #[serde(rename_all = "camelCase")]
    GetState {
        id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    GetMessages {
        id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ExportSession {
        id: Option<String>,
        output_path: String,
    },
    #[serde(rename_all = "camelCase")]
    Shutdown {
        id: Option<String>,
    },
    #[serde(other)]
    Invalid,
}

/// Envelope wrapping a client command
#[derive(Debug, Clone, Deserialize)]
pub struct ClientCommandEnvelope {
    #[serde(flatten)]
    pub command: ClientCommand,
}
