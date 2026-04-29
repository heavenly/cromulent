# Development

This document describes cromulent internals: architecture, JSONL protocol, testing, tools, and project layout.

## Architecture

```text
stdin(JSONL) ──> TransportReader ──> CommandRouter ──> AppRuntime
                                                      │
                                                      ├── SessionManager
                                                      ├── AgentRunner
                                                      ├── ToolRegistry / tools
                                                      ├── ProviderManager
                                                      ├── AskManager
                                                      └── BashRunner

AppRuntime ──> OutputQueue ──> TransportWriter ──> stdout(JSONL)
stderr ──────> structured debug logs only
```

### Core principles

- **Single stdout writer**: only `TransportWriter` writes JSONL to stdout, avoiding interleaved output.
- **Single runtime owner**: `AppRuntime` owns and mutates runtime state.
- **Provider normalization**: providers stream into a common `ProviderEvent` enum before the agent loop sees them.
- **Transcript-first sessions**: sessions persist as append-only JSONL entries with a header containing cwd, model, and thinking level.
- **Shared cancellation**: one `CancellationToken` propagates through provider streams, tool execution, `ask_user`, and run state.
- **Tool metadata**: tools return model-visible text plus optional structured metadata, surfaced on `toolResult` events and persisted in transcript `ToolResult` content blocks.

## Runtime flow

1. `TransportReader` parses stdin lines into `ClientCommand` values.
2. `CommandRouter` validates idle/running constraints and calls `AppRuntime` handlers.
3. `AppRuntime` updates state, persists sessions, starts/cancels runs, and emits responses/events.
4. `AgentRunner` builds the system prompt, sends provider requests, handles provider events, executes tools, and appends transcript messages.
5. `TransportWriter` serializes all `CommandResponse` and `ServerEvent` values as JSONL.

## Protocol

All protocol messages are JSON objects with a `type` field. Commands are sent on stdin; responses/events are emitted on stdout.

### Commands

| Command | Key fields | Notes |
|---|---|---|
| `prompt` | `message` | Starts an agent run. Returns `runId`. |
| `abort` | — | Cancels the active run. |
| `userResponse` | `askId`, `response` | Resolves a pending `ask_user`. |
| `setModel` | `provider`, `modelId` | Idle-only. |
| `setThinking` | `level` | Idle-only; `low`, `medium`, or `high`. |
| `cycleModel` | — | Switches to next configured model. |
| `bash` | `command` | UI-initiated raw bash, outside agent tools. |
| `listSessions` | — | Lists persisted session IDs. |
| `loadSession` | `sessionId` | Idle-only. |
| `newSession` | — | Idle-only. |
| `forkSession` | `entryId` | Forks transcript up to an entry. |
| `getState` | — | Returns current state snapshot. |
| `getMessages` | — | Returns current session messages. |
| `exportSession` | `outputPath` | Writes portable export JSON. |
| `shutdown` | — | Graceful daemon shutdown. |

Every command with an `id` receives one synchronous `response` object:

```json
{"id":"1","success":true,"data":{"runId":"run_abc123"}}
```

or:

```json
{"id":"1","success":false,"error":"message"}
```

### Events

| Event | Purpose |
|---|---|
| `sessionChanged` | Active session/model/thinking changed. |
| `agentStart` | Agent run started. |
| `turnStart` | LLM turn started. |
| `textDelta` | Assistant text chunk. |
| `thinkingDelta` | Reasoning/thinking chunk. |
| `thinkingEnd` | Reasoning stream ended. |
| `toolCall` | Agent requested a tool. |
| `toolResult` | Tool result text plus optional metadata. |
| `ask` | Agent is waiting for user input. |
| `error` | Non-fatal runtime error. |
| `turnEnd` | LLM turn completed. |
| `agentEnd` | Whole run completed. |
| `bashOutput` | UI raw bash stdout/stderr chunk. |
| `bashDone` | UI raw bash exit code. |

### Tool result metadata

`ServerEvent::ToolResult` includes optional `metadata`:

```json
{
  "type": "toolResult",
  "runId": "run_1",
  "toolCallId": "call_1",
  "content": [{"type":"text","text":"Applied 1 edit"}],
  "metadata": {"changedSpan": {"start": 42, "end": 42}}
}
```

Metadata is host/UI-facing. The LLM receives flattened tool text.

## Tools

Tools implement `src/tools/registry.rs::Tool`:

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

Default tools:

| Tool | Module | Purpose |
|---|---|---|
| `read` | `src/tools/read.rs` | Reads UTF-8 text as `LINE#HASH:content`. |
| `hashline_edit` | `src/tools/hashline/edit.rs` | Anchor-validated existing-file edits. |
| `write` | `src/tools/write.rs` | Creates new files; overwrite requires `overwrite=true`. |
| `grep` | `src/tools/grep.rs` | Regex/literal content search. |
| `find` | `src/tools/find.rs` | Glob file search. |
| `bash` | `src/tools/bash.rs` | Agent shell command execution. |
| `ask_user` | `src/tools/ask_user.rs` | Blocking focused user question. |

### Hashline editing

Hashline editing is the preferred mutation path for existing files.

- `read` returns text lines as `LINE#HASH:content`.
- `hashline_edit` accepts anchors copied from `read`.
- Anchors are validated against the current file before mutation.
- Stale anchors fail with `[E_STALE_ANCHOR]` and fresh nearby retry anchors.
- Replacement `lines` must be literal file content: no copied `LINE#HASH:` prefixes and no diff `+`/`-` prefixes.
- Edits are validated against one pre-edit snapshot and applied bottom-up.
- Successful edits return a compact diff preview, updated anchors, and structured metadata.
- The legacy exact-text `edit` tool has been removed; use `hashline_edit` with `replace_text` for exact unique substring replacement when necessary.

Hashline modules live under `src/tools/hashline/`:

| File | Responsibility |
|---|---|
| `hash.rs` | Hash alphabet, xxHash32-style line hashing, rendering. |
| `parse.rs` | Anchor parsing and replacement-line validation. |
| `read.rs` | Hashline read preview formatting. |
| `edit.rs` | Tool schema, validation, edit application, response metadata. |
| `diff.rs` | Compact diff preview and changed-span detection. |
| `file_kind.rs` | Text/binary/image/directory classification. |
| `atomic_write.rs` | Atomic temp-write/rename with permission preservation. |
| `queue.rs` | Per-file mutation serialization. |
| `metadata.rs` | Placeholder for future typed metadata structs. |

## Providers

Providers implement `src/providers/mod.rs::LlmProvider` and stream normalized `ProviderEvent` values.

| Provider | Module | Notes |
|---|---|---|
| Fake | `src/providers/fake.rs` | Scriptable, no network, used by tests. |
| OpenAI Responses | `src/providers/openai_responses.rs` | SSE streaming via `reqwest`. |
| DeepSeek-compatible | `src/providers/deepseek_compat.rs` | Chat-completions SSE adapter. |

Relevant env vars:

- `OPENAI_API_KEY`, `OPENAI_BASE_URL`
- `DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL`
- `CROMULENT_FAKE_RESPONSE`, `CROMULENT_FAKE_DELAY_MS`

## Session persistence

Sessions are stored as JSONL files in `~/.cromulent/sessions/` by default, or `--sessions-dir <path>`.

A session file starts with a header entry, followed by transcript messages. The header stores schema version, cwd, model, thinking level, timestamps, and title/session metadata.

Session features:

- create/load/list
- update header
- append messages
- fork up to an entry
- export/import portable JSON

## Testing

Run all tests:

```bash
cargo test
```

Useful targeted runs:

```bash
cargo test --test protocol_jsonl
cargo test --test tools
cargo test --test sessions
cargo test --test ask_user_flow
cargo test --test cancellation
cargo test --test providers
cargo test --test config_merge
```

Major test files:

| Suite | Covers |
|---|---|
| `tests/protocol_jsonl.rs` | Command/event/message serde, including tool metadata. |
| `tests/tools.rs` | Built-in tools, hashline read/edit behavior, registry defaults. |
| `tests/sessions.rs` | Session create/load/update/fork. |
| `tests/ask_user_flow.rs` | Ask registration, resolution, cancellation. |
| `tests/cancellation.rs` | Cancellation token and state transitions. |
| `tests/providers.rs` | Provider manager/adapters without live network. |
| `tests/config_merge.rs` | Config loading and CLI/default merge behavior. |

Before committing larger changes, run `cargo fmt && cargo test`.

## Project structure

```text
cromulent/
├── Cargo.toml
├── README.md
├── DEVELOPMENT.md
├── src/
│   ├── main.rs                 # CLI, startup, top-level daemon loop
│   ├── lib.rs                  # Library crate used by tests
│   ├── app/
│   │   ├── runtime.rs          # Central orchestrator and command handlers
│   │   ├── state.rs            # AppState, RunState, AppConfig
│   │   ├── router.rs           # Command dispatch
│   │   └── output.rs           # Event/response helpers
│   ├── protocol/
│   │   ├── commands.rs         # ClientCommand enum
│   │   ├── events.rs           # ServerEvent enum
│   │   ├── responses.rs        # CommandResponse, StateSnapshot
│   │   └── types.rs            # Shared protocol/model/message types
│   ├── transport/
│   │   ├── reader.rs           # Stdin JSONL reader
│   │   └── writer.rs           # Stdout JSONL writer
│   ├── agent/
│   │   ├── runner.rs           # Agent turn loop and tool execution
│   │   ├── transcript.rs       # Message ↔ LlmMessage conversion
│   │   └── prompt.rs           # System prompt builder
│   ├── providers/
│   │   ├── mod.rs              # LlmProvider trait + ProviderManager
│   │   ├── fake.rs             # Scriptable fake provider
│   │   ├── openai_responses.rs
│   │   └── deepseek_compat.rs
│   ├── tools/
│   │   ├── registry.rs         # Tool trait + ToolRegistry
│   │   ├── read.rs
│   │   ├── write.rs
│   │   ├── grep.rs
│   │   ├── find.rs
│   │   ├── bash.rs
│   │   ├── ask_user.rs
│   │   └── hashline/           # Anchor-based read/edit subsystem
│   ├── session/
│   │   ├── store.rs            # Session JSONL persistence
│   │   ├── export.rs           # Portable JSON export/import
│   │   └── fork.rs             # Session forking helper
│   ├── auth/
│   │   ├── config.rs           # App config file
│   │   └── codex.rs            # Codex credential cache
│   ├── process/
│   │   └── bash_runner.rs      # UI raw bash execution
│   └── util/
│       ├── ids.rs              # ID generation
│       ├── time.rs             # ISO timestamps
│       └── fs.rs               # Filesystem helpers
└── tests/
    ├── protocol_jsonl.rs
    ├── tools.rs
    ├── sessions.rs
    ├── ask_user_flow.rs
    ├── cancellation.rs
    ├── providers.rs
    └── config_merge.rs
```

## Development tips

- Keep stdout reserved for protocol JSONL; use stderr/tracing for diagnostics.
- Add protocol fields with serde defaults/skip rules where possible to preserve compatibility.
- Tool descriptions are part of model behavior; update `src/agent/prompt.rs` and tool schemas together.
- Prefer structured metadata for host/UI data; keep model-visible tool text concise and actionable.
- Avoid direct existing-file overwrites; use `hashline_edit` and atomic write paths.
