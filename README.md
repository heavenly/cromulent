# cromulent

**cromulent** is a headless coding-agent daemon: a single Rust binary that runs an LLM coding loop behind a stable JSONL stdin/stdout protocol.

Thin clients — TUI, CLI, IDE plugins, web frontends — send commands and consume events. The daemon owns provider sessions, tools, transcript state, session persistence, cancellation, and blocking human-in-the-loop prompts.

## Highlights

- JSONL protocol over stdin/stdout
- Provider adapters for OpenAI Responses, DeepSeek-compatible chat completions, and a fake test provider
- Persistent JSONL sessions with load/fork/export support
- Cancellable agent runs, tool calls, bash commands, and `ask_user` waits
- Built-in coding tools: `read`, `hashline_edit`, `write`, `grep`, `find`, `bash`, `ask_user`
- Hash-anchored editing: `read` returns `LINE#HASH:content`; `hashline_edit` validates anchors before mutating files

## Install / build

```bash
git clone https://github.com/heavenly/cromulent.git
cd cromulent
cargo build --release
```

## Run

```bash
# Default config / fake provider fallback
cargo run

# OpenAI
OPENAI_API_KEY=sk-... cargo run -- --provider openai --model gpt-5.5

# DeepSeek-compatible
DEEPSEEK_API_KEY=sk-... cargo run -- --provider deepseek --model deepseek-chat

# Set cwd, thinking, or session
cargo run -- --cwd /path/to/project --thinking high
cargo run -- --session ses_abc123
```

## Configuration

On startup, cromulent loads `~/.cromulent/config.json` unless `--config <path>` is provided. CLI flags override config values.

Common environment variables:

| Variable | Purpose |
|---|---|
| `OPENAI_API_KEY` | OpenAI Responses API auth |
| `OPENAI_BASE_URL` | Optional OpenAI endpoint override |
| `DEEPSEEK_API_KEY` | DeepSeek-compatible API auth |
| `DEEPSEEK_BASE_URL` | Optional DeepSeek endpoint override |
| `CROMULENT_FAKE_RESPONSE` | Fake provider response text |
| `CROMULENT_FAKE_DELAY_MS` | Fake provider chunk delay |

## Protocol quick example

All input/output is newline-delimited JSON.

```text
> {"id":"1","type":"prompt","message":"Read src/main.rs"}
< {"id":"1","success":true,"data":{"runId":"run_abc123"}}
< {"type":"agentStart","runId":"run_abc123"}
< {"type":"toolCall","runId":"run_abc123","id":"call_1","name":"read","arguments":{"path":"src/main.rs"}}
< {"type":"toolResult","runId":"run_abc123","toolCallId":"call_1","content":[{"type":"text","text":"1#MQ:fn main() {\n2#KT:}"}],"metadata":{"fileKind":"text"}}
< {"type":"agentEnd","runId":"run_abc123","stopReason":"completed"}
```

## Testing

```bash
cargo test
```

## Development docs

See [`DEVELOPMENT.md`](DEVELOPMENT.md) for architecture, protocol, testing, tools, and project-structure details.

## License

MIT
