use clap::{Parser, Subcommand};

mod commands;

use commands::agent::AgentCommands;
use commands::artifact::ArtifactCommands;
use commands::chat::ChatCommands;
use commands::schedule::ScheduleCommands;
use commands::task::TaskCommands;
use commands::worktree::WorktreeCommands;

#[derive(Parser)]
#[command(name = "nous", about = "The nous platform CLI")]
struct Cli {
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
    /// Start the HTTP daemon
    Serve,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Doctor => {
            commands::doctor::run().await;
        }
        Commands::Agent { command } => {
            commands::agent::run(command).await;
        }
        Commands::Artifact { command } => {
            commands::artifact::run(command).await;
        }
        Commands::Chat { command } => {
            commands::chat::run(command).await;
        }
        Commands::Task { command } => {
            commands::task::run(command).await;
        }
        Commands::Schedule { command } => {
            commands::schedule::run(command).await;
        }
        Commands::Worktree { command } => {
            commands::worktree::run(command).await;
        }
        Commands::Serve => {
            commands::serve::run().await;
        }
    }
}
