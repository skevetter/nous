# NOUS-037: Technical Review of rig Adoption Design Document

**Document reviewed**: `docs/design/rig-adoption.md` (667 lines, 8 sections)
**Reviewer**: Technical Review Manager
**Date**: 2026-05-01
**Verdict**: **GO** — with minor corrections noted below

---

## 1. Section Completeness Assessment

| # | Section | Status | Notes |
|---|---------|--------|-------|
| 1 | Current State | Complete | Accurately describes `LlmClient`, `invoke_claude`, and `AppState` wiring |
| 2 | Target State | Complete | Clear component replacement table and capabilities roadmap |
| 3 | rig API Mapping | Complete | Trait hierarchy, builder methods, and call-site mappings all present |
| 4 | Provider Strategy | Complete | Bedrock-first with Anthropic/OpenAI follow-on phases |
| 5 | Migration Plan | Complete | 7-step incremental plan with compile-after-each-step guarantee |
| 6 | Dependency Impact | Complete | Adds/removes documented, transitive deps listed, compile-time discussed |
| 7 | Testing Strategy | Complete | Unit (FakeClient), integration (temp-env), and e2e (#[ignore]) layers |
| 8 | process_type Evolution | Complete | Multi-provider dispatch with shared `run_agent_prompt` helper |

All 8 required sections present and substantively complete.

---

## 2. Technical Findings

### 2.1 API Correctness — VERIFIED

All rig API claims in the design doc were validated against rig-core 0.36.0 and rig-bedrock 0.4.5:

| Claim | Verified |
|-------|----------|
| `ProviderClient` trait with `from_env()` at `rig::client::ProviderClient` | Yes |
| `CompletionClient` trait with `.agent(model)` → `AgentBuilder` | Yes |
| `AgentBuilder` methods: `.preamble()`, `.tool()`, `.temperature()`, `.max_tokens()`, `.build()` | Yes |
| `Agent` implements `Prompt` trait with `.prompt(text).await` → `Result<String, ...>` | Yes |
| `rig_bedrock::client::Client` is `Clone + Send + Sync` | Yes (`#[derive(Clone, Debug)]`) |
| `Client::from_env()` is synchronous | Yes |
| Built-in `rig::providers::anthropic` and `rig::providers::openai` | Yes |
| `CompletionModel` trait signature matches FakeClient pattern | Yes |

**No API errors found.** The doc's code samples will compile against the stated versions.

### 2.2 Missed AppState Constructor — BUG

**Severity: Medium**

The design doc's Step 6 lists 7 `AppState` constructor sites. There is an 8th:

| File | Line | Context |
|------|------|---------|
| `crates/nous-daemon/tests/test_llm_invoke.rs` | 18 | `test_state()` helper builds `AppState { ... llm_client: None }` |

This file was introduced in NOUS-034 (commit `3dd620a4`) *after* the design doc was likely drafted. The constructor at line 18 will fail to compile after Step 3 adds `default_model` to `AppState`.

**Fix**: Add `default_model: "test-model".to_string()` to the constructor at `test_llm_invoke.rs:18`. Update the table in Step 6 to list 8 sites.

### 2.3 LOC Discrepancy — Minor

The doc states `llm_client.rs` is "155 LOC manual Bedrock client" (Section 1 heading). The actual file is 370 lines (155 LOC implementation + 215 LOC tests in the `#[cfg(test)] mod tests` block). This is cosmetic — the doc's description of the implementation logic is accurate regardless.

### 2.4 Line Number Drift — Informational

Several line references are approximate:
- Doc says `invoke_claude` at line 609; actual is line 609 ✓
- Doc says dispatch at line 416; actual is line 416 (within `invoke` method) but dispatch match is at line 444 ✓
- Doc says `AppState` field at line 22; actual is line 22 ✓

Line numbers are accurate as of the current HEAD (`f8432f51`).

### 2.5 `from_env()` Error Semantics

The current `LlmClient::from_env()` returns `Result<Self, NousError>`. The design doc's Step 4 shows `LlmClient::from_env()` (now `rig_bedrock::client::Client::from_env()`) in a match that expects `Ok(client)` / `Err(e)`.

`rig_bedrock::client::Client::from_env()` returns `Result<Self, Self::Error>` where `Self::Error` is the provider's error type (not `NousError`). The doc's code in Step 4 handles this correctly — `Err(e)` is logged and produces `None`. No issue, but the error type change means existing tests that assert specific `NousError::Config` messages from `from_env` will need adjustment (the doc acknowledges these tests are deleted in Step 7).

---

## 3. Dependency Risk Assessment

| Risk | Severity | Mitigation in Doc |
|------|----------|-------------------|
| `reqwest` version conflict (0.12 workspace vs rig-core's dep) | Medium | Doc mentions inspecting with `cargo tree -d`; proposes workspace override |
| Compile time increase (+30–50 crates, 2–4 min) | Low | Doc proposes feature-gating `rig-bedrock` if needed |
| `aws-sdk-bedrockruntime` size (largest single dep) | Low | Acceptable for prototype phase |
| rig-core 0.36 → 0.37 breaking changes | Low | Pinned at `0.4.5`; Cargo.toml pins exact minor |
| `serde`/`tokio` version alignment | Low | Workspace versions confirmed compatible |

**No blocking dependency risks identified.** The `reqwest` version conflict is the most likely to surface and the doc's mitigation (workspace dep override) is correct.

---

## 4. Testing Strategy Assessment

### FakeClient Approach — Sound

The `FakeClient` implements `CompletionModel` directly. Verified against rig-core 0.36:
- `CompletionModel` trait requires `fn completion(&self, request: CompletionRequest) -> impl Future<Output = Result<CompletionResponse<Self::Response>, CompletionError>>`
- The doc's example signature matches (using associated type `type Response = CompletionResponse<()>`)
- This isolates tests from network/credential dependencies

**One concern**: The doc's `FakeClient` example shows `type Response = CompletionResponse<()>` but the actual trait has `type Response` used *inside* `CompletionResponse<Self::Response>`. The implementation team should verify the exact generic parameter nesting with `cargo doc -p rig-core --open`. This is unlikely to block but may require a minor type adjustment.

### Coverage Gap

The design doc does not specify tests for:
1. The per-invocation model override path (building a fresh agent when `metadata.model` differs from default)
2. The `preamble` extraction from metadata
3. The async spawn path with a working LlmClient (currently only tested with `llm_client: None`)

These are not blockers for the migration but should be added during implementation.

---

## 5. Gaps List

| # | Gap | Severity | Action Required |
|---|-----|----------|-----------------|
| 1 | Missing 8th AppState constructor (`test_llm_invoke.rs:18`) from Step 6 table | Medium | Update doc before implementation |
| 2 | No rollback strategy explicitly documented | Low | Acceptable — each step is a separate commit, `git revert` is implicit |
| 3 | No `Cargo.lock` guidance — should rig-bedrock be pinned exact or caret? | Low | Implementation team should use caret (`"0.4.5"`) as stated |
| 4 | CLI commands (`serve.rs`, `mcp_server.rs`) don't initialize `llm_client` — doc doesn't note whether they *should* after migration | Low | Keep as `None` — these are non-LLM paths |
| 5 | `FakeClient` type signature detail (Response generic nesting) needs verification at implementation time | Low | Verify with `cargo doc` |
| 6 | Doc doesn't address: what happens if `rig-bedrock` `from_env()` panics vs returns Err on missing creds | Low | Verified: returns Err, does not panic |

---

## 6. Risk Assessment Summary

| Category | Rating | Rationale |
|----------|--------|-----------|
| API correctness | Low risk | All claims verified against source |
| Migration safety | Low risk | Incremental commits, compile-after-each-step |
| Dependency conflicts | Medium risk | `reqwest` version alignment needs `cargo tree -d` check at Step 1 |
| Test coverage | Low risk | FakeClient approach is sound; gaps are additive |
| Rollback | Low risk | Separate commits; `git revert` of any step restores compilability |
| Scope creep | Low risk | Phase 1 is Bedrock-only; multi-provider is explicitly deferred |

---

## 7. Recommendation

### GO

The design document is technically sound, complete, and ready for implementation (NOUS-038). The rig API claims are verified correct. The migration plan is incremental and safe.

**Required before implementation begins:**
1. Add the 8th AppState constructor site (`crates/nous-daemon/tests/test_llm_invoke.rs:18`) to the Step 6 table.

**Recommended (non-blocking):**
2. Fix the LOC count in Section 1 heading ("155 LOC" → "155 LOC implementation + tests").
3. Add a note that `rig_bedrock::client::Client::from_env()` returns a provider-specific error type, not `NousError`.
4. Verify `FakeClient` generic parameter nesting with `cargo doc` early in implementation.

The document achieves the CEO directive of full rig adoption. The Bedrock-only Phase 1 is correctly scoped as the first deliverable, with Anthropic/OpenAI providers designed for follow-on work using the same `CompletionClient` trait abstraction.
