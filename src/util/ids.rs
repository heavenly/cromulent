use uuid::Uuid;

/// Generate a new session ID
pub fn generate_session_id() -> String {
    format!("ses_{}", Uuid::new_v4().to_string().replace('-', "")[..12].to_string())
}

/// Generate a new run ID
pub fn generate_run_id() -> String {
    format!("run_{}", Uuid::new_v4().to_string().replace('-', "")[..12].to_string())
}

/// Generate a new message ID
pub fn generate_message_id() -> String {
    format!("msg_{}", Uuid::new_v4().to_string().replace('-', "")[..12].to_string())
}

/// Generate a new ask ID
pub fn generate_ask_id() -> String {
    format!("ask_{}", Uuid::new_v4().to_string().replace('-', "")[..12].to_string())
}

/// Generate a new tool call ID
pub fn generate_tool_call_id() -> String {
    format!("call_{}", Uuid::new_v4().to_string().replace('-', "")[..12].to_string())
}
