# rig Adoption Design

Replaces `crates/nous-daemon/src/llm_client.rs` with full adoption of the
[rig](https://github.com/0xPlaygrounds/rig) library, using rig's `Agent`
abstraction rather than raw `CompletionModel`. CEO directive: **full adoption**.

---

## 1. Current State

### `llm_client.rs` — 155 LOC manual Bedrock client

`crates/nous-daemon/src/llm_client.rs` implements a single-shot text-in/text-out
Bedrock client with no support for system prompts, tool use, multi-turn
conversation, streaming, or retries.

**`LlmClient` struct** (line 12):

| Field | Type | Purpose |
|---|---|---|
| `http_client` | `reqwest::Client` | raw HTTP |
| `region` | `String` | AWS region, defaults to `us-east-1` |
| `default_model` | `String` | defaults to `us.anthropic.claude-sonnet-4-20250514-v1:0` |
| `credentials` | `aws_credential_types::Credentials` | static key/secret/token |

**Construction** (`LlmClient::from_env`, line 20): reads
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`,
`AWS_REGION`/`AWS_DEFAULT_REGION`, and `NOUS_LLM_MODEL` from the environment.
Returns `Err(NousError::Config)` if the key variables are absent.

**Invocation** (`LlmClient::invoke`, line 50): builds a JSON body with a single
`user` message, signs it via `aws_sigv4::http_request::sign` using
`AWS4-HMAC-SHA256`, copies the `Authorization` and `x-amz-*` headers onto a
`reqwest` request, sends it to
`https://bedrock-runtime.{region}.amazonaws.com/model/{model_id}/converse`,
then parses `output.message.content[0].text` from the JSON response.

**Current limitations**:

- No system prompt (`preamble`) — the `messages` array always has a single user
  turn with no system role.
- No tool use — `build_request_body` produces a bare `messages` object with no
  `toolConfig` or `toolResults`.
- No multi-turn — each invocation is stateless; prior turns are discarded.
- No streaming — response is read as a full `text()` body.
- No retries — a single `reqwest_request.send()` call; HTTP errors surface as
  `NousError::Internal`.

### `invoke_claude` — dispatch in `process_manager.rs`

`ProcessRegistry::invoke` (line 416) reads `agent.process_type` from the
database and branches:

```
Some("claude") → invoke_claude(...)
Some("shell") | None → invoke_shell(...)
Some(other) → Err(NousError::Config("unsupported process_type"))
```

`invoke_claude` (line 609) resolves the model from `metadata.model` or
`llm_client.default_model`, then calls `llm_client.invoke(model, prompt)` inside
a `tokio::time::timeout`. Both sync and async paths write the result back via
`processes::update_invocation`.

### `AppState` wiring (`state.rs`, line 22)

```rust
pub llm_client: Option<Arc<LlmClient>>,
```

`Option` because AWS credentials may be absent. `main.rs` lines 41–50 call
`LlmClient::from_env()`, log success or failure, and wrap the result in
`Some(Arc::new(...))` or `None`.

## 2. Target State

`llm_client.rs` is deleted. `AppState` holds a `rig_bedrock::client::Client`
(wrapped in `Option<Arc<...>>`). Every invoke path goes through a rig `Agent`
built from that client.

### Component replacements

| Current | Replacement | Notes |
|---|---|---|
| `LlmClient` struct | `rig_bedrock::client::Client` | `Clone + Send + Sync`; no Arc needed for the inner client but we keep `Option<Arc<...>>` to preserve the "credentials optional" semantics |
| `LlmClient::from_env()` | `rig_bedrock::client::Client::from_env()` via `ProviderClient` trait | reads standard AWS env vars via `aws-config`; no manual credential construction |
| `sign_request()` | deleted — rig-bedrock handles SigV4 internally via `aws-sdk-bedrockruntime` | the `aws-sigv4` and `aws-credential-types` direct deps are removed |
| `build_request_body()` | deleted — rig serializes messages via its own types | |
| `parse_converse_response()` | deleted — rig deserializes and returns `String` | |
| `llm_client.invoke(model, prompt)` | `agent.prompt(prompt).await` | agent carries model ID and preamble |
| `state.llm_client: Option<Arc<LlmClient>>` | `state.llm_client: Option<Arc<rig_bedrock::client::Client>>` | field rename is the only `state.rs` change |

### New capabilities unlocked

- **System prompt**: set via `.preamble("...")` on the `AgentBuilder` — stored in
  the agent's config, prepended automatically on each call.
- **Tool use**: add tools via `.tool(impl Tool)` on the builder; rig routes
  `tool_use` blocks back to the Rust function automatically.
- **Multi-turn**: pass a `Vec<Message>` history through `agent.chat(...)` or
  manage context windows via `.dynamic_context(...)`.
- **Streaming**: call `agent.stream_prompt(prompt).await` returning
  `StreamingPromptRequest`; iterate with `while let Some(chunk) = stream.next().await`.
- **Structured extraction**: `client.extractor::<T>(model)` for typed JSON output.

### What does NOT change

- `ProcessRegistry::invoke` dispatch logic (match on `process_type` string).
- DB schema — `processes` and `invocations` tables are unchanged.
- The `Option<Arc<...>>` pattern — credentials still optional at startup.
- `tokio::time::timeout` wrapping — rig does not impose its own timeouts.

## 3. rig API Mapping

> **Note**: Import paths below are based on rig-core 0.36 / rig-bedrock 0.4.5 public API. Verify against your pinned version with `cargo doc --open` before use.

### Core traits

| rig type | Crate | Role in our system |
|---|---|---|
| `ProviderClient` trait | `rig-core` | Implemented by `rig_bedrock::client::Client`; provides `from_env()` and `from_val()` constructors |
| `CompletionClient` trait | `rig-core` | Implemented by `rig_bedrock::client::Client`; exposes `.agent(model_id)` → `AgentBuilder` |
| `AgentBuilder<M>` | `rig-core` | Fluent builder — call `.preamble()`, `.tool()`, `.temperature()`, `.max_tokens()`, `.build()` |
| `Agent<M>` | `rig-core` | Built agent; call `.prompt(text).await` → `Result<String, ...>` |
| `Prompt` trait | `rig-core` | `Agent` implements this; `.prompt(text).await` is the primary call site |
| `CompletionModel` trait | `rig-core` | Lower-level trait; `Agent` uses it internally but callers stay at the `Prompt` level |
| `Tool` trait | `rig-core` | Implement or annotate with `#[rig_tool]` (from `rig-derive` crate) to register function-calling tools with an agent |

### Agent builder — method reference

```rust
use rig::client::CompletionClient;
use rig_bedrock::client::Client;

let bedrock = Client::from_env();   // synchronous — reads AWS env vars immediately

let agent = bedrock
    .agent("us.anthropic.claude-sonnet-4-20250514-v1:0")
    .preamble("You are a nous process agent…")   // system prompt
    .temperature(0.0)                             // 0.0 for deterministic tasks
    .max_tokens(4096)                             // optional cap
    // .tool(my_tool)                             // register tools (future)
    .build();

let output: String = agent.prompt("Run analysis on X").await?;
```

### Mapping current call sites

| Current code | Replacement |
|---|---|
| `llm_client.invoke(&model, prompt)` (line 641, 687) | `agent.prompt(prompt).await` |
| `metadata.model` override | build per-invocation agent: `client.agent(&model).build()` |
| `llm_client.default_model` (line 629) | store `default_model: String` on `AppState` separately, or build a default agent at startup |
| `Arc<LlmClient>` (state.rs line 22) | `Arc<rig_bedrock::client::Client>` |

### Per-invocation model override pattern

The current code reads `metadata.model` to override the default model. With rig,
build a fresh `Agent` per invocation when the model differs:

```rust
let model = metadata.model.as_deref().unwrap_or(&state.default_model);
let agent = state.llm_client.as_ref()
    .ok_or_else(|| NousError::Config("LLM client not configured".into()))?
    .agent(model)
    .build();
let output = agent.prompt(prompt).await
    .map_err(|e| NousError::Internal(e.to_string()))?;
```

`AgentBuilder` is cheap to construct (no network call); the underlying
`rig_bedrock::client::Client` is `Clone` so sharing via `Arc` is safe.

## 4. Provider Strategy

### Minimum viable — Bedrock only

Add `rig-bedrock` as a direct dependency. Do not add `rig-core` explicitly; it
is a transitive dependency of `rig-bedrock` and re-exported from it.

```toml
# crates/nous-daemon/Cargo.toml
rig-bedrock = "0.4.5"
```

`rig-bedrock` uses `aws-sdk-bedrockruntime` + `aws-config` under the hood. The
`Client::from_env()` constructor calls `aws-config`'s
`aws_config::from_env().load().await`, which honours the same
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`,
`AWS_REGION`, and `AWS_PROFILE` variables the current `LlmClient::from_env`
reads. **No migration of env var names is required.**

Cross-region inference profiles (`us.*` model IDs) are passed straight through
to the Bedrock Converse API as the `modelId` parameter — rig-bedrock does not
rewrite model identifiers.

### Future providers (CEO directive: full adoption)

The CEO directive specifies full adoption of rig across providers, not just
Bedrock. The three providers to enable in the follow-on iterations:

| Provider | Crate | `Cargo.toml` |
|---|---|---|
| AWS Bedrock | `rig-bedrock` (separate crate) | `rig-bedrock = "0.4.5"` |
| Anthropic direct | via `rig-core` built-in provider | `rig-core = { version = "0.36", features = [] }` + `ANTHROPIC_API_KEY` |
| OpenAI | via `rig-core` built-in provider | `rig-core = { version = "0.36", features = [] }` + `OPENAI_API_KEY` |

`rig-core` ships 25 built-in providers. Anthropic and OpenAI are available
without extra crates:

```rust
// Anthropic
use rig::providers::anthropic;
let client = anthropic::Client::from_env();  // reads ANTHROPIC_API_KEY
let agent = client.agent("claude-sonnet-4-5").preamble("…").build();

// OpenAI
use rig::providers::openai;
let client = openai::Client::from_env();     // reads OPENAI_API_KEY
let agent = client.agent("gpt-4o").preamble("…").build();
```

All three provider clients implement `CompletionClient`, so `AgentBuilder` and
`Agent` behave identically regardless of provider. The `process_type` field
selects the provider at dispatch time (see Section 8).

### Feature flags — rig-core optional features

`rig-core` gated features to consider:

| Feature flag | Adds |
|---|---|
| *(none needed for basic Agent)* | Core agents, completion, tools, streaming included by default |
| `derive` | Enables `#[rig_tool]` attribute macro from `rig-derive` — required when annotating functions as tools |
| `lancedb` | Vector store integration (not needed for this migration) |
| `fastembed` | Local embedding (not needed — we use OnnxEmbeddingModel) |

No feature flags are required on `rig-bedrock` for its default Converse API usage.

## 5. Migration Plan

Each step is a standalone commit. Steps are ordered so the daemon compiles after
every step.

### Step 1 — Add dependency (`Cargo.toml`)

In `crates/nous-daemon/Cargo.toml`:

```toml
# Add
rig-bedrock = "0.4.5"

# Remove
aws-credential-types = "1.2"
aws-sigv4 = "1.4"
http = "1"
```

Leave `reqwest` in place — `axum` HTTP handlers still use it indirectly, and
`nous-core` tests reference it. Verify with `cargo check -p nous-daemon`.

### Step 2 — Replace `llm_client.rs`

Delete `crates/nous-daemon/src/llm_client.rs` and create a new file with:

```rust
// crates/nous-daemon/src/llm_client.rs
pub use rig_bedrock::client::Client as LlmClient;

pub const DEFAULT_MODEL: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";
```

Re-exporting as `LlmClient` keeps the type name stable across `state.rs` and
`main.rs` for the remainder of this migration. The constant is used in `main.rs`
to set `AppState::default_model`.

### Step 3 — Update `state.rs`

```rust
// Before (line 10, 22):
use crate::llm_client::LlmClient;
pub llm_client: Option<Arc<LlmClient>>,

// After — no import change needed (LlmClient re-export covers it):
pub llm_client: Option<Arc<LlmClient>>,
pub default_model: String,   // add this field
```

`default_model: String` is added so the model override logic in
`invoke_claude` no longer touches `llm_client.default_model` (which was a field
on the old struct, not on `rig_bedrock::client::Client`).

### Step 4 — Update `main.rs` (lines 41–50)

Replace the `LlmClient::from_env()` block:

```rust
// Before
let llm_client = match nous_daemon::llm_client::LlmClient::from_env() {
    Ok(client) => { … Some(Arc::new(client)) }
    Err(e) => { … None }
};

// After
use nous_daemon::llm_client::{LlmClient, DEFAULT_MODEL};
use rig::client::ProviderClient;

let (llm_client, default_model) = match LlmClient::from_env() {
    Ok(client) => {
        tracing::info!("LLM client configured for Bedrock");
        let model = std::env::var("NOUS_LLM_MODEL")
            .unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        (Some(Arc::new(client)), model)
    }
    Err(e) => {
        tracing::info!("LLM client not available: {e}");
        (None, DEFAULT_MODEL.to_string())
    }
};
```

Add `default_model` to the `AppState { … }` initializer on line 69.

### Step 5 — Update `process_manager.rs` (`invoke_claude`, lines 609–725)

Replace `llm_client.invoke(&model, prompt)` with an `Agent` call:

```rust
async fn invoke_claude(
    &self,
    state: &AppState,
    invocation: &Invocation,
    prompt: &str,
    timeout_secs: Option<i64>,
    metadata: &Option<serde_json::Value>,
    is_async: bool,
) -> Result<Invocation, NousError> {
    use rig::client::CompletionClient;
    use rig::completion::Prompt as _;

    let client = state.llm_client.as_ref().ok_or_else(|| {
        NousError::Config("LLM client not configured — set AWS credentials".into())
    })?;

    let model = metadata
        .as_ref()
        .and_then(|m| m.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or(&state.default_model)
        .to_string();

    let preamble = metadata
        .as_ref()
        .and_then(|m| m.get("preamble"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let agent = if preamble.is_empty() {
        client.agent(&model).build()
    } else {
        client.agent(&model).preamble(&preamble).build()
    };

    // … rest of timeout / async dispatch unchanged, replacing
    //   llm_client.invoke(&model, prompt)  →  agent.prompt(prompt).await
    //   map_err with NousError::Internal(e.to_string())
}
```

### Step 6 — Update the CLI `AppState` constructors

Seven sites across the workspace build `AppState` directly and must add the
`default_model` field:

| File | Line | Notes |
|---|---|---|
| `crates/nous-cli/src/commands/serve.rs` | 44 | CLI serve command |
| `crates/nous-cli/src/commands/mcp_server.rs` | 59 | CLI MCP server command |
| `crates/nous-daemon/src/lib.rs` | 148 | Test helper constructor |
| `crates/nous-daemon/src/main.rs` | 69 | Daemon entry point (already updated in Step 4) |
| `crates/nous-daemon/tests/integration.rs` | 21 | Integration test fixture |
| `crates/nous-daemon/tests/integration.rs` | 38 | Second integration test fixture |
| `crates/nous-daemon/tests/test_scheduler.rs` | 21 | Scheduler test fixture |

Search for any additional `AppState {` constructors introduced since this doc was written:

```bash
grep -rn --include='*.rs' "AppState {" crates/
```

Add `default_model: DEFAULT_MODEL.to_string()` (or a test-appropriate value such as `"test-model".to_string()`)
to each constructor.

### Step 7 — Delete dead code

Once all callers compile, delete:

- `crates/nous-daemon/src/llm_client.rs` original content (replaced in Step 2)
- `sign_request`, `build_request_body`, `parse_converse_response` (gone with
  the file)
- `converse_url` helper (gone)

Run `cargo test -p nous-daemon` to verify no regressions.

## 6. Dependency Impact

### `crates/nous-daemon/Cargo.toml` changes

| Action | Crate | Version |
|---|---|---|
| **Add** | `rig-bedrock` | `0.4.5` |
| **Remove** | `aws-credential-types` | `1.2` |
| **Remove** | `aws-sigv4` | `1.4` |
| **Remove** | `http` | `1` |
| Keep | `reqwest` | `0.12` (still used by axum layer) |

### New transitive dependencies from `rig-bedrock`

`rig-bedrock` pulls in:

| Crate | Purpose |
|---|---|
| `rig-core` | Agent, CompletionModel, Tool traits |
| `aws-sdk-bedrockruntime` | Bedrock Converse API (replaces our manual reqwest+sigv4 path) |
| `aws-config` | Credential chain loader (replaces manual env-var reading) |
| `aws-smithy-*` | AWS Smithy runtime (HTTP transport, retry, middleware) |
| `tokio-stream` | Streaming support |
| `serde`, `serde_json` | Already in workspace — versions must be compatible |

`aws-sdk-bedrockruntime` brings its own `aws-sigv4` as a transitive dep, so
signing continues to use `AWS4-HMAC-SHA256` — it just moves inside the SDK
layer.

### Compile time

Adding `rig-bedrock` adds approximately 30–50 crates to the dependency graph.
Expect an incremental clean build to take 2–4 minutes longer on a development
laptop (M-series Mac or equivalent x86 build machine). CI cold builds will
increase by a similar amount. Subsequent incremental builds are unaffected.

The `aws-sdk-bedrockruntime` crate is the largest single contributor to compile
time. If compile time becomes a blocker, the `rig-bedrock` dependency can be
gated behind a Cargo feature flag (`bedrock`) in `nous-daemon`'s manifest
so non-Bedrock CI jobs skip it.

### Version compatibility risks

- `rig-bedrock` 0.4.5 targets `rig-core` ^0.36.0. Check that workspace-level
  `serde` and `serde_json` versions satisfy both trees (they do at current
  workspace versions `1.x`).
- `tokio = "1"` (workspace) is compatible with rig-core's `tokio` requirement.
- `reqwest 0.12` (workspace) may conflict with rig-core if it pulls `reqwest
  0.11`. Inspect with `cargo tree -p nous-daemon -d` after adding rig-bedrock;
  resolve by aligning to a single version via workspace dep override if needed.

## 7. Testing Strategy

### What the existing tests cover

`crates/nous-daemon/src/llm_client.rs` lines 157–370 contain 15 unit tests:

| Test function | Group | Fate |
|---|---|---|
| `test_converse_url` | URL construction | deleted with module |
| `test_converse_url_different_region` | URL construction | deleted with module |
| `test_build_request_body` | JSON construction | deleted with module |
| `test_build_request_body_special_chars` | JSON construction | deleted with module |
| `test_parse_converse_response_valid` | JSON parsing | deleted with module |
| `test_parse_converse_response_empty_content` | JSON parsing | deleted with module |
| `test_parse_converse_response_missing_output` | JSON parsing | deleted with module |
| `test_parse_converse_response_invalid_json` | JSON parsing | deleted with module |
| `test_sign_request_produces_auth_header` | SigV4 signing | deleted with module |
| `test_sign_request_with_session_token` | SigV4 signing | deleted with module |
| `test_from_env_missing_access_key` | env-var parsing | replaced by rig-bedrock's own tests |
| `test_from_env_missing_secret_key` | env-var parsing | replaced by rig-bedrock's own tests |
| `test_from_env_success_with_defaults` | env-var parsing | replaced by rig-bedrock's own tests |
| `test_from_env_custom_region_and_model` | env-var parsing | replaced by rig-bedrock's own tests |
| `test_from_env_fallback_to_default_region` | env-var parsing | replaced by rig-bedrock's own tests |

None of these tests make real network calls. After replacing `llm_client.rs`,
these tests are gone. The coverage goal is to replace them with equivalent
tests that verify the dispatch layer, not rig's internals.

### Unit tests — mock provider via `FakeClient`

`rig::providers::mock` does not exist in rig-core 0.36. Implement a thin
`FakeClient` that satisfies the `CompletionModel` trait directly — this is the
guaranteed-to-compile approach and isolates the timeout/async/status-update
logic from network behaviour:

```rust
#[cfg(test)]
mod tests {
    use rig::completion::{CompletionModel, CompletionRequest, CompletionResponse};
    use rig::message::AssistantContent;
    use std::sync::Arc;

    struct FakeClient {
        response: String,
    }

    impl CompletionModel for FakeClient {
        type Response = CompletionResponse<()>;

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<()>, rig::completion::CompletionError> {
            Ok(CompletionResponse {
                choice: rig::completion::ModelChoice::Message(
                    AssistantContent::text(&self.response)
                ),
                raw_response: (),
            })
        }
    }

    #[tokio::test]
    async fn invoke_claude_returns_model_output() {
        let fake = Arc::new(FakeClient { response: "expected output".into() });
        // build AppState with llm_client: Some(fake) (wrapped per AppState type)
        // call invoke_claude with is_async = false
        // assert invocation.output == "expected output"
    }

    #[tokio::test]
    async fn invoke_claude_propagates_error() {
        // FakeClient returning Err exercises the error path
        // assert invocation.status == "failed"
    }
}
```

> **Note**: The `CompletionResponse` and `AssistantContent` field names above
> are based on rig-core 0.36 public API. Verify exact field names with
> `cargo doc -p rig-core --open` before use. rig-core may expose a mock
> convenience type in a future release, which would simplify this pattern.

This pattern keeps test setup in-process with no network calls or credential
requirements.

### Integration tests — `temp-env` + real `from_env`

Add a test that verifies `LlmClient::from_env()` returns `Err` when
`AWS_ACCESS_KEY_ID` is unset. Use `#[tokio::test]` with an inner synchronous
`temp_env::with_vars` scope — the `|| async { }.boxed()` closure pattern from
older `temp_env` versions does not compile with current async Rust:

```rust
#[tokio::test]
async fn from_env_fails_without_credentials() {
    let result = temp_env::with_vars(
        [("AWS_ACCESS_KEY_ID", None::<&str>),
         ("AWS_SECRET_ACCESS_KEY", None::<&str>)],
        || LlmClient::from_env(),
    );
    assert!(result.is_err());
}
```

`rig_bedrock::client::Client::from_env()` is synchronous and returns an error
when required AWS env vars are absent, matching the old `LlmClient::from_env`
behaviour.

### End-to-end — guarded by `#[ignore]`

Real API calls require credentials. Mark them with `#[ignore]` so CI skips them:

```rust
#[tokio::test]
#[ignore = "requires live AWS credentials"]
async fn live_agent_round_trip() {
    let client = LlmClient::from_env().unwrap();
    let agent = client.agent(DEFAULT_MODEL).build();
    let output = agent.prompt("Say 'ok'").await.unwrap();
    assert!(!output.is_empty());
}
```

Run locally with `cargo test -p nous-daemon -- --include-ignored live_agent`.

### No changes to `nous-core` or `nous-cli` tests

Neither crate imports `LlmClient` directly; their test suites are unaffected.

## 8. process_type Evolution

### Current dispatch (process_manager.rs, line 444)

```rust
match agent.process_type.as_deref() {
    Some("claude") => self.invoke_claude(…).await,
    Some("shell") | None => self.invoke_shell(…).await,
    Some(other) => Err(NousError::Config(format!("unsupported process_type '{other}'"))),
}
```

`process_type` is stored as `Option<String>` in the `agents` table and read
from `nous_core::agents::get_agent_by_id`.

### Mapping `process_type` strings to rig providers

With full rig adoption, `process_type` encodes both the provider and, optionally,
the model family. The recommended encoding:

| `process_type` value | rig provider | Client type | Credential source |
|---|---|---|---|
| `"claude"` or `"bedrock"` | AWS Bedrock | `rig_bedrock::client::Client` | `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` |
| `"anthropic"` | Anthropic direct | `rig::providers::anthropic::Client` | `ANTHROPIC_API_KEY` |
| `"openai"` | OpenAI | `rig::providers::openai::Client` | `OPENAI_API_KEY` |
| `"shell"` / `None` | subprocess | existing `invoke_shell` | n/a |

### AppState changes for multi-provider support

`AppState` should carry one optional client per provider. Add each as the
corresponding provider feature is implemented:

```rust
pub struct AppState {
    // … existing fields …
    pub default_model: String,
    pub llm_client: Option<Arc<rig_bedrock::client::Client>>,     // phase 1
    pub anthropic_client: Option<Arc<rig::providers::anthropic::Client>>,  // phase 2
    pub openai_client: Option<Arc<rig::providers::openai::Client>>,        // phase 2
}
```

Each client is initialised from its respective env vars at startup; absence
logs a warning and sets `None`.

### Updated dispatch

```rust
match agent.process_type.as_deref() {
    Some("claude") | Some("bedrock") => self.invoke_rig_bedrock(…).await,
    Some("anthropic") => self.invoke_rig_anthropic(…).await,
    Some("openai") => self.invoke_rig_openai(…).await,
    Some("shell") | None => self.invoke_shell(…).await,
    Some(other) => Err(NousError::Config(format!("unsupported process_type '{other}'"))),
}
```

Each `invoke_rig_*` function follows the same pattern as the migrated
`invoke_claude`: resolve model from metadata, build agent, call
`agent.prompt(prompt).await`, wrap result in `update_invocation`. The shared
timeout/async pattern can be extracted into a private helper:

```rust
async fn run_agent_prompt<M>(
    agent: rig::agent::Agent<M>,
    prompt: &str,
    timeout: Duration,
) -> Result<String, NousError>
where
    M: rig::completion::CompletionModel + Send + Sync + 'static,
```

This helper replaces the duplicated `tokio::time::timeout` + status-update
blocks across provider arms.

### Database migration

`process_type` values already in the database (`"claude"`, `"shell"`) remain
valid. No schema change is required. Add `"bedrock"` as an accepted alias for
`"claude"` in the dispatch match (already shown above) for agents created after
the rename.
