use clap::{Parser, Subcommand};

mod commands;

use commands::agent::AgentCommands;
use commands::artifact::ArtifactCommands;
use commands::chat::ChatCommands;
use commands::inventory::InventoryCommands;
use commands::memory::MemoryCommands;
use commands::model::ModelCommands;
use commands::schedule::ScheduleCommands;
use commands::task::TaskCommands;
use commands::worktree::WorktreeCommands;

#[derive(Parser)]
#[command(name = "nous", about = "The nous platform CLI")]
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
    },
    /// Start the HTTP daemon
    Serve,
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
        Commands::Worktree { command } => {
            commands::worktree::run(command, port).await;
        }
        Commands::McpServer { tools } => {
            commands::mcp_server::run(tools, port).await;
        }
        Commands::Serve => {
            commands::serve::run(port).await;
        }
    }
}
