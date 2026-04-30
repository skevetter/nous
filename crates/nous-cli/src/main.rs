use clap::{Parser, Subcommand};

mod commands;

use commands::chat::ChatCommands;
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
        Commands::Chat { command } => {
            commands::chat::run(command).await;
        }
        Commands::Task { command } => {
            commands::task::run(command).await;
        }
        Commands::Worktree { command } => {
            commands::worktree::run(command).await;
        }
        Commands::Serve => {
            commands::serve::run().await;
        }
    }
}
