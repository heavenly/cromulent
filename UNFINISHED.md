# Unfinished

This tracks remaining work after the phase 6–8 implementation pass.

## Done in phase 6–8

- Real provider adapters are implemented:
  - `src/providers/openai_responses.rs` streams OpenAI Responses API SSE and maps events to `ProviderEvent`.
  - `src/providers/deepseek_compat.rs` streams DeepSeek/OpenAI-compatible Chat Completions SSE and maps deltas/tool calls to `ProviderEvent`.
- Config loading is wired in `src/main.rs`:
  - loads `~/.cromulent/config.json` or `--config <path>`
  - merges CLI overrides for provider/model/thinking/max-turns
- `--setup-codex` now creates the auth directory and can seed `auth/codex.json` from `CODEX_ACCESS_TOKEN` and related env vars.
- `cycle_model` is implemented with a small built-in model list.
- Raw bash execution has cancellation support via `CancellationToken`.
- Transport reader no longer shuts down on malformed JSONL; it logs and continues.
- Provider/config/tool test coverage was added; `cargo test` passes.

---

## Remaining work

### P0 — Live provider validation

The OpenAI and DeepSeek adapters are implemented and tested without network, but they still need validation against real live endpoints.

Remaining:
- Verify exact OpenAI Responses API event names across text, reasoning, tool calls, usage, incomplete/failed responses.
- Verify DeepSeek tool-call streaming behavior with real tool invocation.
- Add opt-in live tests gated by env vars, e.g. `CROMULENT_LIVE_OPENAI=1`.
- Tune error handling for provider-specific rate limit and retry semantics.

Files:
- `src/providers/openai_responses.rs`
- `src/providers/deepseek_compat.rs`
- `tests/providers.rs`

### P1 — Golden JSONL transcript tests

There are serde tests and provider/tool tests, but no end-to-end golden tests that run the daemon/runtime through a full JSONL prompt flow and assert exact output lines.

Add tests for:
- `prompt → fake text → completed`
- `prompt → fake tool call → tool result → next turn`
- `prompt → ask → userResponse → resume`
- `prompt → abort during stream`
- `bash → bashOutput/bashDone`

### P1 — Runtime provider injection

`AgentRunner` currently creates `ProviderManager::default()` inside prompt execution. That works, but it prevents richer runtime/provider configuration from being injected into the running app.

Future improvement:
- Store `ProviderManager` in `AppRuntime`.
- Build provider adapters from `AppConfigFile` including custom `baseUrl` values.
- Avoid env-only auth in provider constructors where config says a different env var should be used.

### P1 — Config persistence/write-back

`set_model`, `set_thinking`, and `cycle_model` update the session header, but they do not persist defaults back to `~/.cromulent/config.json`.

Decide whether command-driven changes should:
- update only the current session (current behavior), or
- also update global defaults in the config file.

### P2 — Full Codex OAuth/token refresh

`--setup-codex` can seed credentials from env vars, and credential cache load/save exists. Full OAuth is not implemented.

Remaining:
- Browser/device-code sign-in flow.
- Token exchange and refresh.
- Expiry-aware provider auth integration.
- Secure storage considerations.

Files:
- `src/auth/codex.rs`
- `src/main.rs`

### P2 — Tool guardrails

Tools currently operate relative to cwd and validate many obvious errors, but there are no policy guardrails for protected paths.

Consider adding:
- deny writes/edits inside `.git/`
- deny writes to session/config/auth directories unless explicit
- file size limits for `read`, `grep`, and `edit`
- binary file detection
- symlink traversal policy

### P2 — Raw bash policy

Raw bash is cancellable, but there is no allow/deny policy, timeout, or audit metadata beyond emitted output.

Potential improvements:
- default timeout
- deny dangerous commands unless explicitly allowed
- structured command metadata in responses/events
- persistent command history if desired

### P3 — Warning cleanup

`cargo test` passes, but there are many warnings from unused future-facing APIs and test-only helpers.

Potential cleanup:
- gate test-only constructors under `#[cfg(test)]` or accept them as public embedding APIs
- remove stale unused imports in protocol modules
- document intentionally unused extension points

---

## Priority table

| Priority | Item | Area |
|----------|------|------|
| P0 | Validate real OpenAI/DeepSeek streams against live APIs | `providers/*` |
| P1 | Golden JSONL runtime transcript tests | `tests/` |
| P1 | Inject `ProviderManager` into `AppRuntime` | `app/runtime.rs`, `providers/mod.rs` |
| P1 | Decide/persist global config write-back | `app/runtime.rs`, `auth/config.rs` |
| P2 | Full Codex OAuth + refresh | `auth/codex.rs` |
| P2 | Tool path/size/symlink guardrails | `tools/*` |
| P2 | Bash timeout/policy/audit metadata | `process/bash_runner.rs` |
| P3 | Warning cleanup | all modules |
