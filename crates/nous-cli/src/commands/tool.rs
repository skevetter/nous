use clap::Subcommand;

use nous_core::config::Config;

#[derive(Subcommand)]
pub enum ToolCommands {
    /// List all registered MCP tools
    List,
    /// Invoke an MCP tool by name
    Invoke {
        /// Tool name (e.g. "room_create")
        name: String,
        /// JSON arguments string (e.g. '{"name": "my-room"}')
        args: String,
    },
}

pub async fn run(cmd: ToolCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: ToolCommands, port: Option<u16>) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    let base = format!("http://127.0.0.1:{}", config.port);
    let client = reqwest::Client::new();

    match cmd {
        ToolCommands::List => {
            let resp = client
                .get(format!("{base}/mcp/tools"))
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let value: serde_json::Value = serde_json::from_str(&resp)?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        ToolCommands::Invoke { name, args } => {
            let arguments: serde_json::Value = serde_json::from_str(&args)?;
            let body = serde_json::json!({
                "name": name,
                "arguments": arguments,
            });
            let resp = client
                .post(format!("{base}/mcp/call"))
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let value: serde_json::Value = serde_json::from_str(&resp)?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
    }

    Ok(())
}
