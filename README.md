# cromulent

**Headless coding agent daemon** — a single-binary Rust daemon that runs the
coding-agent loop behind a stable JSONL stdin/stdout protocol.

UIs (TUI, CLI, IDE plugin, web frontend) remain thin clients that send commands
and consume events, while `cromulent` owns LLM/provider sessions, tool
execution, transcript state, session persistence, cancellation, and blocking
human-in-the-loop (`ask_user`) interactions.

## Architecture

```text
stdin(JSONL) ──> TransportReader ──> CommandRouter ──> AppRuntime
                                                      │
                                                      ├── SessionManager
                                                      ├── AgentRunner
                                                      ├── ToolExecutor
                                                      ├── ProviderManager
                                                      ├── AskManager
                                                      └── BashRunner

AppRuntime ──> OutputQueue ──> TransportWriter ──> stdout(JSONL)
stderr ──────> structured debug logs only
```

### Key design properties

- **Single writer** — only one component writes to stdout, preventing
  interleaved JSONL.
- **Single owner** — `AppRuntime` is the sole mutator of runtime state;
  no cloned agents or split state.
- **Provider-agnostic** — all provider events are normalized into a
  `ProviderEvent` enum before they reach the agent loop.
- **Transcript-first** — append-only persistence with a rich session header
  that truly restores model, thinking level, and cwd.
- **Safe `ask_user`** — blocking human-in-the-loop is implemented via
  pending `oneshot` channels keyed by `askId`.
- **Unified cancellation** — one `CancellationToken` shared by provider
  streams, tool execution, and ask blocking.

## Protocol

All communication happens over **JSONL** (one JSON object per line) on
stdin/stdout. Every line is a self-describing object with a `type` field.

### Commands (stdin)

| Command           | Fields | Description |
|-------------------|--------|-------------|
| `prompt`          | `message` | Send a user message to the agent |
| `abort`           | — | Cancel the active run |
| `userResponse`    | `askId`, `response` | Resolve a pending `ask_user` |
| `setModel`        | `provider`, `modelId` | Change the active model (idle only) |
| `setThinking`     | `level` (`low`/`medium`/`high`) | Change thinking level (idle only) |
| `cycleModel`      | — | Cycle to next available model |
| `bash`            | `command` | Execute a raw shell command (UI-initiated, not agent) |
| `listSessions`    | — | List persisted session IDs |
| `loadSession`     | `sessionId` | Load a session from disk (idle only) |
| `newSession`      | — | Create and switch to a fresh session (idle only) |
| `forkSession`     | `entryId` | Fork transcript up to `entryId` into a new session |
| `getState`        | — | Return current session state snapshot |
| `getMessages`     | — | Return all messages in the current session |
| `exportSession`   | `outputPath` | Export session to portable JSON |
| `shutdown`        | — | Gracefully shut down the daemon |

### Events (stdout)

| Event              | Description |
|--------------------|-------------|
| `sessionChanged`   | Session, model, or thinking level changed |
| `agentStart`       | A prompt run has started |
| `turnStart`        | A new LLM turn has started |
| `textDelta`        | A chunk of generated text |
| `thinkingDelta`    | A chunk of reasoning/thinking text |
| `thinkingEnd`      | Thinking phase ended |
| `toolCall`         | The agent invoked a tool |
| `toolResult`       | The tool execution result |
| `ask`              | The agent is blocked waiting for user input |
| `error`            | A non-fatal runtime error |
| `turnEnd`          | A turn completed (with usage + stop reason) |
| `agentEnd`         | The full run completed |
| `bashOutput`       | Incremental stdout/stderr from raw bash |
| `bashDone`         | Raw bash command exited with code |

Every command with `id` gets exactly one synchronous `response` object back,
while all stream output arrives as events.

### Example flow

```text
> {"id":"1","type":"prompt","message":"Read src/main.rs"}
< {"id":"1","success":true,"data":{"runId":"run_abc123"}}
< {"type":"agentStart","runId":"run_abc123"}
< {"type":"turnStart","runId":"run_abc123","turn":1}
< {"type":"textDelta","runId":"run_abc123","text":"I'll","partial":"I'll"}
< {"type":"textDelta","runId":"run_abc123","text":" read","partial":"I'll read"}
< {"type":"toolCall","runId":"run_abc123","id":"call_1","name":"read","arguments":{"path":"src/main.rs"}}
< {"type":"toolResult","runId":"run_abc123","toolCallId":"call_1","content":[{"type":"text","text":"..."}],"isError":false}
< {"type":"textDelta","runId":"run_abc123","text":"Here","partial":"Here"}
< {"type":"turnEnd","runId":"run_abc123","turn":2,"stopReason":"completed","usage":{"inputTokens":45,"outputTokens":12}}
< {"type":"agentEnd","runId":"run_abc123","stopReason":"completed"}
```

### Commands reference

**prompt**
```json
{"id":"1","type":"prompt","message":"Write a Rust function to sort a list"}
```

**userResponse** (resolves a pending `ask` event)
```json
{"id":"2","type":"userResponse","askId":"ask_1","response":{"selected":["Option A"],"freeform":null,"comment":null}}
```

**setModel**
```json
{"id":"3","type":"setModel","provider":"openai","modelId":"gpt-5-codex"}
```

**bash** (UI-initiated, not agent tool)
```json
{"id":"4","type":"bash","command":"git status"}
```

## Setup

### Prerequisites

- **Rust 1.75+** — install via [rustup](https://rustup.rs/)
- **An LLM provider API key** (if using a real provider):
  - OpenAI: `OPENAI_API_KEY` environment variable
  - DeepSeek: `DEEPSEEK_API_KEY` environment variable

### Environment variables

| Variable | Purpose |
|----------|---------|
| `OPENAI_API_KEY` | Auth for the OpenAI Responses API adapter |
| `OPENAI_BASE_URL` | Optional OpenAI Responses API endpoint override |
| `DEEPSEEK_API_KEY` | Auth for the DeepSeek-compatible adapter |
| `DEEPSEEK_BASE_URL` | Optional DeepSeek chat completions endpoint override |
| `CODEX_ACCESS_TOKEN` | Seeds `auth/codex.json` when using `--setup-codex` |
| `CODEX_REFRESH_TOKEN` | Optional refresh token stored by `--setup-codex` |
| `CODEX_EXPIRES_AT` | Optional ISO 8601 expiry stored by `--setup-codex` |
| `CROMULENT_FAKE_RESPONSE` | Script default fake provider text |
| `CROMULENT_FAKE_DELAY_MS` | Add delay to fake provider chunks |

### Authenticate with Codex

`cromulent` can seed a local Codex credential cache from environment variables. This is useful when another tool or manual flow has already produced a token.

Credential cache location:

```text
~/.cromulent/auth/codex.json
```

Minimum setup:

```bash
CODEX_ACCESS_TOKEN="your-access-token" cargo run -- --setup-codex
```

Optional fields:

```bash
CODEX_ACCESS_TOKEN="your-access-token" \
CODEX_REFRESH_TOKEN="your-refresh-token" \
CODEX_EXPIRES_AT="2026-04-28T12:00:00Z" \
CODEX_SCOPE="openid profile email" \
cargo run -- --setup-codex
```

If `CODEX_ACCESS_TOKEN` is not set, `--setup-codex` creates the auth directory and prints instructions without writing credentials.

Current limitation: this does **not** perform browser OAuth/device-code sign-in yet. It only writes cached credentials from `CODEX_*` environment variables.

### Build

```bash
git clone <repo-url>
cd cromulent
cargo build --release
```

### Run

```bash
# Using configured defaults (config file + CLI overrides)
cargo run

# Start with a specific model/provider
OPENAI_API_KEY=sk-... cargo run -- --provider openai --model gpt-5.5
DEEPSEEK_API_KEY=sk-... cargo run -- --provider deepseek --model deepseek-chat

# Load an existing session
cargo run -- --session ses_abc123

# Set thinking level
cargo run -- --thinking high

# Custom working directory
cargo run -- --cwd /path/to/project

# Custom sessions and config paths
cargo run -- --sessions-dir /tmp/my-sessions --config /tmp/cromulent-config.json

# Seed Codex credentials from env vars
CODEX_ACCESS_TOKEN=... cargo run -- --setup-codex
```

### CLI options

```
Usage: cromulent [OPTIONS]

Options:
      --provider <PROVIDER>      Provider to use (overrides config default)
      --model <MODEL>            Model ID to use (overrides config default)
      --thinking <low|medium|high>
                                 Thinking level
      --session <SESSION_ID>     Session ID to load on startup
      --cwd <PATH>               Working directory (default: current dir)
      --max-turns <N>            Maximum turns per agent run
      --sessions-dir <PATH>      Directory for session persistence
      --config <PATH>            Config file path (default: ~/.cromulent/config.json)
      --setup-codex              Seed Codex credential cache from CODEX_* env vars and exit
  -h, --help                     Print help
  -V, --version                  Print version
```

### Configuration

On startup, `cromulent` loads `~/.cromulent/config.json` (or `--config <path>`) and then applies CLI overrides.

Example config:

```json
{
  "providers": {
    "openai": { "apiKeyEnv": "OPENAI_API_KEY", "defaultModel": "gpt-5.5" },
    "deepseek": { "apiKeyEnv": "DEEPSEEK_API_KEY", "defaultModel": "deepseek-chat" }
  },
  "defaultModel": {
    "provider": "openai",
    "id": "gpt-5.5",
    "displayName": "GPT-5.5",
    "contextWindow": 200000,
    "supportsReasoning": true,
    "supportsTools": true
  },
  "thinkingLevel": "medium",
  "maxTurns": 40
}
```

### Session persistence

Sessions are stored as JSONL files (one line per entry, header first) in:

- **Default**: `~/.cromulent/sessions/`
- **Override**: `--sessions-dir <path>`

## Testing

```bash
# Run all tests
cargo test

# Run specific test suites
cargo test protocol_jsonl
cargo test sessions
cargo test ask_user_flow
cargo test cancellation
cargo test auth

# Run with output (for debugging)
cargo test -- --nocapture
```

### Test suites

| Suite | Description |
|-------|-------------|
| `tests/protocol_jsonl.rs` | 40 serde roundtrip tests for all commands and events |
| `tests/sessions.rs` | 13 integration tests for session create/load/update/fork |
| `tests/ask_user_flow.rs` | 9 tests for pending ask register/resolve/cancel |
| `tests/cancellation.rs` | 12 tests for cancellation token and state transitions |
| `tests/providers.rs` | 17 tests for provider managers/adapters without live network |
| `tests/config_merge.rs` | 16 tests for config loading and CLI merge behavior |
| `tests/tools.rs` | 33 unit tests for built-in tools |
| `lib unit tests` | 46 tests across agent, auth, provider, session modules |

## Tools

The agent has access to these built-in tools:

| Tool | Description | Execution rules |
|------|-------------|-----------------|
| `read` | Read a text file with optional offset/limit | Prefer over `bash` for file reading |
| `write` | Write/overwrite a file, creating dirs | Use for new files |
| `edit` | Targeted text replacement in existing files | Requires unique `oldText`; reads before editing |
| `grep` | Regex/literal search with glob filtering | Prefer over `bash grep` |
| `find` | Glob-based file search | Prefer over `bash find` |
| `bash` | Execute a shell command | Auditable, cancellable |
| `ask_user` | Block for user input | Gathers context before asking |

## Providers

| Provider | Module | Status |
|----------|--------|--------|
| Fake (testing) | `providers/fake.rs` | ✅ Complete, scriptable for tests |
| OpenAI Responses API | `providers/openai_responses.rs` | ✅ Streaming adapter implemented (`OPENAI_API_KEY`) |
| DeepSeek Compatible | `providers/deepseek_compat.rs` | ✅ Streaming adapter implemented (`DEEPSEEK_API_KEY`) |

Both real adapters use `reqwest`, stream SSE responses, normalize deltas into `ProviderEvent`, and defensively convert HTTP/stream failures into error events so the agent loop does not hang.

The fake provider can be scripted via environment variables for integration testing:

```bash
CROMULENT_FAKE_RESPONSE="Hello from fake!" cargo run
CROMULENT_FAKE_DELAY_MS=50 cargo run
```

## Project structure

```
cromulent/
├── Cargo.toml
├── src/
│   ├── main.rs               # CLI, startup, command loop
│   ├── lib.rs                # Library crate for tests
│   ├── app/
│   │   ├── runtime.rs        # Central orchestrator
│   │   ├── state.rs          # AppState, RunState, AppConfig
│   │   ├── router.rs         # Command dispatch
│   │   └── output.rs         # Event/response helpers
│   ├── protocol/
│   │   ├── commands.rs       # ClientCommand enum
│   │   ├── events.rs         # ServerEvent enum
│   │   ├── responses.rs      # CommandResponse, StateSnapshot
│   │   └── types.rs          # Shared types (ModelInfo, Message, etc.)
│   ├── transport/
│   │   ├── reader.rs         # Stdin JSONL reader
│   │   └── writer.rs         # Stdout JSONL writer
│   ├── agent/
│   │   ├── runner.rs         # Normalized agent turn loop
│   │   ├── transcript.rs     # Message ↔ LlmMessage conversion
│   │   └── prompt.rs         # System prompt builder
│   ├── providers/
│   │   ├── mod.rs            # LlmProvider trait + ProviderManager
│   │   ├── fake.rs           # Scriptable fake provider
│   │   ├── openai_responses.rs
│   │   └── deepseek_compat.rs
│   ├── tools/
│   │   ├── registry.rs       # Tool trait + ToolRegistry
│   │   ├── read/write/edit/grep/find/bash/ask_user.rs
│   │   └── mod.rs            # ToolError
│   ├── session/
│   │   ├── store.rs          # Session persistence (JSONL)
│   │   ├── export.rs         # Portable JSON export/import
│   │   └── fork.rs           # Session forking helper
│   ├── auth/
│   │   ├── config.rs         # App config file
│   │   └── codex.rs          # Codex credential cache
│   ├── process/
│   │   └── bash_runner.rs    # Raw bash execution
│   └── util/
│       ├── ids.rs            # UUID-based ID generation
│       ├── time.rs           # ISO timestamp
│       └── fs.rs             # Directory resolution
└── tests/
    ├── protocol_jsonl.rs
    ├── sessions.rs
    ├── ask_user_flow.rs
    └── cancellation.rs
```

## License

MIT
