use std::path::PathBuf;

use clap::Subcommand;
use sha2::{Digest, Sha256};

use nous_core::agents;
use nous_core::config::Config;
use nous_core::db::DbPools;

#[derive(Subcommand)]
pub enum AgentCommands {
    /// Add an agent from a TOML definition file
    Add {
        /// Path to the TOML definition file
        file: String,
    },
    /// Remove an agent by name or ID
    Remove {
        /// Agent name or UUID
        name_or_id: String,
        /// Namespace (for name resolution; defaults to 'default')
        #[arg(long)]
        namespace: Option<String>,
        /// Cascade delete children
        #[arg(long)]
        cascade: bool,
    },
    /// Register a new agent in the org hierarchy
    Register {
        /// Agent name (must be unique within namespace)
        #[arg(long)]
        name: String,
        /// Agent type: engineer, manager, director, senior-manager
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
        /// Parent agent ID
        #[arg(long)]
        parent: Option<String>,
        /// Namespace (defaults to config or 'default')
        #[arg(long)]
        namespace: Option<String>,
        /// Room name for this agent
        #[arg(long)]
        room: Option<String>,
        /// JSON metadata string
        #[arg(long)]
        metadata: Option<String>,
        /// Initial status
        #[arg(long)]
        status: Option<String>,
    },
    /// Update agent config (process type, spawn command, working dir, etc.)
    Update {
        /// Agent ID
        id: String,
        /// Process type: shell, claude, http
        #[arg(long)]
        process_type: Option<String>,
        /// Default spawn command
        #[arg(long)]
        spawn_command: Option<String>,
        /// Working directory
        #[arg(long)]
        working_dir: Option<String>,
        /// Auto-restart on failure
        #[arg(long)]
        auto_restart: Option<bool>,
        /// JSON metadata string
        #[arg(long)]
        metadata: Option<String>,
    },
    /// Deregister an agent (remove from registry)
    Deregister {
        /// Agent ID or name
        id: String,
        /// Cascade delete children
        #[arg(long)]
        cascade: bool,
    },
    /// Look up an agent by name
    Lookup {
        /// Agent name
        name: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// List agents with optional filters
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by agent type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show the agent tree (hierarchy)
    Tree {
        /// Root agent ID (omit for full tree)
        #[arg(long)]
        root: Option<String>,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Send a heartbeat for an agent
    Heartbeat {
        /// Agent ID
        id: String,
        /// New status (running, idle, blocked, done)
        #[arg(long)]
        status: Option<String>,
    },
    /// List children of an agent
    Children {
        /// Parent agent ID
        id: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// List ancestors of an agent (root to parent)
    Ancestors {
        /// Agent ID
        id: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Search agents by name/metadata
    Search {
        /// Search query
        query: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// List stale agents (past heartbeat threshold)
    Stale {
        /// Threshold in seconds (default 900)
        #[arg(long, default_value = "900")]
        threshold: u64,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Inspect an agent (full details with version and template)
    Inspect {
        /// Agent ID
        id: String,
    },
    /// List version history for an agent
    Versions {
        /// Agent ID
        id: String,
        /// Max results
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Record a version for an agent (skills + config hashes)
    RecordVersion {
        /// Agent ID
        #[arg(long)]
        agent_id: String,
        /// SHA-256 hash of concatenated skill contents
        #[arg(long)]
        skill_hash: String,
        /// SHA-256 hash of effective config
        #[arg(long)]
        config_hash: String,
        /// JSON array of skill details: [{name, path, hash}]
        #[arg(long)]
        skills_json: Option<String>,
    },
    /// Rollback an agent to a previous version
    Rollback {
        /// Agent ID
        id: String,
        /// Target version ID
        #[arg(long)]
        version: String,
    },
    /// Update agent status with optional reason
    Status {
        /// Agent ID
        id: String,
        /// New status
        status: String,
    },
    /// List agents with upgrade_available flag set
    Outdated {
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Notify an agent that an upgrade is available
    NotifyUpgrade {
        /// Agent ID
        id: String,
    },
    /// Template operations
    Template {
        #[command(subcommand)]
        command: TemplateCommands,
    },
    /// Spawn an agent process
    Spawn {
        /// Agent ID
        id: String,
        /// Command to run
        #[arg(long)]
        command: Option<String>,
        /// Process type: claude, shell, http, sandbox
        #[arg(long, default_value = "shell")]
        r#type: String,
        /// Working directory
        #[arg(long)]
        working_dir: Option<String>,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<i64>,
        /// Restart policy: never, on-failure, always
        #[arg(long, default_value = "never")]
        restart: String,
        /// OCI image for the sandbox (e.g. python:3.12). Requires --type sandbox
        #[arg(long)]
        sandbox_image: Option<String>,
        /// Number of vCPUs for the sandbox. Requires --type sandbox
        #[arg(long)]
        sandbox_cpus: Option<u32>,
        /// Memory in MiB for the sandbox. Requires --type sandbox
        #[arg(long)]
        sandbox_memory: Option<u32>,
        /// Network policy: none, public-only, allow-all. Requires --type sandbox
        #[arg(long)]
        sandbox_network: Option<String>,
        /// Volume mounts in format 'guest_path:host_path[:ro]' (repeatable). Requires --type sandbox
        #[arg(long)]
        sandbox_volume: Vec<String>,
    },
    /// Stop a running agent process
    Stop {
        /// Agent ID
        id: String,
        /// Force kill immediately (SIGKILL)
        #[arg(long)]
        force: bool,
        /// Grace period in seconds before SIGKILL
        #[arg(long, default_value = "10")]
        grace: u64,
    },
    /// Restart an agent process
    Restart {
        /// Agent ID
        id: String,
        /// New command
        #[arg(long)]
        command: Option<String>,
    },
    /// Send work to an agent
    Invoke {
        /// Agent ID
        id: String,
        /// Work prompt
        #[arg(long)]
        prompt: String,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<i64>,
        /// Return immediately with invocation ID
        #[arg(long, name = "async")]
        is_async: bool,
    },
    /// Get result of an async invocation
    InvokeResult {
        /// Invocation ID
        invocation_id: String,
    },
    /// List invocation history for an agent
    Invocations {
        /// Agent ID
        id: String,
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Max results
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// List all running agent processes
    Ps,
    /// Get logs/process history for an agent
    Logs {
        /// Agent ID
        id: String,
        /// Max process records
        #[arg(long, default_value = "5")]
        lines: u32,
    },
    /// Sandbox management subcommands (see Agent Skills Specification for tools declaration)
    Sandbox {
        #[command(subcommand)]
        command: SandboxCommands,
    },
}

#[derive(Subcommand)]
pub enum SandboxCommands {
    /// Show status of a sandboxed agent process
    Status {
        /// Agent ID
        id: String,
    },
    /// Show resource metrics for a sandboxed agent
    Metrics {
        /// Agent ID
        id: String,
    },
    /// Execute a command inside a running sandbox (runtime integration in Task 4)
    Exec {
        /// Agent ID
        id: String,
        /// Command and arguments to execute
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum TemplateCommands {
    /// Create a new agent template (immutable)
    Create {
        /// Template name (unique)
        #[arg(long)]
        name: String,
        /// Template type (e.g. engineer, reviewer, monitor)
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
        /// Default config JSON
        #[arg(long)]
        config: Option<String>,
        /// Skill refs JSON array
        #[arg(long)]
        skills: Option<String>,
    },
    /// List templates
    List {
        /// Filter by type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Get a template by ID
    Get {
        /// Template ID
        id: String,
    },
    /// Create a new agent from a template
    Instantiate {
        /// Template ID
        template_id: String,
        /// Agent name override
        #[arg(long)]
        name: Option<String>,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Parent agent ID
        #[arg(long)]
        parent: Option<String>,
        /// Config overrides JSON
        #[arg(long)]
        config_overrides: Option<String>,
    },
}

pub async fn run(cmd: AgentCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: AgentCommands, port: Option<u16>) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;

    match cmd {
        AgentCommands::Add { file } => {
            let path = PathBuf::from(&file);
            let toml_content = std::fs::read(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let def = agents::definition::load_definition(&path)?;

            let agent_type: agents::AgentType = def.agent.r#type.parse()?;
            let agent = agents::register_agent(
                pool,
                agents::RegisterAgentRequest {
                    name: def.agent.name,
                    agent_type,
                    parent_id: None,
                    namespace: def.agent.namespace,
                    room: None,
                    metadata: None,
                    status: Some(agents::AgentStatus::Idle),
                },
            )
            .await?;

            if let Some(process) = &def.process {
                agents::processes::update_agent(
                    pool,
                    &agent.id,
                    process.r#type.as_deref(),
                    process.spawn_command.as_deref(),
                    process.working_dir.as_deref(),
                    process.auto_restart,
                    None,
                )
                .await?;
            }

            if let Some(skills) = &def.skills {
                let skills_dir = dirs::config_dir()
                    .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
                    .ok_or("cannot determine config directory")?
                    .join("nous")
                    .join("skills");
                let resolved = agents::definition::resolve_skills(&skills.refs, &skills_dir)?;

                let mut hasher = Sha256::new();
                for s in &resolved {
                    hasher.update(s.content.as_bytes());
                }
                let hex_hash = format!("{:x}", hasher.finalize());

                let config_hash = format!("{:x}", Sha256::digest(&toml_content));

                let skills_json: Vec<serde_json::Value> = resolved
                    .iter()
                    .map(|s| {
                        let h = format!("{:x}", Sha256::digest(s.content.as_bytes()));
                        serde_json::json!({
                            "name": s.name,
                            "path": s.path.to_string_lossy(),
                            "hash": h,
                        })
                    })
                    .collect();
                let skills_json_str = serde_json::to_string(&skills_json)?;

                agents::record_version(
                    pool,
                    agents::RecordVersionRequest {
                        agent_id: agent.id.clone(),
                        skill_hash: hex_hash,
                        config_hash,
                        skills_json: Some(skills_json_str),
                    },
                )
                .await?;
            }

            if let Some(metadata) = &def.metadata {
                let metadata_json = serde_json::to_string(metadata)?;
                agents::processes::update_agent(
                    pool,
                    &agent.id,
                    None,
                    None,
                    None,
                    None,
                    Some(&metadata_json),
                )
                .await?;
            }

            let final_agent = agents::get_agent_by_id(pool, &agent.id).await?;
            println!("{}", serde_json::to_string_pretty(&final_agent)?);
        }
        AgentCommands::Remove {
            name_or_id,
            namespace,
            cascade,
        } => {
            let resolved_id = if looks_like_uuid(&name_or_id) {
                name_or_id
            } else {
                let agent = agents::lookup_agent(pool, &name_or_id, namespace.as_deref()).await?;
                agent.id
            };
            let result = agents::deregister_agent(pool, &resolved_id, cascade).await?;
            println!("{{\"result\": \"{result}\"}}");
        }
        AgentCommands::Register {
            name,
            r#type,
            parent,
            namespace,
            room,
            metadata,
            status,
        } => {
            let agent_type: agents::AgentType = r#type.parse()?;
            let agent_status = status
                .as_deref()
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            let agent = agents::register_agent(
                pool,
                agents::RegisterAgentRequest {
                    name,
                    agent_type,
                    parent_id: parent,
                    namespace,
                    room,
                    metadata,
                    status: agent_status,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&agent)?);
        }
        AgentCommands::Update {
            id,
            process_type,
            spawn_command,
            working_dir,
            auto_restart,
            metadata,
        } => {
            let agent = agents::processes::update_agent(
                pool,
                &id,
                process_type.as_deref(),
                spawn_command.as_deref(),
                working_dir.as_deref(),
                auto_restart,
                metadata.as_deref(),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&agent)?);
        }
        AgentCommands::Deregister { id, cascade } => {
            let resolved_id = if looks_like_uuid(&id) {
                id
            } else {
                let agent = agents::lookup_agent(pool, &id, None).await?;
                agent.id
            };
            let result = agents::deregister_agent(pool, &resolved_id, cascade).await?;
            println!("{{\"result\": \"{result}\"}}");
        }
        AgentCommands::Lookup { name, namespace } => {
            let agent = agents::lookup_agent(pool, &name, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&agent)?);
        }
        AgentCommands::List {
            status,
            r#type,
            namespace,
            limit,
        } => {
            let status_parsed = status
                .as_deref()
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<agents::AgentType>())
                .transpose()?;
            let list = agents::list_agents(
                pool,
                &agents::ListAgentsFilter {
                    namespace,
                    status: status_parsed,
                    agent_type: type_parsed,
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        AgentCommands::Tree { root, namespace } => {
            let tree = agents::get_tree(pool, root.as_deref(), namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&tree)?);
        }
        AgentCommands::Heartbeat { id, status } => {
            let agent_status = status
                .as_deref()
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            agents::heartbeat(pool, &id, agent_status).await?;
            println!("{{\"ok\": true}}");
        }
        AgentCommands::Children { id, namespace } => {
            let children = agents::list_children(pool, &id, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&children)?);
        }
        AgentCommands::Ancestors { id, namespace } => {
            let ancestors = agents::list_ancestors(pool, &id, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&ancestors)?);
        }
        AgentCommands::Search {
            query,
            namespace,
            limit,
        } => {
            let results =
                agents::search_agents(pool, &query, namespace.as_deref(), Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        AgentCommands::Stale {
            threshold,
            namespace,
        } => {
            let stale = agents::list_stale_agents(pool, threshold, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&stale)?);
        }
        AgentCommands::Inspect { id } => {
            let resolved_id = if looks_like_uuid(&id) {
                id
            } else {
                let agent = agents::lookup_agent(pool, &id, None).await?;
                agent.id
            };
            let inspection = agents::inspect_agent(pool, &resolved_id).await?;
            println!("{}", serde_json::to_string_pretty(&inspection)?);
        }
        AgentCommands::Versions { id, limit } => {
            let versions = agents::list_versions(pool, &id, Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&versions)?);
        }
        AgentCommands::RecordVersion {
            agent_id,
            skill_hash,
            config_hash,
            skills_json,
        } => {
            let version = agents::record_version(
                pool,
                agents::RecordVersionRequest {
                    agent_id,
                    skill_hash,
                    config_hash,
                    skills_json,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&version)?);
        }
        AgentCommands::Rollback { id, version } => {
            let v = agents::rollback_agent(pool, &id, &version).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        AgentCommands::Status { id, status } => {
            let agent_status: agents::AgentStatus = status.parse()?;
            let agent = agents::update_agent_status(pool, &id, agent_status).await?;
            println!("{}", serde_json::to_string_pretty(&agent)?);
        }
        AgentCommands::Outdated { namespace, limit } => {
            let outdated =
                agents::list_outdated_agents(pool, namespace.as_deref(), Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&outdated)?);
        }
        AgentCommands::NotifyUpgrade { id } => {
            agents::set_upgrade_available(pool, &id, true).await?;
            println!("{{\"notified\": true}}");
        }
        AgentCommands::Template { command } => match command {
            TemplateCommands::Create {
                name,
                r#type,
                config,
                skills,
            } => {
                let template = agents::create_template(
                    pool,
                    agents::CreateTemplateRequest {
                        name,
                        template_type: r#type,
                        default_config: config,
                        skill_refs: skills,
                    },
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&template)?);
            }
            TemplateCommands::List { r#type, limit } => {
                let templates =
                    agents::list_templates(pool, r#type.as_deref(), Some(limit)).await?;
                println!("{}", serde_json::to_string_pretty(&templates)?);
            }
            TemplateCommands::Get { id } => {
                let template = agents::get_template_by_id(pool, &id).await?;
                println!("{}", serde_json::to_string_pretty(&template)?);
            }
            TemplateCommands::Instantiate {
                template_id,
                name,
                namespace,
                parent,
                config_overrides,
            } => {
                let agent = agents::instantiate_from_template(
                    pool,
                    agents::InstantiateRequest {
                        template_id,
                        name,
                        namespace,
                        parent_id: parent,
                        config_overrides,
                    },
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&agent)?);
            }
        },
        AgentCommands::Spawn {
            id,
            command,
            r#type,
            working_dir,
            timeout,
            restart,
            sandbox_image,
            sandbox_cpus,
            sandbox_memory,
            sandbox_network,
            sandbox_volume,
        } => {
            let has_sandbox_flags = sandbox_image.is_some()
                || sandbox_cpus.is_some()
                || sandbox_memory.is_some()
                || sandbox_network.is_some()
                || !sandbox_volume.is_empty();

            if has_sandbox_flags && r#type != "sandbox" {
                return Err("--sandbox-* flags require --type sandbox".into());
            }

            if r#type == "sandbox" {
                let image =
                    sandbox_image.ok_or("--sandbox-image is required when --type sandbox")?;

                if let Some(ref policy) = sandbox_network {
                    validate_network_policy(policy)
                        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                }

                let volumes_json = if sandbox_volume.is_empty() {
                    None
                } else {
                    let volumes: Vec<nous_core::agents::sandbox::VolumeMount> = sandbox_volume
                        .iter()
                        .map(|spec| parse_volume_spec(spec))
                        .collect::<Result<Vec<_>, _>>()?;
                    Some(serde_json::to_string(&volumes)?)
                };

                let process = agents::processes::create_sandbox_process(
                    agents::processes::CreateSandboxProcessParams {
                        db: pool,
                        agent_id: &id,
                        sandbox_image: &image,
                        sandbox_cpus,
                        sandbox_memory_mib: sandbox_memory,
                        sandbox_network_policy: sandbox_network.as_deref(),
                        sandbox_volumes_json: volumes_json.as_deref(),
                        sandbox_name: None,
                        timeout_secs: timeout,
                        restart_policy: Some(&restart),
                    },
                )
                .await?;
                agents::processes::update_agent(pool, &id, Some("sandbox"), None, None, None, None)
                    .await?;
                println!("{}", serde_json::to_string_pretty(&process)?);
            } else {
                let agent = agents::get_agent_by_id(pool, &id).await?;
                let cmd = command
                    .or(agent.spawn_command)
                    .ok_or("command is required (not set on agent config either)")?;
                let pt = if r#type == "shell" {
                    agent.process_type.unwrap_or_else(|| "shell".to_string())
                } else {
                    r#type
                };
                let process = agents::processes::create_process(
                    agents::processes::CreateProcessParams {
                        db: pool,
                        agent_id: &id,
                        process_type: &pt,
                        command: &cmd,
                        working_dir: working_dir.as_deref().or(agent.working_dir.as_deref()),
                        env_json: None,
                        timeout_secs: timeout,
                        restart_policy: Some(&restart),
                        max_restarts: None,
                    },
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&process)?);
            }
        }
        AgentCommands::Stop {
            id,
            force: _,
            grace: _,
        } => {
            // CLI stop updates DB status; runtime stop requires daemon
            if let Some(process) = agents::processes::get_active_process(pool, &id).await? {
                let process = agents::processes::update_process_status(
                    pool,
                    &process.id,
                    "stopped",
                    None,
                    None,
                    None,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&process)?);
            } else {
                eprintln!("no active process for agent '{id}'");
            }
        }
        AgentCommands::Restart { id, command } => {
            // Stop existing
            if let Some(process) = agents::processes::get_active_process(pool, &id).await? {
                agents::processes::update_process_status(
                    pool,
                    &process.id,
                    "stopped",
                    None,
                    None,
                    None,
                )
                .await?;
            }
            // Get agent config for defaults
            let agent = agents::get_agent_by_id(pool, &id).await?;
            let cmd = command
                .or(agent.spawn_command)
                .ok_or("command is required (not set on agent config either)")?;
            let pt = agent.process_type.unwrap_or_else(|| "shell".to_string());
            let process = agents::processes::create_process(
                agents::processes::CreateProcessParams {
                    db: pool,
                    agent_id: &id,
                    process_type: &pt,
                    command: &cmd,
                    working_dir: agent.working_dir.as_deref(),
                    env_json: None,
                    timeout_secs: None,
                    restart_policy: Some("never"),
                    max_restarts: None,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&process)?);
        }
        AgentCommands::Invoke {
            id,
            prompt,
            timeout,
            is_async,
        } => {
            let url = format!("http://{}:{}/mcp/call", config.host, config.port);
            let mut args = serde_json::json!({
                "agent_id": id,
                "prompt": prompt,
            });
            if let Some(t) = timeout {
                args["timeout_secs"] = serde_json::json!(t);
            }
            if is_async {
                args["async"] = serde_json::json!(true);
            }
            let body = serde_json::json!({
                "name": "agent_invoke",
                "arguments": args,
            });
            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("daemon unreachable at {url}: {e}"))?;
            let status_code = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("failed to read response: {e}"))?;
            if !status_code.is_success() {
                return Err(format!("daemon returned {status_code}: {text}").into());
            }
            let mcp_resp: serde_json::Value = serde_json::from_str(&text)?;
            if mcp_resp
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let msg = mcp_resp["content"][0]["text"]
                    .as_str()
                    .unwrap_or("unknown error");
                return Err(msg.to_string().into());
            }
            let content_text = mcp_resp["content"][0]["text"].as_str().unwrap_or("{}");
            let invocation: serde_json::Value = serde_json::from_str(content_text)?;
            println!("{}", serde_json::to_string_pretty(&invocation)?);
        }
        AgentCommands::InvokeResult { invocation_id } => {
            let url = format!("http://{}:{}/mcp/call", config.host, config.port);
            let body = serde_json::json!({
                "name": "agent_invoke_result",
                "arguments": {
                    "invocation_id": invocation_id,
                },
            });
            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("daemon unreachable at {url}: {e}"))?;
            let status_code = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("failed to read response: {e}"))?;
            if !status_code.is_success() {
                return Err(format!("daemon returned {status_code}: {text}").into());
            }
            let mcp_resp: serde_json::Value = serde_json::from_str(&text)?;
            if mcp_resp
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let msg = mcp_resp["content"][0]["text"]
                    .as_str()
                    .unwrap_or("unknown error");
                return Err(msg.to_string().into());
            }
            let content_text = mcp_resp["content"][0]["text"].as_str().unwrap_or("{}");
            let invocation: serde_json::Value = serde_json::from_str(content_text)?;
            println!("{}", serde_json::to_string_pretty(&invocation)?);
        }
        AgentCommands::Invocations { id, status, limit } => {
            let url = format!("http://{}:{}/mcp/call", config.host, config.port);
            let mut args = serde_json::json!({
                "agent_id": id,
            });
            if let Some(s) = status {
                args["status"] = serde_json::json!(s);
            }
            args["limit"] = serde_json::json!(limit);
            let body = serde_json::json!({
                "name": "agent_invocations",
                "arguments": args,
            });
            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("daemon unreachable at {url}: {e}"))?;
            let status_code = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("failed to read response: {e}"))?;
            if !status_code.is_success() {
                return Err(format!("daemon returned {status_code}: {text}").into());
            }
            let mcp_resp: serde_json::Value = serde_json::from_str(&text)?;
            if mcp_resp
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let msg = mcp_resp["content"][0]["text"]
                    .as_str()
                    .unwrap_or("unknown error");
                return Err(msg.to_string().into());
            }
            let content_text = mcp_resp["content"][0]["text"].as_str().unwrap_or("{}");
            let invocations: serde_json::Value = serde_json::from_str(content_text)?;
            println!("{}", serde_json::to_string_pretty(&invocations)?);
        }
        AgentCommands::Ps => {
            let processes = agents::processes::list_all_active_processes(pool).await?;
            println!("{}", serde_json::to_string_pretty(&processes)?);
        }
        AgentCommands::Logs { id, lines } => {
            let processes = agents::processes::list_processes(pool, &id, Some(lines)).await?;
            println!("{}", serde_json::to_string_pretty(&processes)?);
        }
        AgentCommands::Sandbox { command } => match command {
            SandboxCommands::Status { id } => {
                let process = agents::processes::get_active_process(pool, &id).await?;
                match process {
                    Some(p) if p.process_type == "sandbox" => {
                        println!("{}", serde_json::to_string_pretty(&p)?);
                    }
                    Some(_) => {
                        return Err(format!(
                            "agent '{id}' has an active process but it is not a sandbox"
                        )
                        .into());
                    }
                    None => {
                        let processes =
                            agents::processes::list_processes(pool, &id, Some(10)).await?;
                        let sandbox = processes.into_iter().find(|p| p.process_type == "sandbox");
                        match sandbox {
                            Some(p) => println!("{}", serde_json::to_string_pretty(&p)?),
                            None => {
                                return Err(
                                    format!("no sandbox process found for agent '{id}'").into()
                                )
                            }
                        }
                    }
                }
            }
            SandboxCommands::Metrics { id } => {
                let process = agents::processes::get_active_process(pool, &id).await?;
                match process {
                    Some(p) if p.process_type == "sandbox" => {
                        let metrics = serde_json::json!({
                            "agent_id": id,
                            "process_id": p.id,
                            "status": p.status,
                            "sandbox_image": p.sandbox_image,
                            "sandbox_cpus": p.sandbox_cpus,
                            "sandbox_memory_mib": p.sandbox_memory_mib,
                            "note": "live metrics available after runtime integration (Task 4)"
                        });
                        println!("{}", serde_json::to_string_pretty(&metrics)?);
                    }
                    _ => {
                        return Err(format!("no active sandbox process for agent '{id}'").into());
                    }
                }
            }
            SandboxCommands::Exec { id, args } => {
                if args.is_empty() {
                    return Err("exec requires a command after '--'".into());
                }
                let process = agents::processes::get_active_process(pool, &id).await?;
                match process {
                    Some(p) if p.process_type == "sandbox" && p.status == "running" => {
                        println!(
                            "sandbox exec not yet implemented (Task 4): would run {:?} in sandbox {} (process {})",
                            args,
                            p.sandbox_name.as_deref().unwrap_or(&id),
                            p.id
                        );
                    }
                    Some(p) if p.process_type == "sandbox" => {
                        return Err(format!(
                            "sandbox process for agent '{id}' is not running (status: {})",
                            p.status
                        )
                        .into());
                    }
                    _ => {
                        return Err(format!("no active sandbox process for agent '{id}'").into());
                    }
                }
            }
        },
    }

    pools.close().await;
    Ok(())
}

fn parse_volume_spec(spec: &str) -> Result<nous_core::agents::sandbox::VolumeMount, String> {
    let parts: Vec<&str> = spec.split(':').collect();
    match parts.len() {
        1 => Ok(nous_core::agents::sandbox::VolumeMount {
            guest_path: parts[0].to_string(),
            host_path: None,
            readonly: false,
        }),
        2 => Ok(nous_core::agents::sandbox::VolumeMount {
            guest_path: parts[0].to_string(),
            host_path: Some(parts[1].to_string()),
            readonly: false,
        }),
        3 => {
            let readonly = match parts[2] {
                "ro" => true,
                "rw" => false,
                other => {
                    return Err(format!(
                        "invalid volume flag '{other}', expected 'ro' or 'rw'"
                    ))
                }
            };
            Ok(nous_core::agents::sandbox::VolumeMount {
                guest_path: parts[0].to_string(),
                host_path: Some(parts[1].to_string()),
                readonly,
            })
        }
        _ => Err(format!(
            "invalid volume spec '{spec}': expected 'guest_path:host_path[:ro]'"
        )),
    }
}

fn looks_like_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

const VALID_NETWORK_POLICIES: &[&str] = &["none", "public-only", "allow-all"];

fn validate_network_policy(policy: &str) -> Result<(), String> {
    if VALID_NETWORK_POLICIES.contains(&policy) {
        Ok(())
    } else {
        Err(format!(
            "invalid network policy: {policy}. Must be one of: none, public-only, allow-all"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_volume_spec_guest_only() {
        let result = parse_volume_spec("/workspace").unwrap();
        assert_eq!(result.guest_path, "/workspace");
        assert_eq!(result.host_path, None);
        assert!(!result.readonly);
    }

    #[test]
    fn parse_volume_spec_guest_host() {
        let result = parse_volume_spec("/workspace:/host/path").unwrap();
        assert_eq!(result.guest_path, "/workspace");
        assert_eq!(result.host_path, Some("/host/path".to_string()));
        assert!(!result.readonly);
    }

    #[test]
    fn parse_volume_spec_guest_host_ro() {
        let result = parse_volume_spec("/workspace:/host/path:ro").unwrap();
        assert_eq!(result.guest_path, "/workspace");
        assert_eq!(result.host_path, Some("/host/path".to_string()));
        assert!(result.readonly);
    }

    #[test]
    fn parse_volume_spec_guest_host_rw() {
        let result = parse_volume_spec("/workspace:/host/path:rw").unwrap();
        assert_eq!(result.guest_path, "/workspace");
        assert_eq!(result.host_path, Some("/host/path".to_string()));
        assert!(!result.readonly);
    }

    #[test]
    fn parse_volume_spec_invalid_flag() {
        let result = parse_volume_spec("/workspace:/host/path:invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid volume flag"));
    }

    #[test]
    fn parse_volume_spec_too_many_parts() {
        let result = parse_volume_spec("/a:/b:ro:extra");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid volume spec"));
    }

    #[test]
    fn validate_network_policy_valid_values() {
        assert!(validate_network_policy("none").is_ok());
        assert!(validate_network_policy("public-only").is_ok());
        assert!(validate_network_policy("allow-all").is_ok());
    }

    #[test]
    fn validate_network_policy_invalid_values() {
        let err = validate_network_policy("bridge").unwrap_err();
        assert!(err.contains("invalid network policy: bridge"));
        assert!(err.contains("none, public-only, allow-all"));

        let err = validate_network_policy("").unwrap_err();
        assert!(err.contains("invalid network policy"));

        let err = validate_network_policy("NONE").unwrap_err();
        assert!(err.contains("invalid network policy: NONE"));
    }
}
