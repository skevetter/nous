# Agent Definition UX

**Status:** Draft  
**Date:** 2026-05-01

---

## 1. Prior Art Survey

Six systems were examined: Claude Code skills/CLAUDE.md, Dagger modules, Pulumi component packages, Helm charts, Homebrew tap formulas, npm packages, and Cargo crates. Each shows a distinct point on the tradeoff curve between flexibility, type safety, and distribution ergonomics.

### 1.1 Claude Code Skills and CLAUDE.md

**Unit definition.** A skill is a Markdown file placed under `.claude/skills/` (project) or `~/.claude/skills/` (user). CLAUDE.md files are also Markdown, scoped to a directory level. Both are prose-first: no schema enforces structure, so the author shapes the file with free text, headers, and code blocks.

**Versioning model.** No built-in version field. Versioning relies entirely on the git history of the repository containing the skill files. The `/init` flow can hash skill contents (`skill_hash`) and record them against an agent version, as seen in `crates/nous-cli/src/commands/agent.rs:RecordVersion`.

**Discovery mechanism.** Claude Code walks the directory tree from the working directory upward, loading every CLAUDE.md it finds. Skills under `.claude/skills/` are loaded on demand when Claude judges them relevant to the current prompt, or when explicitly invoked with a slash command. There is no registry; there is no index file. Discovery is purely filesystem-based.

**Tradeoffs.** Markdown is near-zero friction to author but provides no machine-parseable contract. Skills cannot express typed inputs or validated configuration. The implicit discovery model works well at small scale but breaks down when dozens of skills accumulate — there is no way to list available skills without scanning the filesystem.

### 1.2 Dagger Modules

**Unit definition.** A Dagger module is a directory containing a `dagger.json` manifest and language-native source code (Go, Python, TypeScript, PHP, or Java). The manifest carries the module name and a `sdk` field indicating the language runtime. The public API is whatever types and functions the module's top-level package exports; Dagger generates a GraphQL schema from those exports automatically.

**Versioning model.** Modules are versioned via Git tags on their host repository. A dependency reference like `github.com/user/repo@v0.2.0` pins to a specific tag. Language-native lock files (`go.sum`, `uv.lock`, `package-lock.json`) pin transitive dependencies.

**Discovery mechanism.** No central registry. Modules are referenced by Git URL. The Dagger CLI resolves them by cloning or fetching the referenced commit. `dagger install github.com/user/repo/path@v1.0.0` adds an entry under `dependencies` in the consuming module's `dagger.json`.

**Tradeoffs.** Language-native code enables full type safety and IDE tooling within each supported language. The Git-URL model makes publishing trivially easy — no registry account needed. The tradeoff is that cross-language module reuse requires the consumer to pick a supported SDK language; Dagger bridges this via its generated GraphQL layer, but that layer is invisible to module authors.

### 1.3 Pulumi Component Packages

**Unit definition.** A Pulumi component package is described by a JSON schema file. The schema declares component resources, their `inputProperties` (typed parameters) and output `properties`, plus custom types in a shared `types` section. Language-specific SDKs are generated from this single schema, meaning one definition produces idiomatic Rust, Python, Go, TypeScript, and C# packages simultaneously.

**Versioning model.** The schema carries a top-level `version` field that must be valid SemVer. SDK packages published to language registries (npm, PyPI, NuGet, Go modules) carry this same version. Providers are distributed as binaries with a `pluginDownloadURL` field pointing to a release artifact.

**Discovery mechanism.** Consumers reference Pulumi packages by their registry identifier (`pulumi:aws:5.0.0`). The Pulumi CLI fetches the provider binary and SDK from the declared download URL. There is no Git-URL mechanism; distribution goes through language registries and binary hosting.

**Tradeoffs.** The schema-first, polyglot approach is powerful for infrastructure components but carries significant authoring overhead. Writing a correct JSON schema by hand is error-prone, and the code-generation pipeline requires a build step. This model suits large, stable components with many consumers but is overkill for ephemeral or experimental agents.

### 1.4 Helm Charts

**Unit definition.** A Helm chart is a directory with a fixed layout: `Chart.yaml` (manifest), `values.yaml` (defaults), `values.schema.json` (optional JSON Schema for validation), and a `templates/` directory containing Go-template Kubernetes manifests. The `Chart.yaml` requires exactly three fields: `apiVersion` (always `v2`), `name`, and `version`.

**Versioning model.** SemVer. The chart version appears in `Chart.yaml` and in the packaged filename (`nginx-1.2.3.tgz`). The version in the filename must match the manifest or installation fails. Charts track an `appVersion` separately from the chart's own version, which lets the chart evolve independently of the software it deploys.

**Discovery mechanism.** Helm chart repositories are HTTP servers hosting an `index.yaml` that lists all available chart versions with checksums. OCI registries (standard container registries) also host charts directly. `helm repo add <name> <url>` registers a repository; `helm search repo <keyword>` queries across all registered repositories.

**Tradeoffs.** Helm's two-version model (`version` vs. `appVersion`) and strict directory layout enforce predictable structure across thousands of charts from different authors. The `values.schema.json` provides typed parameter validation. The tradeoff is the Go template language: it is powerful but produces error messages that are hard to interpret and discourages non-experts from modifying charts.

### 1.5 Homebrew Tap Formulas

**Unit definition.** A Homebrew formula is a Ruby DSL file. The formula declares a `url` (download location), `sha256` checksum, `version`, build steps, and dependencies via `depends_on`. A tap is a Git repository named `homebrew-<name>` containing formula files under a `Formula/` directory.

**Versioning model.** Each formula has an explicit `version` field. Homebrew does not version the tap itself — the tap is always the current HEAD of its default branch. Formula versions are immutable once merged into the official tap (homebrew/core). Third-party taps can change versions freely.

**Discovery mechanism.** `brew tap <user>/<repo>` clones `github.com/<user>/homebrew-<repo>` into `$(brew --repository)/Library/Taps/<user>/homebrew-<repo>`. After tapping, `brew install <user>/<repo>/<formula>` installs the formula. `brew update` pulls the latest HEAD of all tapped repositories. Conflict resolution prefers homebrew/core; third-party tap formulas need unique names or the fully qualified `<user>/<repo>/<formula>` form.

**Tradeoffs.** The tap model makes third-party distribution trivially easy: any public GitHub repository named `homebrew-*` becomes a distributable tap. The Ruby DSL is readable but not statically checkable without running a Ruby linter. The always-latest-HEAD model means tap consumers get updates automatically on `brew update`, which can break reproducibility unless formulas explicitly pin versions in their `url` field.

> **Terminology note:** Homebrew calls its distribution repositories "taps." Nous adopts the same Git-repository-as-distribution-unit pattern but uses the term "form" to avoid overloading the Homebrew-specific name. All references to taps in this section (§1.5) use Homebrew's terminology; the rest of this document uses "form" for the nous equivalent.

### 1.6 npm Packages

**Unit definition.** An npm package is a directory containing `package.json`. The manifest declares a `name` (scoped or unscoped), `version` (SemVer), entry points (`main` or the newer `exports` map), `dependencies`, `devDependencies`, `peerDependencies`, and lifecycle `scripts`. The `exports` map enables conditional resolution — different entry points for CommonJS vs. ESM consumers.

**Versioning model.** SemVer with `^` (compatible minor) and `~` (compatible patch) range specifiers in dependency declarations. `package-lock.json` pins the exact resolved tree. Published packages are immutable on the registry — once `1.0.0` is published to npmjs.com, its content cannot change (only deprecated or unpublished).

**Discovery mechanism.** `npm install <name>` resolves from the configured registry (npmjs.com by default, or a private registry). Scoped packages (`@org/name`) provide namespace isolation. `npm search <keyword>` queries the registry full-text index.

**Tradeoffs.** The npm registry provides centralized discovery, immutable releases, and scoped namespacing. The `package.json` format is JSON (not TOML), which is familiar but more verbose than TOML for configuration use cases. The `exports` map and conditional resolution are powerful but have a steep learning curve. Peer dependencies are a recurring source of installation conflicts.

### 1.7 Cargo Crates

**Unit definition.** A Rust crate is a directory containing `Cargo.toml`. The `[package]` section carries `name`, `version` (SemVer), `edition` (Rust edition year), `description`, and `license`. Dependencies go in `[dependencies]`, `[dev-dependencies]`, and `[build-dependencies]`. Features (`[features]`) enable conditional compilation. Workspace manifests (`[workspace]`) aggregate multiple crates without their own `[package]` section — exactly the pattern nous uses at the repository root.

**Versioning model.** SemVer. `Cargo.lock` pins the exact resolved crate versions for binary crates (optional for libraries). The resolver selects the highest compatible version for each dependency. Pre-release versions (`1.0.0-alpha`) are not automatically selected by the resolver; consumers must opt in explicitly.

**Discovery mechanism.** `cargo add <name>` fetches metadata from crates.io (or a configured private registry). The crates.io index is a Git repository containing a compact metadata file per crate. `cargo search <keyword>` queries crates.io's full-text index. Private registries expose the same sparse index protocol.

**Tradeoffs.** TOML is the most ergonomic manifest format of the group — human-readable, supports comments, has a clean table syntax. The workspace model (`[workspace]` + member crates) maps cleanly to nous's existing structure. Crates.io is append-only and immutable per version. The tradeoff is that crates.io is a code registry, not a content registry — it is designed for library code, not for agent behavior definitions that might be 100 lines of TOML or Markdown.

## 2. Format Evaluation

Four candidate formats were evaluated against nous's specific constraints: the platform is Rust (v0.10.0), config already lives in TOML (`~/.config/nous/config.toml`), and the target audience is developers comfortable with CLIs but not necessarily with any single scripting language.

| Format | Type Safety | Tooling | Learning Curve | Ecosystem Fit | Tradeoffs |
|--------|-------------|---------|----------------|---------------|-----------|
| **TOML** | Schema-validated at parse time via `serde` + `toml` crate; no runtime type coercion | VS Code TOML extension, `taplo` formatter/LSP; no IDE-level autocomplete without a schema server | Near-zero for Rust/Go developers; familiar from `Cargo.toml` and existing `config.toml` | Excellent — nous already uses `toml = "0.8"` in `Cargo.toml`; `serde` deserialization is native | Static only; cannot express imperative logic (conditional behavior, computed values) |
| **YAML** | Schema-validated via external validators (JSON Schema, `schemars`); implicit type coercion is a common footgun | Wide tooling support (yaml-language-server, Prettier); native in CI platforms | Low for ops/DevOps profiles; moderate for developers unfamiliar with YAML gotchas (Norway problem, string/bool ambiguity) | Moderate — Helm, Kubernetes, GitHub Actions use YAML; adds `serde_yaml` dependency to Rust | Coercion bugs (`on: true`, bare `NO` → `false`) make machine-generated YAML fragile; significant whitespace rules are invisible errors |
| **TypeScript** | Full static types via TypeScript compiler; interface definitions are first-class | Excellent IDE support (VS Code, JetBrains); `tsc`, `eslint`, `prettier` are mature | Low for JS/TS developers; high for Rust-only teams | Poor — would require a Node.js runtime or Deno alongside the Rust binary; adds a non-trivial dependency to a Rust-first project | Requires a JS runtime for evaluation; definition files that import SDKs can trigger arbitrary code execution; poor fit for a pure-Rust platform |
| **Python** | Type hints with `mypy`; runtime type checking via `pydantic` | Good tooling (`pyright`, `ruff`, `black`); `pydantic` v2 generates JSON Schema automatically | Low for Python developers; high for teams without a Python background | Poor — same runtime dependency problem as TypeScript; Dagger chose Python for its module SDK but provides a full Python runtime | Pydantic models produce excellent JSON Schema, enabling UI generation; arbitrary code execution risk in definition files is a security concern for an agent platform |

### Verdict

TOML is the correct format for nous agent definitions at this stage. The decision rests on three concrete points:

1. **Zero new dependencies.** `toml = "0.8"` and `serde` are already in the workspace. Parsing, validating, and serializing agent definitions adds no new crates.
2. **Consistent user experience.** Users already configure nous via `~/.config/nous/config.toml`. An agent definition file that looks identical — same comments, same table syntax — removes the cognitive cost of switching between formats.
3. **Rust `serde` deserialization provides structural type safety** without a separate schema compiler step. A missing required field (`name`, `version`) produces a clear error at `nous agent add` time, before any process is spawned.

The tradeoff accepted is that TOML cannot express conditional logic. Agents that require runtime branching (e.g., "use model X on weekdays, model Y on weekends") need that logic in a shell script or Rust plugin — it cannot live in the definition file itself. This is acceptable at the prototype phase and consistent with how Helm handles the same tradeoff (logic lives in templates, not in `Chart.yaml`).

## 3. Proposed File Structure

### 3.1 Directory Layout

Agent definitions and skills live under the XDG config directory (`~/.config/nous/`) alongside the existing `config.toml`. The layout mirrors the Claude Code `.claude/skills/` convention but uses the XDG path nous already owns.

```
~/.config/nous/
├── config.toml                    # Existing daemon/DB config
├── agents/
│   ├── reviewer.toml              # Agent definition
│   ├── planner.toml
│   └── researcher.toml
├── skills/
│   ├── code-review.md             # Skill files (Markdown)
│   ├── git-workflow.md
│   └── summarize.md
└── forms/
    ├── paseo-org/
    │   ├── form.toml              # Form metadata (name, url, pinned_ref)
    │   ├── agents/
    │   │   ├── monitor.toml
    │   │   └── deployer.toml
    │   └── skills/
    │       └── incident-response.md
    └── acme-corp/
        ├── form.toml
        └── agents/
            └── data-pipeline.toml
```

The `forms/` subdirectory is populated by `nous form add`; its contents are managed by nous and should not be edited by hand. User-authored agents and skills live directly under `~/.config/nous/agents/` and `~/.config/nous/skills/`.

### 3.2 Agent Definition File Format

An agent definition is a TOML file. Fields map directly to the `agents::RegisterAgentRequest` structure in `crates/nous-core/src/agents/mod.rs`.

```toml
# ~/.config/nous/agents/reviewer.toml

[agent]
name       = "reviewer"
type       = "engineer"          # engineer | manager | director | senior-manager
version    = "1.2.0"
namespace  = "eng"
description = "Performs code review on feature branches"

[process]
type         = "claude"          # claude | shell | http
spawn_command = "claude --model claude-sonnet-4-6"
working_dir  = "~"
auto_restart = false

[skills]
refs = [
  "code-review",                 # local skill by name (resolved from ~/.config/nous/skills/)
  "git-workflow",
  "paseo-org/incident-response", # form-qualified skill reference
]

[tools]
refs = [
  "web-search",
  "file-read",
  "code-execution",
]

[metadata]
model   = "global.anthropic.claude-sonnet-4-6-v1"
timeout = 3600
tags    = ["review", "quality"]
```

Required fields: `agent.name`, `agent.type`, `agent.version`. All other fields have defaults matching those in `agents::RegisterAgentRequest`.

The `[tools]` section declares which tools the agent has access to at runtime. Each entry in the `refs` array is a tool name string. Tool names are resolved against the runtime's built-in tool set first, then against any tool registry configured in `~/.config/nous/config.toml`. If a referenced tool is not found at spawn time, `nous agent spawn` produces an error listing the unresolved tool names. Omitting the `[tools]` section entirely grants the agent no tool access by default.

A skill file is plain Markdown, exactly as Claude Code skills work today. The `[skills].refs` array in the agent definition resolves each entry first against `~/.config/nous/skills/<name>.md`, then against installed form skill directories.

### 3.3 Naming Conventions

| Item | Convention | Example |
|------|-----------|---------|
| Agent definition file | lowercase kebab-case, `.toml` extension | `code-reviewer.toml` |
| Skill file | lowercase kebab-case, `.md` extension | `git-workflow.md` |
| Form directory | `<owner>` form matching `nous form add <owner>/<repo>` | `paseo-org/` |
| Form-qualified skill ref | `<form-owner>/<skill-name>` | `paseo-org/incident-response` |
| Agent name field | lowercase, hyphens allowed, no spaces | `code-reviewer` |

Agent names must be unique within a namespace. Two agents in different namespaces may share a name; `nous agent list --namespace <ns>` scopes the listing accordingly.

### 3.4 CLI Commands

All agent definition operations go through the existing `nous agent` subcommand tree. Three new subcommands are added under `nous agent`:

#### `nous agent add <file>`

Reads a local TOML definition file, registers the agent in the nous database, and uploads referenced skill content.

```bash
# Register from a local file
nous agent add ~/.config/nous/agents/reviewer.toml

# Register from a form-installed definition
nous agent add paseo-org/monitor
# Resolves to ~/.config/nous/forms/paseo-org/agents/monitor.toml

# Output (matches existing agent JSON output format)
{
  "id": "01931f7a-...",
  "name": "reviewer",
  "namespace": "eng",
  "agent_type": "engineer",
  "status": "idle",
  ...
}
```

#### `nous agent list`

Lists registered agents. Existing command; no changes needed. The `--type` and `--namespace` filters are already implemented.

```bash
nous agent list --namespace eng --type engineer
```

#### `nous agent remove <name-or-id>`

Deregisters an agent. This is a thin alias over the existing `nous agent deregister` command with name resolution via `agents::lookup_agent`.

```bash
nous agent remove reviewer
nous agent remove 01931f7a-...   # also accepts UUID
```

#### `nous agent sync`

Reconciles the filesystem definition files with the database. For each `.toml` file in `~/.config/nous/agents/`, it creates or updates the corresponding agent record. Agents in the database that have no corresponding file are left untouched (not deleted), so manually-registered agents are preserved.

```bash
nous agent sync
# synced: reviewer (updated), planner (created)
# skipped: manually-registered-agent (no definition file)
```

#### `nous skill list`

Lists skills available to the current user (local skills + all installed form skills).

```bash
nous skill list
# LOCAL
#   code-review     (~/.config/nous/skills/code-review.md)
#   git-workflow    (~/.config/nous/skills/git-workflow.md)
# paseo-org
#   incident-response
#   deploy-runbook
```

### 3.5 Agent Skills Specification Alignment

The [Agent Skills Specification](https://agentskills.io/specification) is an emerging standard for agent skill interoperability across platforms. Nous agent definitions align with this specification at the structural level: the TOML `[agent]` section maps to the spec's skill metadata (name, version, description), and the `[skills].refs` array maps to the spec's capability declarations. This alignment means a nous agent definition can be mechanically translated to and from the Agent Skills Spec format, enabling interoperability with other platforms that adopt the standard.

Full compliance with the Agent Skills Specification is not a goal for Phase 1. The alignment documented here is a forward reference — as the spec matures, nous can adopt stricter conformance without restructuring the definition format. Implementors should consult the spec when extending the `[skills]` or `[tools]` sections to avoid divergence that would make future alignment more costly.

### 3.6 Hooks

Agent definitions support lifecycle hooks that execute at defined points during an agent's interaction with its underlying model. Hooks are based on rig's [`PromptHook`](https://docs.rs/rig-core/latest/rig/agent/trait.PromptHook.html) trait, which provides structured extension points in the prompt-completion lifecycle. Each hook declaration specifies a name, a lifecycle event, a handler type, and the command to execute.

```toml
[[hooks]]
name       = "audit-log"
event      = "before_completion"
handler    = "shell"
command    = "nous hook run audit-log --event before_completion"

[[hooks]]
name       = "token-counter"
event      = "after_completion"
handler    = "shell"
command    = "nous hook run token-counter --event after_completion"
```

The `event` field maps directly to rig's `PromptHook` trait lifecycle: `before_completion` fires before the LLM call is made (corresponding to the trait's pre-request hook point), and `after_completion` fires after the LLM returns a response (corresponding to the post-response hook point). The `handler` field specifies the execution mode — `shell` runs the command as a subprocess. Future handler types (e.g., `wasm`, `http`) can be added without changing the hook declaration syntax.

## 4. Form Model Design

The form model takes its structure from Homebrew taps: a Git repository serves as the distribution unit. Any public (or private, with credentials) Git repository that follows the form layout becomes installable with a single command. No central registry account is required.

### 4.1 Form Repository Layout

A form repository must contain a `form.toml` manifest at the root. Agents go under `agents/`, skills under `skills/`:

```
github.com/paseo-org/nous-form/         # Git repository
├── form.toml                           # Form manifest (required)
├── agents/
│   ├── monitor.toml
│   ├── deployer.toml
│   └── data-pipeline.toml
└── skills/
    ├── incident-response.md
    └── deploy-runbook.md
```

The `form.toml` manifest:

```toml
[form]
name        = "paseo-org"
description = "Paseo platform agents and skills"
url         = "https://github.com/paseo-org/nous-form"
maintainers = ["team@paseo.dev"]
```

The `name` field in `form.toml` determines the directory name under `~/.config/nous/forms/` and the namespace prefix for form-qualified references (`paseo-org/monitor`).

### 4.2 CLI Commands

#### `nous form add <owner>/<repo>`

Clones the form repository and writes a `form.toml` entry pinning the current HEAD SHA.

```bash
nous form add paseo-org/nous-form
# Cloning https://github.com/paseo-org/nous-form ...
# Installed form 'paseo-org' at ~/.config/nous/forms/paseo-org/
# 12 agents, 7 skills available.
```

#### `nous form remove <name>`

Removes the form directory and its `form.toml` entry. Does not deregister agents that were instantiated from the form.

```bash
nous form remove paseo-org
```

#### `nous form update [<name>]`

Pulls the latest HEAD (or the pinned ref if `--pin` was used) for one or all forms.

```bash
nous form update               # updates all forms
nous form update paseo-org     # updates one form
```

#### `nous form list`

Lists installed forms with their pinned ref and available agent/skill counts.

```bash
nous form list
# NAME           URL                                    REF        AGENTS  SKILLS
# paseo-org      github.com/paseo-org/nous-form         a3f1c2d    12      7
# acme-corp      github.com/acme/nous-agents            HEAD       3       2
```

### 4.3 Step-by-Step: Installing and Using a Form Agent

```
1. Add the form:
   $ nous form add paseo-org/nous-form
   → Clones repo to ~/.config/nous/forms/paseo-org/
   → Reads form.toml; records pinned SHA in ~/.config/nous/forms/paseo-org/form.toml

2. Inspect available agents:
   $ nous skill list
   → Shows paseo-org/* skills alongside local skills
   $ nous agent list    # (agents from forms are not yet registered)

3. Register an agent from the form:
   $ nous agent add paseo-org/monitor
   → Resolves to ~/.config/nous/forms/paseo-org/agents/monitor.toml
   → Calls agents::register_agent with definition fields
   → Writes skill content for referenced skills into the DB

4. Spawn the agent:
   $ nous agent spawn <id> --type claude
   → Uses spawn_command from the definition file

5. Update the form to get new agent versions:
   $ nous form update paseo-org
   → git pull in ~/.config/nous/forms/paseo-org/

6. Sync updated definitions into the DB:
   $ nous agent sync
   → Re-reads all definition files; updates changed fields
```

### 4.4 Versioning Strategy

Form repositories are versioned with Git tags following SemVer (`v1.0.0`, `v1.2.3`). By default, `nous form add` follows HEAD (equivalent to Homebrew's default behavior). Two optional modes allow pinning:

```bash
# Pin to a tag
nous form add paseo-org/nous-form@v1.2.0

# Pin to a specific commit SHA
nous form add paseo-org/nous-form@a3f1c2d
```

The pinned ref is recorded in `~/.config/nous/forms/paseo-org/form.toml`:

```toml
[form]
name       = "paseo-org"
url        = "https://github.com/paseo-org/nous-form"
pinned_ref = "v1.2.0"           # empty string means HEAD
fetched_at = "2026-05-01T10:00:00Z"
```

`nous form update` respects the pinned ref: if `pinned_ref` is set, update is a no-op unless `--upgrade` is passed to advance to a new tag.

### 4.5 Namespace Isolation

Each form occupies an isolated namespace equal to its `name` field in `form.toml`. Two forms that both define an agent named `monitor` do not conflict: one is `paseo-org/monitor`, the other is `acme-corp/monitor`. Unqualified names (`nous agent add monitor`) resolve against local `~/.config/nous/agents/` first, then in order of form installation. Conflicts produce a disambiguation error:

```
error: agent name 'monitor' is ambiguous — found in: paseo-org, acme-corp
       Use a qualified reference: nous agent add paseo-org/monitor
```

### 4.6 Update Mechanism

```
nous form update paseo-org
  │
  ├── git fetch origin in ~/.config/nous/forms/paseo-org/
  ├── Computes diff of changed agent/skill files
  ├── Reports: "3 agents updated, 1 agent added, 0 removed"
  └── Does NOT automatically re-register agents.
      Run: nous agent sync  (to propagate changes to the DB)
```

Separating `form update` (fetches files) from `agent sync` (updates the DB) lets users review what changed before applying it — the same pattern as `helm repo update` followed by `helm upgrade`.

## 5. Lockfile Design

The lockfile (`~/.config/nous/nous.lock`) pins every agent and skill to a specific content hash and source ref. Its purpose mirrors `Cargo.lock`: given the same lockfile, two different machines produce an identical agent configuration even if form repositories have changed upstream.

### 5.1 Lockfile Format

The lockfile is TOML (consistent with the rest of nous configuration):

```toml
# ~/.config/nous/nous.lock
# Generated by nous agent sync. Do not edit manually.
# Regenerate: nous agent sync --refresh-lock

lock_version = 1
generated_at = "2026-05-01T10:00:00Z"

[[agent]]
name      = "reviewer"
source    = "local"
file      = "~/.config/nous/agents/reviewer.toml"
sha256    = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
version   = "1.2.0"

[[agent]]
name      = "monitor"
source    = "form:paseo-org"
file      = "~/.config/nous/forms/paseo-org/agents/monitor.toml"
sha256    = "a665a45920422f9d417e4867efdc4fb8a04a1f3fff1fa07e998e86f7f7a27ae3"
version   = "2.0.1"
form_ref   = "v1.2.0"

[[skill]]
name      = "code-review"
source    = "local"
file      = "~/.config/nous/skills/code-review.md"
sha256    = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"

[[skill]]
name      = "paseo-org/incident-response"
source    = "form:paseo-org"
file      = "~/.config/nous/forms/paseo-org/skills/incident-response.md"
sha256    = "82e35a63ceba37e9646434c5dd412ea577596f72d2c7d0b7d3f11e5d8d9a3d65"
form_ref   = "v1.2.0"
```

### 5.2 Resolution Algorithm

When `nous agent sync` runs, it resolves in this order:

```
1. Read all .toml files in ~/.config/nous/agents/ (local agents).
2. For each form in ~/.config/nous/forms/:
     a. Read form.toml to get pinned_ref.
     b. Verify the local clone is at pinned_ref (git rev-parse HEAD).
     c. Read all .toml files in <form>/agents/.
3. For each agent definition:
     a. Compute SHA-256 of the file contents.
     b. Compare against nous.lock entry.
     c. If SHA differs → mark as "changed"; update DB and lock entry.
     d. If no lock entry → mark as "new"; create DB record and lock entry.
     e. If in lock but file missing → mark as "removed" (warning only; no DB delete).
4. For each skill ref in each agent definition:
     a. Resolve skill file path (local or form-qualified).
     b. Compute SHA-256.
     c. Record in nous.lock.
5. Write updated nous.lock atomically (write to nous.lock.tmp, rename).
```

Step 5 uses a rename to prevent a crash during write from producing a corrupted lockfile.

### 5.3 Offline Support

nous resolves all definitions from the local filesystem. No network access is required at `nous agent sync` time. The lockfile enables fully reproducible, offline operation:

| Operation | Network required? |
|-----------|------------------|
| `nous agent sync` | No — reads local files only |
| `nous agent add <local-file>` | No |
| `nous agent add <form>/<name>` | No (form must already be fetched) |
| `nous form add <owner>/<repo>` | Yes — initial clone |
| `nous form update` | Yes — git fetch |
| `nous agent spawn` | No — uses local DB records |

Machines in air-gapped environments can pre-populate `~/.config/nous/forms/` by copying the directories directly (rsync, archive, or git bundle) and running `nous agent sync` locally. The lockfile verifies integrity via SHA-256 without contacting any external service.

### 5.4 When to Regenerate

The lockfile regenerates automatically on:

- `nous agent sync`
- `nous agent add <file>` (adds or updates a single entry)
- `nous form update` (updates form_ref and file hashes for that form's entries)

Force-regenerate from scratch (discards the existing lock and re-resolves everything):

```bash
nous agent sync --refresh-lock
```

This is equivalent to deleting `Cargo.lock` and re-running `cargo build`. Use it after manually editing agent definition files or after resolving a merge conflict in `nous.lock`.

### 5.5 Reproducibility Guarantee

Two machines with the same `nous.lock` and the same source files (local `agents/`, `skills/`, and form directories at the recorded `form_ref`) will produce identical agent registration state. The guarantee holds because:

- SHA-256 hashes in the lockfile are content-addressed, not path-addressed.
- Form refs pin git commits, making form contents deterministic.
- The DB schema stores all agent fields written at registration time (`nous agent sync --refresh-lock` re-writes the DB from the lockfile).

The lockfile should be committed to version control when sharing an agent configuration across a team. Treat it like `Cargo.lock` for a binary: commit it for deployment repos, `.gitignore` it for reusable agent library repos (forms).

## 6. Recommendation

Build the file-based agent definition system in three phases. Each phase is independently shippable and produces user value without requiring the next phase.

### Phase 1 — Prototype (target: 1 sprint)

**Goal:** Make it possible to define an agent as a TOML file and register it with a single command, without touching forms or lockfiles.

**What to build:**

1. A TOML schema for agent definitions (`[agent]`, `[process]`, `[skills]`, `[tools]`, `[metadata]` sections as described in §3.2).
2. `nous agent add <file>` command that:
   - Parses the TOML using the existing `serde` + `toml` stack.
   - Calls the existing `agents::register_agent` (no new DB schema needed).
   - Resolves `[skills].refs` to local skill files under `~/.config/nous/skills/`.
3. `nous skill list` command showing local skills.
4. A few example definition files under `examples/agents/` in the repository.

**What not to build in Phase 1:** forms, lockfiles, `nous agent sync`, `nous form *` commands. These add distribution complexity before the basic format is validated.

**Done criteria:** A developer can `git clone` the nous repository, write a 20-line TOML file, run `nous agent add my-agent.toml`, and have the agent registered and visible in `nous agent list` — with no prior knowledge of the nous database schema.

### Phase 2 — Distribution (target: 2–3 sprints after Phase 1 ships)

**Goal:** Enable teams to share agent definitions via Git repositories.

**What to build:**

1. `nous form add <owner>/<repo>[@<ref>]` — clones a form repository.
2. `nous form update [<name>]` — git fetch + report changed files.
3. `nous form remove <name>` — removes form directory.
4. `nous form list` — lists installed forms.
5. Form-qualified agent references in `nous agent add paseo-org/monitor`.
6. Form-qualified skill refs in agent definition files (`[skills].refs`).
7. Namespace conflict detection and disambiguation errors (§4.5).

**What not to build in Phase 2:** the lockfile. Forms at HEAD are sufficient for team distribution before reproducibility is critical.

**Done criteria:** An operator can `nous form add acme-corp/agents`, run `nous agent add acme-corp/monitor`, and have the agent registered — pulling its skill files from the form, not from local files.

### Phase 3 — Reproducibility (target: 1–2 sprints after Phase 2 ships)

**Goal:** Guarantee identical agent state across machines, enable CI/CD workflows, and support offline environments.

**What to build:**

1. `nous.lock` file generation during `nous agent sync`.
2. `nous agent sync` command — full reconciliation of filesystem → DB with lockfile write.
3. `nous agent sync --refresh-lock` for forced regeneration.
4. Form `pinned_ref` support in `nous form add @<ref>` and `nous form update --upgrade`.
5. SHA-256 content verification on sync (detects tampering or manual edits).
6. Documentation on committing `nous.lock` vs. `.gitignore`-ing it.

**Done criteria:** Running `nous agent sync` on two machines with the same `~/.config/nous/agents/`, `~/.config/nous/forms/`, and `nous.lock` produces byte-identical JSON output from `nous agent list`.

### Rationale for Phasing

The three-phase approach reflects three different user needs that arrive at different project maturities:

| Phase | Primary user | Key need |
|-------|-------------|----------|
| 1 — Prototype | Individual developers | Author agents without editing SQL or raw API calls |
| 2 — Distribution | Small teams | Share proven agent definitions without a registry |
| 3 — Reproducibility | Ops / CI pipelines | Deploy agent configs deterministically |

Skipping to Phase 3 on the first iteration would produce a technically correct system that no one uses because the definition format is unfamiliar. Starting with Phase 1 validates the format with real usage before investing in the form distribution and lockfile machinery.

The total surface area across all three phases maps to approximately 5 new Rust source files (one per command group), 2 new `clap` subcommand trees (`nous form`, `nous skill`), and 1 new module (`crates/nous-cli/src/commands/form.rs`). No new database schema changes are required — all three phases consume the existing `agents`, `agent_versions`, and `agent_templates` tables already present in `crates/nous-core/src/agents/`.
