use clap::{Parser, Subcommand};

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
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Doctor => {
            println!("doctor command not yet implemented");
        }
    }
}
