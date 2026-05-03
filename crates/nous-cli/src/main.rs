use clap::{Parser, Subcommand};

mod commands;

use commands::agent::AgentCommands;
use commands::artifact::ArtifactCommands;
use commands::chat::ChatCommands;
use commands::inventory::InventoryCommands;
use commands::memory::MemoryCommands;
use commands::model::ModelCommands;
use commands::notifications::NotificationCommands;
use commands::resource::ResourceCommands;
use commands::schedule::ScheduleCommands;
use commands::skill::SkillCommands;
use commands::task::TaskCommands;
use commands::tool::ToolCommands;
use commands::worktree::WorktreeCommands;

#[derive(Parser)]
#[command(name = "nous", about = "The nous platform CLI", version)]
struct Cli {
    #[arg(long, global = true, help = "Override daemon port")]
    port: Option<u16>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run diagnostic checks
    Doctor,
    /// Agent registry operations
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Artifact registry operations
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommands,
    },
    /// Chat room operations
    Chat {
        #[command(subcommand)]
        command: ChatCommands,
    },
    /// Inventory management (P5 artifact registry)
    Inventory {
        #[command(subcommand)]
        command: InventoryCommands,
    },
    /// Unified resource management (replaces artifact + inventory)
    Resource {
        #[command(subcommand)]
        command: ResourceCommands,
    },
    /// Memory operations (P6 persistent structured memory)
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
    /// Manage embedding model files
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },
    /// Task management operations
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    /// Schedule management operations
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommands,
    },
    /// Skill management operations
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
    /// Tool registry operations (list and invoke MCP tools)
    Tool {
        #[command(subcommand)]
        command: ToolCommands,
    },
    /// Notification management (subscriptions and testing)
    Notifications {
        #[command(subcommand)]
        command: NotificationCommands,
    },
    /// Git worktree operations
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
    /// Start the MCP server (stdio transport for agent integration)
    McpServer {
        /// Comma-separated tool prefixes to expose (e.g. "chat,task" exposes room_ and task_ tools)
        #[arg(long)]
        tools: Option<String>,
        /// LLM model ID (e.g. anthropic.claude-sonnet-4-20250514-v1:0)
        #[arg(long)]
        model: Option<String>,
        /// AWS region for Bedrock
        #[arg(long)]
        region: Option<String>,
        /// AWS profile name
        #[arg(long)]
        profile: Option<String>,
    },
    /// Start the HTTP daemon
    Serve {
        /// LLM model ID (e.g. anthropic.claude-sonnet-4-20250514-v1:0)
        #[arg(long)]
        model: Option<String>,
        /// AWS region for Bedrock
        #[arg(long)]
        region: Option<String>,
        /// AWS profile name
        #[arg(long)]
        profile: Option<String>,
        /// Run as a background daemon
        #[arg(long)]
        daemon: bool,
        /// Internal flag: indicates this process IS the daemon (skip re-fork)
        #[arg(long, hide = true)]
        foreground_daemon: bool,
    },
    /// Start the HTTP daemon in the background (alias for `serve --daemon`)
    Start {
        /// LLM model ID (e.g. anthropic.claude-sonnet-4-20250514-v1:0)
        #[arg(long)]
        model: Option<String>,
        /// AWS region for Bedrock
        #[arg(long)]
        region: Option<String>,
        /// AWS profile name
        #[arg(long)]
        profile: Option<String>,
    },
    /// Reload daemon configuration (sends SIGHUP)
    Reload,
    /// Show daemon status
    Status,
    /// Stop the running daemon
    Stop,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let port = cli.port;

    match cli.command {
        Commands::Doctor => {
            commands::doctor::run(port).await;
        }
        Commands::Agent { command } => {
            commands::agent::run(command, port).await;
        }
        Commands::Artifact { command } => {
            commands::artifact::run(command, port).await;
        }
        Commands::Chat { command } => {
            commands::chat::run(command, port).await;
        }
        Commands::Inventory { command } => {
            commands::inventory::run(command, port).await;
        }
        Commands::Resource { command } => {
            commands::resource::run(command, port).await;
        }
        Commands::Memory { command } => {
            commands::memory::run(command, port).await;
        }
        Commands::Model { command } => {
            commands::model::run(command, port).await;
        }
        Commands::Task { command } => {
            commands::task::run(command, port).await;
        }
        Commands::Schedule { command } => {
            commands::schedule::run(command, port).await;
        }
        Commands::Skill { command } => {
            commands::skill::run(command, port).await;
        }
        Commands::Tool { command } => {
            commands::tool::run(command, port).await;
        }
        Commands::Notifications { command } => {
            commands::notifications::run(command, port).await;
        }
        Commands::Worktree { command } => {
            commands::worktree::run(command, port).await;
        }
        Commands::McpServer {
            tools,
            model,
            region,
            profile,
        } => {
            commands::mcp_server::run(tools, model, region, profile, port).await;
        }
        Commands::Serve {
            model,
            region,
            profile,
            daemon,
            foreground_daemon,
        } => {
            commands::serve::run(model, region, profile, port, daemon, foreground_daemon).await;
        }
        Commands::Start {
            model,
            region,
            profile,
        } => {
            commands::serve::run(model, region, profile, port, true, false).await;
        }
        Commands::Reload => {
            commands::reload::run().await;
        }
        Commands::Status => {
            commands::status::run().await;
        }
        Commands::Stop => {
            commands::stop::run().await;
        }
    }
}
