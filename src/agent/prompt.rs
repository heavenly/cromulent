use std::path::Path;

use crate::protocol::types::ToolDefinition;

/// Context used to render the system prompt.
#[derive(Debug)]
pub struct PromptContext {
    /// Current working directory.
    pub cwd: String,
    /// Current date (ISO 8601 date portion).
    pub date: String,
    /// Registered tool definitions to include in the prompt.
    pub tools: Vec<ToolDefinition>,
}

/// Build the system prompt for the agent.
///
/// The prompt is generated dynamically from runtime context so it always
/// reflects the current working directory, date, and available tools.
pub fn build_system_prompt(ctx: &PromptContext) -> String {
    let tool_list: Vec<String> = ctx
        .tools
        .iter()
        .map(|t| format!("- `{}`: {}", t.name, t.description))
        .collect();
    let tool_section = if tool_list.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n## Available tools\n\nYou have access to the following tools:\n{}\n\n### Tool rules\n\
            - Prefer `read`/`find`/`grep` over `bash` for exploration.\n\
            - `read` before `edit`.\n\
            - Use `edit` for existing files, `write` for new files.\n\
            - Before `ask_user`, gather enough context to ask a focused question.\n\
            - Make real changes with tools when appropriate.",
            tool_list.join("\n")
        )
    };

    format!(
        "You are cromulent, a headless coding agent.\n\
        You can inspect files, edit files, run shell commands, \
        search with grep/find, and ask the user for clarification.\n\n\
        ## Core rules\n\
        - Be concise and explicit about file paths.\n\
        - Do not invent file contents or command outputs.\n\
        - Do not make assumptions about the codebase without reading relevant files first.\n\
        - If you are unsure, ask the user for clarification using `ask_user`.\n\n\
        ## Operational context\n\
        Working directory: {cwd}\n\
        Current date: {date}\n\
        {tool_section}",
        cwd = ctx.cwd,
        date = ctx.date,
        tool_section = tool_section,
    )
}

/// Convenience: build a system prompt from a cwd path and tool definitions.
pub fn build_system_prompt_from(
    cwd: &Path,
    tools: &[ToolDefinition],
) -> String {
    let date = chrono::Utc::now()
        .format("%Y-%m-%d")
        .to_string();

    let ctx = PromptContext {
        cwd: cwd.to_string_lossy().to_string(),
        date,
        tools: tools.to_vec(),
    };

    build_system_prompt(&ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_system_prompt_includes_cwd() {
        let ctx = PromptContext {
            cwd: "/home/user/project".into(),
            date: "2026-04-28".into(),
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({}),
            }],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("/home/user/project"));
        assert!(prompt.contains("2026-04-28"));
        assert!(prompt.contains("read"));
        assert!(prompt.contains("- `read`"));
    }

    #[test]
    fn test_build_system_prompt_no_tools() {
        let ctx = PromptContext {
            cwd: "/tmp".into(),
            date: "2026-04-28".into(),
            tools: vec![],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Available tools"));
    }

    #[test]
    fn test_convenience_fn() {
        let prompt = build_system_prompt_from(
            &PathBuf::from("/test"),
            &[],
        );
        assert!(prompt.contains("/test"));
    }
}
