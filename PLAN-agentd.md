Yes — here is a refined plan that keeps your original vision, but fixes the state model, transport safety, provider abstraction, `ask_user` flow, and persistence semantics.

## Overview

`cromulent` is a headless, single-binary Rust daemon that runs the coding-agent loop behind a stable JSONL stdin/stdout protocol. The daemon owns LLM/provider sessions, tool execution, transcript state, session persistence, cancellation, and blocking human-in-the-loop interactions, while UIs remain thin clients that render events and send commands. Headless separation is a strong architectural choice because it keeps rendering concerns out of the agent core and lets different clients interpret the same event stream consistently. [discuss.google](https://discuss.google.dev/t/headless-agents-architecting-decoupled-ai-systems/323144)

The main refinement is this: `cromulent` should be modeled as a single stateful runtime with one serialized stdout writer and one authoritative in-memory session state. Public model/tool APIs should normalize upstream provider behavior instead of leaking provider-specific streaming formats into the core loop. OpenAI’s modern function/tool calling guidance also makes strict schema handling and event normalization important for reliability. [community.openai](https://community.openai.com/t/chatcompletions-vs-responses-api-difference-in-parallel-tool-call-behaviour-observed/1369663)

## Goals

- Single long-lived process with one active session loaded at a time.
- JSONL protocol over stdin/stdout, one object per line, no mixed writers.
- One authoritative transcript in memory, persisted append-only to disk.
- Provider-agnostic agent loop with normalized stream events.
- Blocking `ask_user` interaction implemented safely with pending oneshots.
- Safe cancellation for streaming and tool execution.
- Separate UI processes: TUI, CLI, IDE plugin, web frontend.
- Deterministic integration testing with a fake provider.

## Non-goals

- No UI rendering logic in `cromulent`.
- No multiplexed multi-client shared socket server in v1.
- No distributed execution or remote tool workers in v1.
- No MCP, sub-agents, or background planning in v1.
- No provider-specific assumptions in the core loop.

## Core principles

1. **One writer** to stdout. Events and responses must flow through the same serializer to avoid interleaved JSONL.
2. **One owner** of mutable runtime state. Do not clone the agent and let copies drift.
3. **One source of truth** for transcript mutation and session persistence.
4. **One normalized stream model** across providers.
5. **One cancellation path** shared by provider streams and tools.
6. **One pending-ask registry** keyed by `askId`.

***

## Runtime architecture

```text
stdin(JSONL) ──> TransportReader ──> CommandRouter ──> AppRuntime
                                                      │
                                                      ├── SessionManager
                                                      ├── AgentRunner
                                                      ├── ToolExecutor
                                                      ├── ProviderManager
                                                      ├── AskManager
                                                      └── BashRunner

AppRuntime ──> EventBus/OutputQueue ──> TransportWriter ──> stdout(JSONL)
stderr <──── structured debug logs only
```

### Responsibilities

**TransportReader**
- Reads stdin line by line.
- Parses JSON into `ClientCommandEnvelope`.
- Emits parse errors as structured internal errors, not panics.

**TransportWriter**
- The only component allowed to write to stdout.
- Serializes both server events and command responses.
- Flushes after every line.

**CommandRouter**
- Validates command shape.
- Converts commands into internal actions.
- Returns synchronous command responses when applicable.

**AppRuntime**
- Owns global state.
- Tracks active session, model, thinking level, cwd, current run, pending asks.
- Prevents conflicting operations.

**AgentRunner**
- Runs exactly one foreground agent turn at a time.
- Owns the mutable transcript during execution.
- Normalizes tool-call execution and follow-up turns.

**ToolExecutor**
- Executes registered tools.
- Accepts cancel token.
- Returns typed content blocks and structured errors.

**ProviderManager**
- Resolves provider adapter from selected model.
- Converts transcript + tools into provider-specific request payloads.

**AskManager**
- Owns `HashMap<AskId, oneshot::Sender<AskUserResponse>>`.
- Coordinates blocking `ask_user`.

**SessionManager**
- Creates, loads, appends, exports, forks sessions.
- Handles metadata and atomic disk writes.

***

## State model

Use a single authoritative runtime state.

```rust
pub struct AppState {
    pub current_session: LoadedSessionState,
    pub model: ModelInfo,
    pub thinking_level: ThinkingLevel,
    pub cwd: PathBuf,
    pub run_state: RunState,
    pub pending_asks: HashMap<String, oneshot::Sender<AskUserResponse>>,
    pub sessions_dir: PathBuf,
    pub config: AppConfig,
}

pub enum RunState {
    Idle,
    Running {
        run_id: String,
        cancel: CancellationHandle,
        started_at: chrono::DateTime<chrono::Utc>,
    },
}
```

### Key rule

Only `AppRuntime` mutates `AppState`. Other components receive snapshots, typed commands, or handles.

This removes the biggest flaw in the original sketch: cloning `agent` into spawned tasks would create split state and inconsistent transcript history.

***

## Protocol revision

Keep JSONL, but refine the protocol around consistency and forward compatibility.

### Envelope rules

- Every client command may include `id`.
- Every command gets exactly one `response`.
- Events never include command correlation IDs unless they are part of runtime state, like `ask.id`.
- All field names should use `camelCase` on the wire.
- Rust structs should use `#[serde(rename_all = "camelCase")]`.

### Revised command set

```jsonc
{"id":"1","type":"prompt","message":"Write a Rust function to sort"}
{"id":"2","type":"abort"}
{"id":"3","type":"user_response","askId":"ask_1","response":{...}}
{"id":"4","type":"set_model","provider":"openai","modelId":"gpt-5-codex"}
{"id":"5","type":"set_thinking","level":"high"}
{"id":"6","type":"cycle_model"}
{"id":"7","type":"bash","command":"git status"}
{"id":"8","type":"list_sessions"}
{"id":"9","type":"load_session","sessionId":"abc123"}
{"id":"10","type":"new_session"}
{"id":"11","type":"fork_session","entryId":"msg_456"}
{"id":"12","type":"get_state"}
{"id":"13","type":"get_messages"}
{"id":"14","type":"export_session","outputPath":"/tmp/session.json"}
{"id":"15","type":"shutdown"}
```

### Revised event set

Keep your existing event model, but tighten it:

```jsonc
{"type":"session_changed","sessionId":"abc123","cwd":"/proj","model":{"provider":"openai","id":"gpt-5-codex"},"thinkingLevel":"medium"}
{"type":"agent_start","runId":"run_1"}
{"type":"turn_start","runId":"run_1","turn":1}
{"type":"text_delta","runId":"run_1","text":"Here","partial":"Here"}
{"type":"thinking_delta","runId":"run_1","text":"Let me analyze","partial":"Let me analyze"}
{"type":"thinking_end","runId":"run_1"}
{"type":"tool_call","runId":"run_1","id":"call_1","name":"read","arguments":{"path":"src/main.rs"}}
{"type":"tool_result","runId":"run_1","toolCallId":"call_1","content":[{"type":"text","text":"..."}],"isError":false}
{"type":"ask","runId":"run_1","id":"ask_1","question":"Which approach?","context":"...","options":[...],"allowMultiple":false,"allowFreeform":true,"allowComment":false,"timeoutMs":null}
{"type":"error","runId":"run_1","message":"Tool failed"}
{"type":"turn_end","runId":"run_1","turn":1,"stopReason":"tool_calls","usage":{"inputTokens":123,"outputTokens":55}}
{"type":"agent_end","runId":"run_1","stopReason":"completed"}
{"type":"bash_output","stdout":"line 1\n","stderr":""}
{"type":"bash_done","exitCode":0}
```

### Add `runId`

This is important. It lets the UI correlate an event stream with the active turn and discard stale deltas if a new prompt starts after an abort.

***

## Protocol types

Refine protocol structs around wire compatibility and typed levels.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThinkingLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub provider: String,
    pub id: String,
    pub display_name: String,
    pub context_window: u32,
    pub supports_reasoning: bool,
    pub supports_tools: bool,
}
```

Use enums where the allowed values are finite. Do not store `"medium"` and `"high"` as arbitrary strings internally.

***

## Session format

This is the largest structural improvement over the original plan.

Your current header is too small to satisfy “restore history, cwd, and model.” Persist enough state to actually restore the session.

### Recommended on-disk layout

```text
~/.local/share/cromulent/
  sessions/
    <sessionId>.jsonl
  exports/
  logs/
~/.config/cromulent/
  config.json
  auth/
    codex.json
    openai.json
```

### Session file format

First line is a typed header, followed by append-only entries.

```jsonc
{"type":"session_header","sessionId":"abc123","created":"2026-04-28T06:00:00Z","updated":"2026-04-28T06:10:00Z","cwd":"/proj","model":{"provider":"openai","id":"gpt-5-codex","displayName":"GPT-5 Codex","contextWindow":200000,"supportsReasoning":true,"supportsTools":true},"thinkingLevel":"medium","schemaVersion":1}
{"type":"message", ...}
{"type":"message", ...}
{"type":"message", ...}
```

### Session rules

- Header rewrite is atomic: write temp file, rename.
- Message append is append-only.
- `updated` and `messageCount` should be derived or cached safely.
- Session load should validate `schemaVersion`.
- Forking copies messages up to and including the selected entry into a new session file with a fresh header.

### Session structs

```rust
pub struct SessionHeader {
    pub session_id: String,
    pub created: String,
    pub updated: String,
    pub cwd: String,
    pub model: ModelInfo,
    pub thinking_level: ThinkingLevel,
    pub schema_version: u32,
}

pub struct LoadedSessionState {
    pub header: SessionHeader,
    pub messages: Vec<Message>,
}
```

***

## Message model

Keep messages simple and transcript-oriented.

```rust
pub struct Message {
    pub id: String,
    pub timestamp: String,
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub is_error: Option<bool>,
}

pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}
```

Prefer a role enum internally, with serde mapping to wire strings.

### Important transcript rule

Only the `AgentRunner` appends user, assistant, and tool messages during a run. `main.rs` or the command router must not independently mutate the transcript.

That prevents duplicate messages and persistence drift.

***

## Agent loop

The refined loop should be explicit and provider-neutral.

### Prompt flow

1. Validate runtime is idle.
2. Create `runId`.
3. Set `RunState::Running`.
4. Append user message to transcript.
5. Persist user message.
6. Emit `agent_start`.
7. Enter turn loop.
8. Build provider request from transcript + tools + system prompt.
9. Stream normalized provider events.
10. Buffer assistant text and tool calls.
11. When tool calls complete, execute them, append tool results, persist them, then continue the next turn.
12. When no tool calls remain and provider completes, finalize assistant message, persist it.
13. Emit `agent_end`.
14. Return runtime to idle.

### Turn algorithm

```text
for turn in 1..=max_turns:
  emit turn_start
  provider.stream(...)
  collect:
    - text deltas
    - thinking deltas
    - tool call start / argument deltas / tool call end
    - usage
    - provider errors

  if cancelled: stop cleanly
  if assistant emitted tool calls:
      execute tools
      append tool messages
      emit turn_end(stopReason="tool_calls")
      continue
  else:
      append assistant message
      emit turn_end(stopReason="completed")
      break
```

### Guardrails

- `max_turns` must stop loops deterministically.
- Tool call IDs must be stable.
- If a provider emits invalid tool JSON, return a tool-style error event and let the model recover next turn.
- If a tool fails, append a tool-result message with `is_error=true`; do not crash the run.

***

## Provider abstraction

This part should be more explicit than the original plan.

### Internal normalized stream model

```rust
pub enum ProviderEvent {
    TextDelta { text: String },
    ThinkingDelta { text: String },
    ThinkingEnd,
    ToolCallStarted { id: String, name: String },
    ToolCallArgumentsDelta { id: String, delta: String },
    ToolCallCompleted { id: String },
    Usage { input_tokens: u32, output_tokens: u32 },
    Completed,
    Error { message: String },
}
```

The `AgentRunner` then reconstructs tool arguments from deltas and emits UI-friendly `ServerEvent`s.

### Why this matters

OpenAI’s function/tool streaming is event-driven and not equivalent to a plain text stream, so treating all providers as if they emit the same shape is brittle. Their tool-calling guidance also emphasizes structured function definitions and strict schemas rather than ad hoc argument parsing. [community.openai](https://community.openai.com/t/chatcompletions-vs-responses-api-difference-in-parallel-tool-call-behaviour-observed/1369663)

### Provider trait

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError>;
}
```

### Provider request

```rust
pub struct ProviderRequest {
    pub model: ModelInfo,
    pub system_prompt: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
    pub thinking_level: ThinkingLevel,
    pub cwd: PathBuf,
}
```

### Providers in v1

- `openai_responses.rs`
- `openai_chat_compat.rs` only if truly needed
- `fake.rs` for tests

I would not make a private `chatgpt.com/backend-api/responses` adapter a core v1 dependency. Public auth flows documented for Codex are a safer baseline for an agent tool than relying on a consumer web backend contract. [community.openai](https://community.openai.com/t/responses-api-parallel-tool-calls-not-happening/1226942)

***

## Authentication plan

Refine this to use supported auth paths first.

### Recommended auth priority

1. API key providers from env/config.
2. Supported OpenAI/Codex auth flow with cached credentials.
3. Optional experimental ChatGPT account auth behind a feature flag.

### Why

OpenAI’s Codex auth documentation describes sign-in flows, cached auth state, and refresh behavior intended for developer tooling, which is more durable than scraping or depending on private web-app behavior. [community.openai](https://community.openai.com/t/responses-api-parallel-tool-calls-not-happening/1226942)

### Config layout

```json
{
  "providers": {
    "openai": { "apiKeyEnv": "OPENAI_API_KEY" },
    "deepseek": { "apiKeyEnv": "DEEPSEEK_API_KEY" },
    "opencode": { "apiKeyEnv": "OPENCODE_API_KEY" }
  },
  "defaultModel": { "provider": "openai", "id": "gpt-5-codex" },
  "thinkingLevel": "medium",
  "maxTurns": 40
}
```

***

## Tool system

Keep the registry approach, but tighten types and safety.

### Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    async fn execute(
        &self,
        ctx: ToolContext,
        arguments: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolError>;
}
```

### Tool context

```rust
pub struct ToolContext {
    pub cwd: PathBuf,
    pub event_tx: mpsc::UnboundedSender<ServerEvent>,
    pub ask_manager: AskManagerHandle,
}
```

### Tool result

```rust
pub struct ToolResult {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub metadata: Option<serde_json::Value>,
}
```

### Required v1 tools

- `read`
- `write`
- `edit`
- `grep`
- `find`
- `bash`
- `ask_user`

### Tool execution rules

- `read` before `edit`.
- `edit` for existing files, `write` for new files.
- Prefer purpose-built tools over shelling out.
- `bash` is allowed but should be auditable and cancellable.
- Tool schemas must use strict JSON Schema where supported. OpenAI’s function-calling docs emphasize structured schemas and stricter validation for better reliability. [community.openai](https://community.openai.com/t/chatcompletions-vs-responses-api-difference-in-parallel-tool-call-behaviour-observed/1369663)

***

## `ask_user` design

This is one of the most important fixes.

### Correct flow

When the model invokes `ask_user`:

1. `ToolExecutor` parses arguments.
2. `AskManager` creates `askId`.
3. `AskManager` creates `oneshot::channel()`.
4. Sender goes into `pending_asks[askId]`.
5. Emit `ServerEvent::Ask`.
6. Block on `oneshot::Receiver` or cancellation.
7. On `user_response`, runtime looks up `askId` and resolves the sender.
8. Tool returns a structured result back into the transcript.

### Ask manager interface

```rust
pub struct AskManager {
    pending: Mutex<HashMap<String, oneshot::Sender<AskUserResponse>>>,
}

impl AskManager {
    pub async fn create_pending(&self, ask: AskPayload) -> oneshot::Receiver<AskUserResponse>;
    pub async fn resolve(&self, ask_id: &str, response: AskUserResponse) -> Result<(), AskError>;
    pub async fn cancel_all(&self);
}
```

### Important behaviors

- Unknown `askId` returns a command error response.
- Duplicate `user_response` for the same `askId` returns an error.
- Abort should cancel any currently pending ask for the active run.

***

## Cancellation model

Use one cancellation primitive everywhere.

### Recommended approach

Use `tokio_util::sync::CancellationToken` instead of a watch bool for the top-level run, then bridge as needed.

```rust
pub struct CancellationHandle {
    pub token: CancellationToken,
}
```

### Cancellation semantics

- `abort` cancels the active run only.
- Provider stream must stop quickly.
- Tool executions must check cancellation cooperatively.
- Pending `ask_user` must resolve with cancellation.
- After abort, emit:
  - `error` if needed
  - `turn_end(stopReason="aborted")`
  - `agent_end(stopReason="aborted")`

This is cleaner than a scattered `watch::Sender<bool>` model.

***

## Raw bash command path

Keep UI-initiated `bash`, but isolate it from agent tool execution.

### Difference

- `ClientCommand::bash` is a direct runtime command.
- Tool `bash` is an agent-invoked tool within a run.

These should use different code paths and event sets, even if they share process execution internals.

### Raw bash rules

- Stream `stdout` and `stderr` incrementally as `bash_output`.
- Emit `bash_done`.
- Return a normal `response` to the command as acknowledgment.

Do not route raw `bash` through the transcript unless you intentionally want command history persisted.

***

## System prompt

Make it runtime-generated and scoped.

```rust
fn build_system_prompt(ctx: &PromptContext) -> String
```

### Prompt context

- cwd
- current date
- tool list
- tool usage rules
- output style
- safety constraints

### Recommended prompt shape

```text
You are cromulent, a headless coding agent.

You can inspect files, edit files, run shell commands, search with grep/find, and ask the user for clarification.

Rules:
- Prefer read/find/grep over bash for exploration.
- Read before editing.
- Use edit for existing files, write for new files.
- Before ask_user, gather enough context to ask a focused question.
- Make real changes with tools when appropriate.
- Be concise and explicit about file paths.
- Do not invent file contents or command outputs.

Working directory: ...
Date: ...
```

Keep this short. Overgrown prompts create drift.

***

## Crate structure

Refactor the project around runtime ownership.

```text
cromulent/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── app/
│   │   ├── mod.rs
│   │   ├── runtime.rs
│   │   ├── state.rs
│   │   ├── router.rs
│   │   └── output.rs
│   ├── protocol/
│   │   ├── mod.rs
│   │   ├── commands.rs
│   │   ├── events.rs
│   │   ├── responses.rs
│   │   └── types.rs
│   ├── transport/
│   │   ├── mod.rs
│   │   ├── reader.rs
│   │   └── writer.rs
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── runner.rs
│   │   ├── transcript.rs
│   │   └── prompt.rs
│   ├── providers/
│   │   ├── mod.rs
│   │   ├── openai_responses.rs
│   │   ├── deepseek_compat.rs
│   │   └── fake.rs
│   ├── tools/
│   │   ├── mod.rs
│   │   ├── registry.rs
│   │   ├── bash.rs
│   │   ├── read.rs
│   │   ├── write.rs
│   │   ├── edit.rs
│   │   ├── grep.rs
│   │   ├── find.rs
│   │   └── ask_user.rs
│   ├── session/
│   │   ├── mod.rs
│   │   ├── store.rs
│   │   ├── export.rs
│   │   └── fork.rs
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   └── codex.rs
│   ├── process/
│   │   ├── mod.rs
│   │   └── bash_runner.rs
│   └── util/
│       ├── ids.rs
│       ├── time.rs
│       └── fs.rs
└── tests/
    ├── protocol_jsonl.rs
    ├── agent_fake_provider.rs
    ├── ask_user_flow.rs
    ├── cancellation.rs
    └── sessions.rs
```

***

## Main runtime flow

A good `main.rs` should mostly be wiring.

### Startup

1. Parse CLI.
2. Load config.
3. Resolve sessions dir and config dir.
4. Load or create session.
5. Build tool registry.
6. Start transport reader.
7. Start transport writer.
8. Start `AppRuntime`.
9. Route commands until shutdown.
10. Gracefully cancel active run on exit.

### CLI

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long)]
    provider: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    thinking: Option<String>,

    #[arg(long)]
    session: Option<String>,

    #[arg(long)]
    cwd: Option<PathBuf>,

    #[arg(long, default_value_t = 40)]
    max_turns: u32,

    #[arg(long)]
    sessions_dir: Option<PathBuf>,

    #[arg(long)]
    setup_codex: bool,
}
```

Make provider/model optional so config can be the source of truth.

***

## Command handling rules

### `prompt`
- Reject if already running, or choose queue semantics later.
- Return immediate success response.
- Start run asynchronously.
- All stream output arrives as events.

### `abort`
- Succeeds whether or not a run is active.
- If idle, no-op.
- If active, cancel and emit final lifecycle events.

### `set_model`
- Allowed only when idle in v1.
- Update runtime state and session header.
- Emit `session_changed`.

### `load_session`
- Allowed only when idle.
- Load transcript + metadata.
- Update cwd/model/thinking from session header.
- Emit `session_changed`.

### `new_session`
- Allowed only when idle.
- Persist current session metadata.
- Create fresh session file and load it as active.

### `fork_session`
- Allowed only when idle.
- Copy transcript up to `entryId`.
- New session becomes active.

### `get_state`
Return:
```jsonc
{
  "model": {...},
  "thinkingLevel":"medium",
  "sessionId":"abc123",
  "cwd":"/proj",
  "messageCount":42,
  "isStreaming":false,
  "runId":null
}
```

***

## Error model

Define structured errors internally, plain strings on the wire.

```rust
pub enum AppError {
    InvalidCommand(String),
    Busy,
    Session(SessionError),
    Provider(ProviderError),
    Tool(ToolError),
    Io(std::io::Error),
}
```

### Behavior

- Command-level failures become `response.success=false`.
- Runtime non-fatal issues become `error` events.
- Tool failures stay inside the transcript as tool results with `isError=true`.
- Panics should be avoided entirely in command handling.

***

## Persistence strategy

### Atomicity

- Append messages with a buffered file handle or reopen-on-append.
- Rewrite headers atomically via temp file + rename.
- Consider `fsync` only where needed; default append safety is enough for v1.

### Export

Prefer a portable JSON export format:

```json
{
  "schemaVersion": 1,
  "header": {...},
  "messages": [...]
}
```

Better than raw JSONL for interchange.

***

## Testing strategy

This deserves more emphasis than in the original plan.

### Add a fake provider first

The fake provider should support scripted turns like:
- text only
- one tool call
- multiple tool calls
- invalid tool args
- thinking deltas
- provider error
- hanging stream for abort tests

### Test layers

**Unit**
- protocol serde roundtrip
- session header parsing
- ask manager resolve/cancel behavior
- tool argument validation

**Integration**
- prompt → stream → complete
- prompt → tool call → tool result → next turn
- prompt → ask → user_response → resume
- prompt → abort during stream
- prompt → abort during tool
- load/new/fork/export session

**Golden transcript tests**
Use fake provider scripts to assert emitted JSONL lines exactly.

***

## Revised implementation phases

### Phase 1: Transport and protocol
1. Initialize crate and dependencies.
2. Implement protocol structs with strict serde tests.
3. Build `TransportReader` and `TransportWriter`.
4. Create single output queue for events and responses.
5. Milestone: `get_state` roundtrip works over JSONL.

### Phase 2: App runtime and sessions
6. Implement `AppState`.
7. Implement `SessionStore` with enriched header.
8. Add `new_session`, `load_session`, `list_sessions`, `get_messages`, `get_state`.
9. Milestone: sessions persist and restore model/cwd/thinking correctly.

### Phase 3: Fake provider and agent runner
10. Implement normalized `ProviderEvent`.
11. Implement `fake` provider.
12. Build `AgentRunner` turn loop.
13. Milestone: prompt works with fake streaming responses.

### Phase 4: Tools
14. Implement `ToolDefinition`, registry, and execution path.
15. Add `read`, `write`, `edit`, `find`, `grep`.
16. Add tool transcript persistence.
17. Milestone: fake provider can invoke tools and continue.

### Phase 5: `ask_user` and cancellation
18. Implement `AskManager`.
19. Implement `ask_user` tool.
20. Add `abort` with `CancellationToken`.
21. Milestone: blocked ask resumes on `user_response`, abort interrupts safely.

### Phase 6: Real providers
22. Add `openai_responses`.
23. Add env/config auth resolution.
24. Add model registry.
25. Milestone: real prompt streams from supported provider.

### Phase 7: Raw bash and polish
26. Add raw `bash` command execution and streaming.
27. Add export/import polish and richer errors.
28. Add CLI help and integration scripts.
29. Milestone: end-to-end usable daemon.

### Phase 8: Optional auth extensions
30. Add supported Codex-style auth flow if needed.
31. Put any ChatGPT-account integration behind an experimental feature.
32. Milestone: optional alternate auth works without affecting core stability. [community.openai](https://community.openai.com/t/responses-api-parallel-tool-calls-not-happening/1226942)

***

## Dependency revision

A tighter dependency list:

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
futures = "0.3"
dirs = "5"
regex = "1"
walkdir = "2"
similar = "2"
async-trait = "0.1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
```

### Notes

- Add `tokio-util` for cancellation token.
- Add `thiserror` for clean internal error types.
- Add `tracing` instead of ad hoc stderr logging.

***

## Example: refined `ask_user` wire flow

```text
Client                         cromulent
  |                              |
  | prompt --------------------> |
  | <-------- response ----------|
  | <------ agent_start -------- |
  | <------- turn_start -------- |
  | <------- tool_call --------- |
  | <---------- ask ------------ |  (run blocks)
  | user_response -------------> |
  | <-------- response ----------|
  | <------ tool_result -------- |
  | <-------- text_delta ------- |
  | <-------- turn_end --------- |
  | <-------- agent_end -------- |
```

This flow stays deterministic because there is one active run, one pending ask entry, and one writer.

***

## Example: session header

```json
{
  "type": "session_header",
  "sessionId": "abc123",
  "created": "2026-04-28T06:00:00Z",
  "updated": "2026-04-28T06:15:22Z",
  "cwd": "/home/user/project",
  "model": {
    "provider": "openai",
    "id": "gpt-5-codex",
    "displayName": "GPT-5 Codex",
    "contextWindow": 200000,
    "supportsReasoning": true,
    "supportsTools": true
  },
  "thinkingLevel": "medium",
  "schemaVersion": 1
}
```

That now actually supports restore semantics.

***

## Final spec summary

The refined version of `cromulent` should be:

- A headless JSONL daemon with strict single-writer output discipline.
- A single-owner runtime, not cloned agents.
- A provider-normalized streaming engine.
- A transcript-first agent loop with append-only persistence.
- A proper `AskManager` using pending oneshot channels.
- A cancellable runtime with one foreground run at a time.
- A session format that truly restores cwd, model, and thinking state.
- A test-first implementation built around a fake provider before live APIs.