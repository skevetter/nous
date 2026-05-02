# Multi-Provider LLM Design

Extends the rig adoption (see `rig-adoption.md`) from Bedrock-only to support
multiple LLM providers: AWS Bedrock, Anthropic direct API, and OpenAI. Designed
for a flexible architecture that makes adding future providers trivial.

---

## 1. Current State

### Single provider: AWS Bedrock via rig-bedrock

The rig adoption from `rig-adoption.md` has been completed.
`crates/nous-daemon/src/llm_client.rs` now re-exports `rig_bedrock::client::Client`
as `LlmClient` and provides:

- `LlmConfig` struct with `model`, `region`, `profile` fields
- `LlmConfig::resolve()` implementing the layered config pattern:
  CLI flags > env vars > `~/.config/nous/config.toml` > hardcoded defaults
- `build_client()` async constructor using `ClientBuilder` or `Client::with_profile_name()`
- Default model: `anthropic.claude-sonnet-4-20250514-v1:0`

### AppState wiring

`crates/nous-daemon/src/state.rs` holds:

```rust
pub struct AppState {
    // ...
    pub llm_client: Option<Arc<LlmClient>>,  // LlmClient = rig_bedrock::client::Client
    pub default_model: String,
}
```

The `Option` pattern allows the daemon to start without LLM credentials; LLM
invocations return a config error when `llm_client` is `None`.

### Dispatch: process_manager.rs

`ProcessRegistry::invoke()` reads `agent.process_type` from the database and
dispatches:

```rust
match agent.process_type.as_deref() {
    Some("claude") => self.invoke_claude(state, ...).await,
    Some("shell") | None => self.invoke_shell(state, ...).await,
    Some(other) => Err(NousError::Config("unsupported process_type")),
}
```

`invoke_claude()` builds a rig `Agent` per-invocation using the Bedrock client:

```rust
let agent = client.agent(&model).preamble(&preamble).build();
let output = agent.prompt(prompt).await;
```

### Client initialization sites

Three places build `AppState` with LLM configuration:

| File | Context |
|---|---|
| `crates/nous-daemon/src/main.rs` | Standalone daemon entry point |
| `crates/nous-cli/src/commands/serve.rs` | CLI `nous serve` / `nous start` |
| `crates/nous-cli/src/commands/mcp_server.rs` | CLI `nous mcp-server` |

All three follow the same pattern: resolve `LlmConfig`, check for AWS
credentials, call `build_client()`, wrap in `Option<Arc<...>>`.

### Dependencies

```toml
# crates/nous-daemon/Cargo.toml
rig-bedrock = "0.4.5"
rig-core = "0.36"

# crates/nous-cli/Cargo.toml
rig-core = "0.36"
```

`rig-core` is also a transitive dependency of `rig-bedrock`. The CLI uses
`rig-core` directly for the `ProviderClient` trait (`from_env()`).

---

## 2. Target State

### Multiple providers, one dispatch interface

`AppState` holds one optional client per provider. The `process_type` field on
an agent (or override in invocation metadata) selects which provider to use.
All providers go through rig's `CompletionClient` / `Agent` / `Prompt` traits,
so the invocation logic is identical regardless of backend.

### Supported providers (initial)

| Provider | rig crate | Client type | Auth |
|---|---|---|---|
| AWS Bedrock | `rig-bedrock` | `rig_bedrock::client::Client` | AWS SSO / env creds |
| Anthropic direct | `rig-core` built-in | `rig::providers::anthropic::Client` | `ANTHROPIC_API_KEY` |
| OpenAI | `rig-core` built-in | `rig::providers::openai::Client` | `OPENAI_API_KEY` |

Future providers (Google Vertex, Mistral, Cohere, local Ollama, etc.) follow
the same pattern -- `rig-core` ships 25+ built-in providers, and additional
providers are available as separate crates.

---

## 3. Provider Abstraction Layer

### What rig already provides

rig-core defines a trait hierarchy that all providers implement:

```
CompletionClient  -->  AgentBuilder<M>  -->  Agent<M>  -->  Prompt trait
```

- `CompletionClient::agent(model_id) -> AgentBuilder<M>` -- fluent builder
- `AgentBuilder::preamble()`, `.temperature()`, `.max_tokens()`, `.tool()`, `.build() -> Agent<M>`
- `Agent<M>` implements `Prompt` trait: `.prompt(text).await -> Result<String>`

Because `Agent<M>` is generic over the model type `M`, the dispatch code must
be provider-aware at the point of building the agent. However, the
prompt-execute-timeout-record pattern is identical across all providers.

### What nous needs: a thin provider registry

Rather than a trait object (which would require `dyn CompletionClient` --
not object-safe in rig), nous uses an enum-based dispatch:

```rust
// crates/nous-daemon/src/llm_client.rs

pub enum LlmProvider {
    Bedrock(Arc<rig_bedrock::client::Client>),
    Anthropic(Arc<rig::providers::anthropic::Client>),
    OpenAi(Arc<rig::providers::openai::Client>),
}
```

Each variant wraps an `Arc<Client>` for cheap cloning into async tasks.

A shared helper extracts the prompt-timeout-record pattern:

```rust
async fn run_prompt(
    provider: &LlmProvider,
    model: &str,
    preamble: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, NousError> {
    let result = tokio::time::timeout(timeout, async {
        match provider {
            LlmProvider::Bedrock(c) => {
                let agent = build_agent(c, model, preamble);
                agent.prompt(prompt).await.map_err(|e| e.to_string())
            }
            LlmProvider::Anthropic(c) => {
                let agent = build_agent(c, model, preamble);
                agent.prompt(prompt).await.map_err(|e| e.to_string())
            }
            LlmProvider::OpenAi(c) => {
                let agent = build_agent(c, model, preamble);
                agent.prompt(prompt).await.map_err(|e| e.to_string())
            }
        }
    }).await;
    // handle timeout / error ...
}
```

Where `build_agent` is a generic helper:

```rust
fn build_agent<C: CompletionClient>(
    client: &C,
    model: &str,
    preamble: &str,
) -> Agent<C::CompletionModel> {
    let mut builder = client.agent(model);
    if !preamble.is_empty() {
        builder = builder.preamble(preamble);
    }
    builder.build()
}
```

This keeps the pattern DRY while respecting rig's generic type system.

---

## 4. Configuration Model

### Layered resolution (unchanged principle)

CLI flags > env vars > `~/.config/nous/config.toml` > hardcoded defaults

### New config fields

#### CLI flags

```
nous serve --provider bedrock --model anthropic.claude-sonnet-4-20250514-v1:0 --region us-east-1 --profile h3-dev
nous serve --provider anthropic --model claude-sonnet-4-5
nous serve --provider openai --model gpt-4o
```

`--provider` selects the backend. `--model`, `--region`, `--profile` remain as
today but `--region` and `--profile` are only meaningful for Bedrock.

#### Environment variables

| Variable | Provider | Purpose |
|---|---|---|
| `NOUS_LLM_PROVIDER` | all | `bedrock`, `anthropic`, or `openai` |
| `NOUS_LLM_MODEL` | all | Model ID override |
| `AWS_REGION` | bedrock | AWS region |
| `AWS_PROFILE` | bedrock | AWS SSO profile |
| `AWS_ACCESS_KEY_ID` | bedrock | Static credentials |
| `AWS_SECRET_ACCESS_KEY` | bedrock | Static credentials |
| `AWS_SESSION_TOKEN` | bedrock | Session token |
| `ANTHROPIC_API_KEY` | anthropic | API key |
| `OPENAI_API_KEY` | openai | API key |

#### Config file: `~/.config/nous/config.toml`

```toml
[llm]
provider = "bedrock"          # "bedrock" | "anthropic" | "openai"
model = "anthropic.claude-sonnet-4-20250514-v1:0"

# Bedrock-specific
region = "us-east-1"
profile = "h3-dev"

# Anthropic-specific (API key via ANTHROPIC_API_KEY env var, not stored in config)
# anthropic_api_key_env = "ANTHROPIC_API_KEY"   # for documentation only

# OpenAI-specific (API key via OPENAI_API_KEY env var, not stored in config)
# openai_api_key_env = "OPENAI_API_KEY"         # for documentation only
```

API keys are **never** stored in the config file. They must come from
environment variables. The config file only stores non-secret settings like
provider name, model ID, and region.

### Updated `LlmConfig`

```rust
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: ProviderKind,     // new
    pub model: String,
    pub region: String,             // bedrock only
    pub profile: Option<String>,    // bedrock only
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Bedrock,
    Anthropic,
    OpenAi,
}
```

`LlmConfig::resolve()` gains a `cli_provider: Option<String>` parameter and
resolves `ProviderKind` from CLI > `NOUS_LLM_PROVIDER` env var > config file >
default (`Bedrock`).

### Auto-detection fallback

If `provider` is not explicitly set, nous can auto-detect based on available
credentials:

1. If `AWS_ACCESS_KEY_ID` or `AWS_PROFILE` is set -> `Bedrock`
2. Else if `ANTHROPIC_API_KEY` is set -> `Anthropic`
3. Else if `OPENAI_API_KEY` is set -> `OpenAi`
4. Else -> no LLM client (same as today)

This provides zero-config UX for users who only have one provider configured.

---

## 5. Credential Management Per Provider

### AWS Bedrock

No change from today. Uses `aws-config`'s credential chain via
`rig_bedrock::client::Client`:

- `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` + optional `AWS_SESSION_TOKEN`
- `AWS_PROFILE` for SSO (dev testing: `AWS_PROFILE=h3-dev`)
- `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` for ECS
- Instance metadata for EC2

### Anthropic Direct

```rust
use rig::providers::anthropic;
let client = anthropic::Client::from_env();  // reads ANTHROPIC_API_KEY
```

`rig::providers::anthropic::Client::from_env()` reads `ANTHROPIC_API_KEY` from
the environment. No additional configuration needed.

For explicit construction:
```rust
let client = anthropic::Client::new("sk-ant-...");
```

### OpenAI

```rust
use rig::providers::openai;
let client = openai::Client::from_env();  // reads OPENAI_API_KEY
```

`rig::providers::openai::Client::from_env()` reads `OPENAI_API_KEY` from the
environment.

### Security considerations

- API keys must only come from environment variables, never from config files or
  CLI arguments (which would appear in process listings).
- The `nous doctor` command should validate credential availability for the
  configured provider without exposing the key values.
- Log messages must never include API key values. The current logging in
  `serve.rs` and `mcp_server.rs` only logs region/model, not credentials.

---

## 6. AppState Changes

### New fields

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub vec_pool: VecPool,
    pub registry: Arc<NotificationRegistry>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub schedule_notify: Arc<Notify>,
    pub shutdown: CancellationToken,
    pub process_registry: Arc<ProcessRegistry>,

    // LLM provider clients (each optional, set based on available credentials)
    pub bedrock_client: Option<Arc<rig_bedrock::client::Client>>,
    pub anthropic_client: Option<Arc<rig::providers::anthropic::Client>>,
    pub openai_client: Option<Arc<rig::providers::openai::Client>>,

    pub default_provider: ProviderKind,
    pub default_model: String,
}
```

The single `llm_client: Option<Arc<LlmClient>>` field is replaced by three
provider-specific optional clients. `default_provider` indicates which one is
the primary.

### Initialization pattern

At startup, each provider is initialized independently. Failure to initialize
one provider does not block others:

```rust
let bedrock_client = if has_aws_credentials {
    Some(Arc::new(build_bedrock_client(&llm_config).await))
} else {
    tracing::info!("Bedrock: no AWS credentials found");
    None
};

let anthropic_client = match std::env::var("ANTHROPIC_API_KEY") {
    Ok(_) => {
        let client = rig::providers::anthropic::Client::from_env();
        tracing::info!("Anthropic: API key configured");
        Some(Arc::new(client))
    }
    Err(_) => {
        tracing::info!("Anthropic: ANTHROPIC_API_KEY not set");
        None
    }
};

let openai_client = match std::env::var("OPENAI_API_KEY") {
    Ok(_) => {
        let client = rig::providers::openai::Client::from_env();
        tracing::info!("OpenAI: API key configured");
        Some(Arc::new(client))
    }
    Err(_) => {
        tracing::info!("OpenAI: OPENAI_API_KEY not set");
        None
    }
};
```

---

## 7. Dispatch Changes (process_manager.rs)

### Updated process_type mapping

| `process_type` | Provider | Default model |
|---|---|---|
| `"claude"` or `"bedrock"` | Bedrock | `anthropic.claude-sonnet-4-20250514-v1:0` |
| `"anthropic"` | Anthropic direct | `claude-sonnet-4-5` |
| `"openai"` | OpenAI | `gpt-4o` |
| `"shell"` or `None` | subprocess | n/a |

### Updated dispatch

```rust
match agent.process_type.as_deref() {
    Some("claude") | Some("bedrock") => {
        self.invoke_with_provider(state, &LlmProvider::Bedrock(...), ...).await
    }
    Some("anthropic") => {
        self.invoke_with_provider(state, &LlmProvider::Anthropic(...), ...).await
    }
    Some("openai") => {
        self.invoke_with_provider(state, &LlmProvider::OpenAi(...), ...).await
    }
    Some("shell") | None => {
        self.invoke_shell(state, ...).await
    }
    Some(other) => Err(NousError::Config(format!("unsupported process_type '{other}'"))),
}
```

### Metadata-driven provider override

In addition to agent-level `process_type`, invocation metadata can override the
provider for a single call:

```json
{
  "provider": "anthropic",
  "model": "claude-sonnet-4-5",
  "preamble": "You are a code reviewer."
}
```

This lets agents experiment with different providers without changing their
registration.

### Shared invoke pattern

The current `invoke_claude()` method is ~140 lines with duplicated async/sync
paths. With multi-provider support, this is refactored into a shared helper:

```rust
async fn invoke_with_provider(
    &self,
    state: &AppState,
    provider: &LlmProvider,
    invocation: &Invocation,
    prompt: &str,
    timeout_secs: Option<i64>,
    metadata: &Option<serde_json::Value>,
    is_async: bool,
) -> Result<Invocation, NousError>
```

The async/sync branching and timeout/status-update logic is provider-agnostic
and handled by this single method, eliminating the need for separate
`invoke_bedrock()`, `invoke_anthropic()`, `invoke_openai()` methods.

---

## 8. Dependency Impact

### Cargo.toml changes

```toml
# crates/nous-daemon/Cargo.toml

[dependencies]
rig-bedrock = "0.4.5"      # existing
rig-core = "0.36"           # existing -- anthropic + openai providers are built-in
```

No new crate dependencies are needed. `rig-core` already includes the Anthropic
and OpenAI providers. The only change is that nous will now use additional
modules from `rig-core` (`rig::providers::anthropic`, `rig::providers::openai`)
that were already compiled but unused.

### Feature flags for optional providers

For production deployments that only use Bedrock, the Anthropic and OpenAI
providers can be gated behind Cargo feature flags to avoid compiling unused
provider code:

```toml
[features]
default = ["bedrock"]
bedrock = ["rig-bedrock"]
anthropic = []     # uses rig-core built-in, no extra dep
openai = []        # uses rig-core built-in, no extra dep
all-providers = ["bedrock", "anthropic", "openai"]
```

The `bedrock` feature gates the `rig-bedrock` dependency. The `anthropic` and
`openai` features gate `#[cfg(feature = "...")]` blocks around their client
initialization and dispatch arms.

For the prototype phase, all providers are compiled unconditionally (feature
flags are a follow-up optimization). The `rig-core` dependency is always
present (transitive from `rig-bedrock`), so the Anthropic and OpenAI client
code is already compiled anyway.

### Compile time impact

Minimal. `rig-core` is already in the dependency graph. No new transitive
dependencies are introduced.

---

## 9. CLI Changes

### New `--provider` flag

Added to `Serve`, `Start`, and `McpServer` subcommands:

```rust
/// LLM provider (bedrock, anthropic, openai)
#[arg(long)]
provider: Option<String>,
```

### `nous doctor` provider diagnostics

The existing `doctor` command is extended to check each provider's credentials:

```
$ nous doctor

LLM Providers:
  bedrock:   OK (AWS_PROFILE=h3-dev, region=us-east-1)
  anthropic: OK (ANTHROPIC_API_KEY set)
  openai:    not configured (OPENAI_API_KEY not set)

Default provider: bedrock
Default model: anthropic.claude-sonnet-4-20250514-v1:0
```

---

## 10. Database Impact

No schema changes required. The `agents` table `process_type` column already
stores arbitrary strings. The new `"anthropic"` and `"openai"` values are
interpreted by the updated dispatch logic in `process_manager.rs`.

Existing agents with `process_type = "claude"` continue to work via the
`Some("claude") | Some("bedrock")` match arm.

---

## 11. Testing Strategy

### Unit tests: mock provider

rig-core 0.36 does not provide a mock client. Continue using the `FakeClient`
pattern from `rig-adoption.md` -- a struct implementing `CompletionModel` that
returns canned responses. Test the dispatch logic, timeout handling, and
status-update paths without network calls.

Extend with per-provider dispatch tests:

```rust
#[tokio::test]
async fn invoke_with_anthropic_process_type_dispatches_correctly() { ... }

#[tokio::test]
async fn invoke_with_openai_process_type_dispatches_correctly() { ... }

#[tokio::test]
async fn invoke_with_unknown_process_type_returns_config_error() { ... }

#[tokio::test]
async fn metadata_provider_override_uses_specified_provider() { ... }
```

### Integration tests: credential checks

```rust
#[tokio::test]
async fn anthropic_client_from_env_fails_without_key() {
    temp_env::with_vars([("ANTHROPIC_API_KEY", None::<&str>)], || {
        // verify client initialization fails gracefully
    });
}
```

### End-to-end tests: `#[ignore]`

Each provider gets a live round-trip test, gated by `#[ignore]`:

```rust
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn live_anthropic_round_trip() { ... }

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn live_openai_round_trip() { ... }

#[tokio::test]
#[ignore = "requires AWS credentials"]
async fn live_bedrock_round_trip() { ... }
```

### Config resolution tests

Verify the layered config resolution for the new `provider` field:

```rust
#[test]
fn resolve_provider_from_cli_flag() { ... }

#[test]
fn resolve_provider_from_env_var() { ... }

#[test]
fn resolve_provider_from_config_file() { ... }

#[test]
fn resolve_provider_defaults_to_bedrock() { ... }

#[test]
fn auto_detect_provider_from_anthropic_key() { ... }
```

---

## 12. Migration Plan

Each step is a standalone commit. The daemon compiles and tests pass after every
step.

### Step 1: Add `ProviderKind` enum and update `LlmConfig`

**File**: `crates/nous-daemon/src/llm_client.rs`

- Add `ProviderKind` enum: `Bedrock`, `Anthropic`, `OpenAi`
- Add `provider: ProviderKind` field to `LlmConfig`
- Update `LlmConfig::resolve()` to accept `cli_provider: Option<String>` and
  resolve from CLI > `NOUS_LLM_PROVIDER` > config file > auto-detect > `Bedrock`
- Add `LlmProvider` enum wrapping the three client types
- Add `build_provider()` function that constructs the appropriate client

No dispatch changes yet -- Bedrock path still works identically.

### Step 2: Update `AppState` to hold `LlmProvider`

**Files**: `crates/nous-daemon/src/state.rs`, `crates/nous-daemon/src/main.rs`

- Replace `llm_client: Option<Arc<LlmClient>>` with
  `llm_provider: Option<LlmProvider>` (or keep as `Arc<LlmProvider>`)
- Update all `AppState` constructors (3 production + 4 test sites)
- Bedrock initialization path unchanged; Anthropic and OpenAI paths added
  but only activate when their respective env vars are set

### Step 3: Refactor `invoke_claude` into `invoke_with_provider`

**File**: `crates/nous-daemon/src/process_manager.rs`

- Extract the shared agent-build / prompt / timeout / status-update pattern
  into `invoke_with_provider()`
- `invoke_claude()` becomes a thin wrapper that selects the Bedrock client
  from `state.llm_provider`
- Add `invoke_anthropic()` and `invoke_openai()` thin wrappers (or dispatch
  directly from the match)
- Update the `invoke()` dispatch match to include `"anthropic"` and `"openai"`
  arms

### Step 4: Add `--provider` CLI flag

**Files**: `crates/nous-cli/src/main.rs`, `commands/serve.rs`, `commands/mcp_server.rs`

- Add `--provider` argument to `Serve`, `Start`, `McpServer` subcommands
- Thread the value through to `LlmConfig::resolve()`
- Update help text to document the new flag

### Step 5: Add metadata-based provider override

**File**: `crates/nous-daemon/src/process_manager.rs`

- In `invoke()`, check `metadata.provider` before falling back to
  `agent.process_type`
- This allows per-invocation provider selection without changing agent config

### Step 6: Update `nous doctor` diagnostics

**File**: `crates/nous-cli/src/commands/doctor.rs`

- Add credential checks for all three providers
- Display which providers are available and which is the default

### Step 7: Tests

**Files**: `crates/nous-daemon/src/llm_client.rs` (unit),
`crates/nous-daemon/tests/` (integration)

- Add `ProviderKind` resolution tests
- Add dispatch tests for each provider type
- Add `#[ignore]` live round-trip tests for Anthropic and OpenAI
- Update existing test fixtures that construct `AppState` with the new fields

---

## 13. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| rig-core Anthropic/OpenAI providers have different API shape than expected | Low | Medium | Verify with `cargo doc -p rig-core --open` before implementation |
| rig-core Anthropic provider does not support all Claude features (extended thinking, etc.) | Medium | Low | Prototype phase -- basic prompt/response is sufficient |
| Credential leaks in logs | Low | High | Review all log sites; API keys never appear in `tracing::info!` |
| Compile time regression | Low | Low | No new crate deps; all code already compiled via rig-core |
| process_type string proliferation | Low | Low | Document valid values; validate in agent registration |

---

## 14. Future Considerations

### Model routing

A future iteration could add intelligent model routing -- selecting the
provider/model based on task characteristics (cost, latency, capability). This
would sit above the provider dispatch layer and is out of scope for this design.

### Streaming support

rig provides `agent.stream_prompt()` for all providers. Multi-provider streaming
follows the same enum dispatch pattern and can be added per-provider as needed.

### Tool use across providers

rig's `.tool()` builder method works identically across providers. Once tool
use is implemented for Bedrock (per `rig-adoption.md`), the same tools work
for Anthropic and OpenAI with no additional code.

### Provider-specific features

Some providers offer unique features (Bedrock's cross-region inference,
Anthropic's extended thinking, OpenAI's function calling format). These can be
exposed via provider-specific metadata fields in the invocation request without
polluting the shared abstraction.
