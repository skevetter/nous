# Sandboxed Agent Deployment via microsandbox

NOUS-050 — sandboxing is **additive and optional**. Existing `claude`, `shell`, and `http` execution modes are unchanged. `process_type='sandbox'` is a fourth mode added alongside them.

---

## 1. Current Agent Execution Model (Unsandboxed)

`ProcessRegistry` (`crates/nous-daemon/src/process_manager.rs`) is the daemon's sole mechanism for running agent processes. Every spawn goes through `sh -c <command>` — the child process inherits the daemon's full OS environment with no namespace boundary.

### Key Files

| File | Relevant Lines | Role |
|---|---|---|
| `crates/nous-daemon/src/process_manager.rs` | L40–167 | `spawn()` — creates DB row, runs `sh -c`, stores child handle |
| `crates/nous-daemon/src/process_manager.rs` | L287–350 | `stop()` — sends SIGTERM, waits `grace_secs`, then SIGKILL |
| `crates/nous-daemon/src/state.rs` | L13–24 | `AppState` — holds `Arc<ProcessRegistry>` |
| `crates/nous-core/src/agents/processes.rs` | L10–57 | `Process` struct — maps `agent_processes` DB rows |
| `crates/nous-core/src/agents/mod.rs` | L188–207 | `Agent` struct — holds `process_type`, `spawn_command`, `working_dir` |

### Spawn Lifecycle

```
Agent registered in DB
  → nous agent spawn <agent_id> --command "..."
  → ProcessRegistry::spawn()
      creates agent_processes row (status: pending)
      updates status → starting
      Command::new("sh").arg("-c").arg(command).spawn()
      updates status → running (with PID)
      tokio::spawn(monitor_process(...))
  → monitor waits for child exit
      exit_code == 0 → status: stopped
      exit_code != 0 → status: crashed
  → nous agent stop <agent_id>
      → ProcessRegistry::stop()
          updates status → stopping
          SIGTERM (with grace period)
          SIGKILL if no exit within grace_secs
          updates status → stopped
```

### `process_type` Constraint

`agent_processes` migration 023 (`crates/nous-core/src/db/pool.rs` line 424):

```sql
process_type TEXT NOT NULL CHECK(process_type IN ('claude','shell','http'))
```

The dispatch in `invoke()` (`process_manager.rs` L464–483) branches on this field:

```rust
match agent.process_type.as_deref() {
    Some("claude") => self.invoke_claude(…).await,
    Some("shell") | None => self.invoke_shell(…).await,
    Some(other) => Err(NousError::Config(format!("unsupported process_type '{other}'")))
}
```

### Current Gaps Relevant to Sandboxing

| Gap | Location | Detail |
|---|---|---|
| In-memory handles lost on daemon restart | `ProcessRegistry.handles` (`Mutex<HashMap>`) | DB records survive restart; in-memory `ProcessHandle` entries do not — running processes become orphaned |
| No health check mechanism | — | No periodic liveness probe; a hung process stays `running` in DB indefinitely |

---

## 2. microsandbox Architecture and Integration Points

[microsandbox](https://github.com/microsandbox/microsandbox) wraps [libkrun](https://github.com/containers/libkrun) to run each workload inside a lightweight microVM. Boot time is under 100ms. No background daemon is required — the Rust SDK manages VM lifecycle in-process.

### System Requirements

- Linux with `/dev/kvm` accessible (no root needed — just `kvm` group membership)
- macOS Apple Silicon (libkrun uses Virtualization.framework)
- No Docker, no containerd, no privileged containers

### Rust SDK v0.4.2 Capabilities

| Feature | API | Notes |
|---|---|---|
| Create sandbox | `Sandbox::builder(name).image(img).cpus(n).memory_mib(m)` | Fluent builder |
| Detached mode | `.create_detached().await` | Persists after parent exits |
| Attached mode | `.create().await` | Dies when parent exits |
| Reconnect | `Sandbox::get(name).await` | Reconnects to a running detached sandbox |
| Run command | `sandbox.run("cmd", &["arg1"]).await` | Returns stdout/stderr/exit_code |
| Bind mount | `.volume("/guest/path", \|m\| m.bind("./host/path"))` | Host directory into guest |
| Named volume | `.volume("/data", \|m\| m.named("vol-name"))` | Persistent across restarts |
| Copy in | `sandbox.copy_to_host("guest/path", "host/path").await` | File exchange |
| Copy out | `sandbox.copy_from_host("host/path", "guest/path").await` | File exchange |
| Network | `.network(\|n\| n.public_only())` | Programmable per-packet policy |
| Secrets | `.secret("KEY", "value").allow_host("api.example.com")` | Placeholder substitution |
| Resource limits | `.cpus(2).memory_mib(512)` | Hard limits enforced by hypervisor |
| Metrics | `sandbox.metrics().await` | CPU/memory/IO usage |
| Stop | `sandbox.stop_and_wait().await` | Drains in-flight work, then terminates |
| Destroy | `sandbox.destroy().await` | Releases all resources |

### Lifecycle States

```
Creating ──► Running ──► Draining ──► Stopped
                 │
                 └──► Crashed
```

`Sandbox::get(name)` returns `Err` if the sandbox is not in `Running` state.

### Attached vs. Detached Mode

- **Attached** (`create()`): sandbox stops when the Rust process that created it exits. Suitable for one-shot agent tasks where the daemon manages the lifetime directly.
- **Detached** (`create_detached()`): sandbox persists independently. The daemon can exit and reconnect later via `Sandbox::get(name)`. Necessary for long-running agent processes that survive daemon restarts.

### Integration Points with ProcessRegistry

`ProcessRegistry::spawn()` currently branches on `process_type` only at the `invoke()` call site. The proposed integration adds a second branch at `spawn()` itself:

```
spawn(process_type = "sandbox")
  → SandboxManager::create(...)        # instead of Command::new("sh")
  → store sandbox name in process row  # instead of PID
  → monitor task polls sandbox.status() instead of child.wait()
stop(process_type = "sandbox")
  → SandboxManager::stop(...)          # instead of SIGTERM/SIGKILL
```

`SandboxManager` is a new struct (not a replacement for `ProcessRegistry`) that wraps the microsandbox SDK and is held on `AppState` behind `Option<Arc<SandboxManager>>`.

---

## 3. Proposed Sandbox Lifecycle

Each sandbox maps onto an `agent_processes` row and follows the same status progression as shell/claude processes — only the underlying mechanism differs.

### Status Mapping

| `agent_processes.status` | microsandbox state | Meaning |
|---|---|---|
| `pending` | Creating | SDK call issued, VM not yet booted |
| `starting` | Creating | VM booting (<100ms) |
| `running` | Running | VM is up, workload running |
| `stopping` | Draining | `stop_and_wait()` issued |
| `stopped` | Stopped | Clean exit |
| `crashed` | Crashed | Non-zero exit or VM fault |

### Rust API Sketch

```rust
// Create (called from ProcessRegistry::spawn when process_type == "sandbox")
let sandbox = Sandbox::builder(&sandbox_name)
    .image(&config.image)
    .cpus(config.cpus)
    .memory_mib(config.memory_mib)
    .create_detached()
    .await?;

// Configure bind mounts, network, secrets
sandbox
    .volume("/workspace", |m| m.bind(&config.working_dir))
    .network(|n| n.public_only())
    .secret("ANTHROPIC_API_KEY", &secret_val)
    .allow_host("api.anthropic.com");

// Start — sandbox boots, status → Running
// (create_detached() boots immediately; no separate start() call needed)

// Execute agent command inside the sandbox
let result = sandbox.run(&config.command, &[]).await?;

// Stop (called from ProcessRegistry::stop when process_type == "sandbox")
sandbox.stop_and_wait().await?;

// Destroy (called on agent deregister)
sandbox.destroy().await?;
```

### ProcessRegistry Branching

At `ProcessRegistry::spawn()` (L40–167) the `process_type` check gates two code paths:

```rust
match process_type {
    "sandbox" => {
        let sm = state.sandbox_manager.as_ref()
            .ok_or_else(|| NousError::Config("sandbox support not enabled".into()))?;
        sm.create_and_run(state, agent_id, &config).await
    }
    _ => {
        // Existing sh -c path — unchanged
        Command::new("sh").arg("-c").arg(command).spawn()
        …
    }
}
```

At `ProcessRegistry::stop()` (L287–350):

```rust
match process.process_type.as_str() {
    "sandbox" => sm.stop(state, agent_id).await,
    _ => {
        // Existing SIGTERM/SIGKILL path — unchanged
        libc::kill(pid, SIGTERM); …
    }
}
```

### Reconnect After Daemon Restart

Detached sandboxes survive daemon restarts. On startup, `SandboxManager::reconcile()` iterates `agent_processes` rows with `status IN ('running', 'starting')` and `process_type = 'sandbox'`, calls `Sandbox::get(sandbox_name)` for each, and either re-attaches the monitor task or marks the row `crashed` if the sandbox is no longer reachable. This fixes the in-memory-handles-lost gap noted in Section 1.

---

## 4. Git Repo Support Inside Sandboxes

Sandboxed agents cannot access the host filesystem unless a mount is explicitly declared. All git repo access goes through bind mounts or named volumes.

### Bind Mounts

```rust
// Read-write mount of the agent's working directory only
sandbox.volume("/workspace", |m| m.bind(&agent.working_dir).read_write());

// Read-only mount of the repo root for reference
sandbox.volume("/repo", |m| m.bind(&repo_root).read_only());
```

Mount only `agent.working_dir`, not the full host filesystem. An agent that only needs to read `main.rs` should not receive a mount that exposes `/etc/passwd`.

### Named Volumes

Named volumes persist across sandbox restarts. Use them for agent-generated outputs that must survive a sandbox stop/start cycle:

```rust
// Declare a named volume for build outputs
sandbox.volume("/workspace/.build", |m| m.named("agent-42-build-cache"));
```

Named volumes are owned by the sandbox manager and must be explicitly destroyed when the agent is deregistered.

### File Exchange

For controlled file transfers between host and guest that do not warrant a persistent mount:

```rust
// Push a config file into the guest before the agent starts
sandbox.copy_to_host("config.json", "/workspace/config.json").await?;

// Pull artifacts back after the agent completes
sandbox.copy_from_host("/workspace/output.tar.gz", "./artifacts/agent-42.tar.gz").await?;
```

### Scope Principle

| Mount target | Mode | Rationale |
|---|---|---|
| `agent.working_dir` | Read-write | Agent's primary workspace |
| Repo root | Read-only | Reference access without write risk |
| Named volume | Read-write | Persistent outputs, build caches |
| System paths (`/etc`, `/usr`, `/home`) | Not mounted | Not needed; attack surface reduction |

The host's git credential store, SSH keys, and AWS credentials are never accessible inside the sandbox unless explicitly mounted — and mounting them defeats the security purpose of sandboxing (see Section 6 for the recommended secrets approach instead).

---

## 5. Agent Definition Integration

Sandbox is **optional per-agent** and **per-spawn**. An agent registered with `process_type = 'shell'` runs unsandboxed. An agent registered with `process_type = 'sandbox'` runs in a microVM. Most agents do not need sandboxing — it is a per-spawn choice, not a platform requirement.

### Schema Change

Migration 026 adds `'sandbox'` to the `process_type` CHECK constraint:

```sql
-- Migration 026
ALTER TABLE agent_processes
  DROP CONSTRAINT IF EXISTS agent_processes_process_type_check;

ALTER TABLE agent_processes
  ADD CONSTRAINT agent_processes_process_type_check
  CHECK(process_type IN ('claude','shell','http','sandbox'));
```

SQLite does not support ALTER TABLE DROP CONSTRAINT — in practice this is a new table definition migration using the standard SQLite column-drop workaround (rename → create → copy → drop).

The `agents.process_type` column (migration 025) has no CHECK constraint, so no schema change is needed there.

### `SandboxConfig` Struct

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub image: String,
    pub cpus: u32,
    pub memory_mib: u32,
    pub network_policy: NetworkPolicy,   // public_only | none | custom
    pub volumes: Vec<VolumeMount>,
    pub secrets: Vec<SecretRef>,
    pub max_duration_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkPolicy {
    PublicOnly,
    None,
    Custom(Vec<String>),  // allowed hostnames
}
```

`SandboxConfig` is serialized to `env_json` on the `agent_processes` row when `process_type = 'sandbox'`.

### CLI Spawn Flags

```
nous agent spawn <agent_id> \
    --type sandbox \
    --sandbox-image ghcr.io/our-org/nous-base:latest \
    --sandbox-cpus 2 \
    --sandbox-memory-mib 512 \
    --sandbox-network public-only \
    --command "claude-code --task 'implement feature X'"
```

### Agent Forms

Agent definitions distributed as forms declare sandbox requirements in their metadata block:

```yaml
# agent-form.yaml
name: code-agent
process_type: sandbox
sandbox:
  image: ghcr.io/our-org/nous-base:latest
  cpus: 2
  memory_mib: 1024
  network_policy: public_only
tools:
  - bash
  - read
  - write
hooks:
  prompt: rig::PromptHook
```

The `tools` declaration follows the Agent Skills Specification (agentskills.io). The `hooks` block supports the rig `PromptHook` pattern for pre/post-prompt processing. Neither `tools` nor `hooks` require sandboxing — they apply to all `process_type` values.

### What Does Not Change

- Agents with `process_type IN ('claude', 'shell', 'http')` spawn identically to today.
- `ProcessRegistry` dispatch logic adds one branch; existing branches are untouched.
- The `agent_invocations` table and invocation flow are unchanged.
- `AppState` gains `Option<Arc<SandboxManager>>`, defaulting to `None` when microsandbox is not available.

---

## 6. Security Model and Resource Constraints

### Isolation Model: VM vs. Container

| Property | Current (unsandboxed) | Sandbox (microVM) |
|---|---|---|
| Kernel | Shared with host | Separate guest kernel |
| Filesystem | Full host access | Only explicitly mounted paths |
| Network | Host network stack | Userspace TCP/IP, per-packet policy |
| Namespace escape | Any container escape applies | No namespace — requires hypervisor escape |
| Privilege escalation | If daemon runs as root, agent inherits | Guest root != host root |
| Secrets access | All env vars visible to child | Only injected via `secret()` API |

### Network Policy

microsandbox implements a programmable per-packet network policy at the userspace TCP/IP stack level. Three policy presets:

| Policy | Allows | Blocks |
|---|---|---|
| `public_only()` | Outbound to public internet (LLM APIs, PyPI, npm) | Private RFC-1918 ranges, link-local |
| `none()` | Nothing | All outbound and inbound |
| `custom(hosts)` | Explicit hostname allowlist | Everything else |

Default recommendation: `public_only()`. Agents need to reach LLM providers (api.anthropic.com, bedrock-runtime.*.amazonaws.com) but must not reach internal services (databases, other agents' API endpoints, cloud metadata at 169.254.169.254).

### Secrets

```rust
sandbox
    .secret("ANTHROPIC_API_KEY", &secret_value)
    .allow_host("api.anthropic.com");   // DNS rebinding protection

sandbox
    .secret("AWS_SECRET_ACCESS_KEY", &aws_secret)
    .allow_host("bedrock-runtime.us-east-1.amazonaws.com");
```

Secrets are injected via placeholder substitution in the guest environment — the literal secret value is never written to disk or passed through the command string. `allow_host()` enforces that the secret is only transmitted to the declared hostname; outbound connections to other hosts cannot read injected secrets. Cloud metadata (169.254.169.254) is blocked when any secret is configured.

### Resource Limits

```rust
Sandbox::builder(name)
    .cpus(2)               // hard CPU cap — enforced by hypervisor
    .memory_mib(512)       // hard memory cap — OOM kills guest, not host
    .create_detached()
    .await?;
```

`max_duration_secs` is enforced by `SandboxManager` via `tokio::time::timeout` wrapping the monitor task. `idle_timeout_secs` requires the agent to emit a heartbeat; the monitor stops the sandbox if no heartbeat arrives within the window.

### Threat Model Summary

| Threat | Mitigated By |
|---|---|
| Agent reads host secrets from env | Secrets only via `secret()` API, never in env |
| Agent reads host filesystem | No mount = no access |
| Agent pivots to internal network | `public_only()` blocks RFC-1918 |
| Agent exhausts host memory | `memory_mib` cap — guest OOM stays in guest |
| Agent escapes via kernel bug | VM boundary requires hypervisor-level CVE |
| DNS rebinding to steal secrets | `allow_host()` enforces per-secret hostname |

---

## 7. Testing Strategy

### Feature Gating

The `sandbox` feature is gated behind a Cargo feature flag so the crate compiles on non-KVM environments (macOS Intel, CI without KVM):

```toml
# crates/nous-daemon/Cargo.toml
[features]
sandbox = ["microsandbox"]

[dependencies]
microsandbox = { version = "0.4.2", optional = true }
```

`SandboxManager` uses `#[cfg(feature = "sandbox")]`. When the feature is absent, `AppState.sandbox_manager` is always `None` and spawning with `process_type = 'sandbox'` returns `NousError::Config("sandbox feature not enabled")`.

### Test Categories

| Category | Approach | Runs In |
|---|---|---|
| Schema migration | Add `'sandbox'` to CHECK, verify row insert succeeds | Standard CI |
| `SandboxConfig` serde roundtrip | `serde_json::to_string` → `from_str` round-trip | Standard CI |
| CLI flag parsing | `--type sandbox --sandbox-image ... --sandbox-cpus ...` | Standard CI |
| `ProcessRegistry` dispatch | Mock `SandboxManager`, assert `spawn()` calls `sm.create_and_run()` | Standard CI (mock) |
| Sandbox stop lifecycle | Mock SM, assert `stop()` calls `sm.stop()` not `SIGTERM` | Standard CI (mock) |
| Reconnect after daemon restart | Mock SM returns pre-existing sandbox handle; assert monitor re-attaches | Standard CI (mock) |
| Real sandbox create/exec/stop | `Sandbox::builder(…).create_detached().await`; run `echo ok`; stop | KVM CI runner |
| Real sandbox network policy | `public_only()` blocks 10.0.0.1, allows api.anthropic.com | KVM CI runner |
| Real sandbox resource limits | `memory_mib(64)` — verify OOM stays in guest | KVM CI runner |

### Mock Layer Pattern

```rust
#[cfg(test)]
mod tests {
    use crate::sandbox::MockSandboxManager;

    #[tokio::test]
    async fn sandbox_process_type_routes_to_sandbox_manager() {
        let mock_sm = Arc::new(MockSandboxManager::new());
        let state = test_app_state(Some(mock_sm.clone()));
        let process = state.process_registry
            .spawn(&state, "agent-1", "echo ok", "sandbox", None, None, None, "never", 3)
            .await
            .unwrap();
        assert!(mock_sm.create_called());
        assert_eq!(process.process_type, "sandbox");
    }
}
```

### KVM CI Job

Add a separate CI job on a KVM-enabled runner. The job is skipped (not failed) on runners without `/dev/kvm`:

```yaml
# .gitlab-ci.yml
test-sandbox:
  tags: [kvm]
  script:
    - cargo test -p nous-daemon --features sandbox -- sandbox_integration
  rules:
    - if: $KVM_RUNNER == "true"
```

### Risk: KVM Availability

KVM is not available in standard containerized CI (GitHub Actions, GitLab shared runners). Mitigation: the mock layer covers all logic paths. KVM tests run only on dedicated hardware runners and are informational on PRs from forks.

---

## Open Questions

### 1. Secrets Management

**Question**: How do agents receive credentials (LLM API keys, git tokens) inside sandboxes?

**Recommendation**: Use the microsandbox `secret()` API with `allow_host()`.

```rust
sandbox
    .secret("ANTHROPIC_API_KEY", &vault_fetch("anthropic-key"))
    .allow_host("api.anthropic.com");
```

Secrets are injected via placeholder substitution into the guest process environment. The literal value never touches the command string, disk, or any connection to a non-declared host.

**Alternative**: Bind-mount a credentials file (`~/.aws/credentials`). Simpler to implement, but the agent can read the file and exfiltrate it — defeats the isolation goal.

---

### 2. Network Policy Default

**Question**: What should `network_policy` default to when not specified in `SandboxConfig`?

**Recommendation**: `public_only()`.

Agents need outbound access to LLM providers (Anthropic, Bedrock) and package registries (PyPI, npm, crates.io). They must not reach internal infrastructure: databases, other agents' HTTP APIs, cloud metadata at 169.254.169.254, or internal VPC services. `public_only()` blocks RFC-1918 ranges by default while allowing public internet.

**Alternative**: `none()` — maximally restrictive, requires explicit `allow_host()` per dependency. Correct for high-security contexts but breaks most agent use cases out of the box.

---

### 3. Image Management

**Question**: Who maintains sandbox base images and where are they stored?

**Recommendation**: Maintain a set of base images in GHCR (`ghcr.io/our-org/nous-base:*`), built by a dedicated CI job. Provide at minimum:
- `nous-base:minimal` — git, ssh, curl
- `nous-base:python` — Python 3.12 + pip
- `nous-base:node` — Node 22 + npm
- `nous-base:claude-code` — Claude Code CLI

Agent forms specify a full image reference. Custom images extend from a base and are the agent author's responsibility. Image building and registry management are out of scope for NOUS-050 and should be tracked in a separate ticket.

---

### 4. Bind Mount Scope

**Question**: Should agents receive access to the full repository root or only their `working_dir`?

**Recommendation**: Mount only `agent.working_dir` as read-write, plus a read-only mount of the repo root at `/repo`. This follows least-privilege: the agent can write only to its designated workspace but can read any file in the repo for context.

Do not mount the entire host filesystem. The risk of an agent reading `/etc/shadow`, `/root/.ssh`, or daemon configuration files outweighs the convenience of broader access.

---

### 5. Process Type Coexistence

**Question**: Should `process_type` be fixed at agent registration or selectable per-spawn?

**Recommendation**: Per-spawn. `process_type` is a field on `agent_processes` (per-spawn row), not on `agents` (per-registration row). An agent definition in a form can specify `default_process_type: sandbox` as metadata, but each `nous agent spawn` invocation can override it with `--type shell` for debugging without sandboxing.

The current schema already supports this — `agents.process_type` (migration 025) is the default hint, while `agent_processes.process_type` (migration 023) is the authoritative value for a given run. No schema change is needed for per-spawn flexibility.
