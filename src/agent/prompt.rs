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
            - `read` exact regions before editing; `read` returns `LINE#HASH:content` anchors.\n\
            - Use `hashline_edit` for existing files, copying anchors from `read`.\n\
            - Replacement lines in `hashline_edit` must be literal file content: no `LINE#HASH:` prefixes and no diff `+`/`-` prefixes.\n\
            - Use `write` for new files only; it is create-only by default.\n\
            - Make the smallest correct change. Do not rewrite whole files/functions, reformat, rename, refactor, or reorganize unless requested.\n\
            - Broad or risky changes require `ask_user` after gathering context.\n\
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
        - Preserve existing style and structure; keep edits surgical unless explicitly asked otherwise.\n\
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

#[cfg(test)]
#[path = "../../tests/inline/agent_prompt_tests.rs"]
mod tests;
