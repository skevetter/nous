use clap::Subcommand;
use rig::client::completion::CompletionClient;
use rig::client::ProviderClient;
use rig::completion::Prompt as _;

const DEFAULT_MODEL: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";

#[derive(Subcommand)]
pub enum LlmCommands {
    /// Send a hello world prompt to the LLM
    Hello {
        /// Optional custom prompt
        #[arg(long, default_value = "Say hello and introduce yourself in one sentence.")]
        prompt: String,
    },
}

pub async fn run(cmd: LlmCommands) {
    match cmd {
        LlmCommands::Hello { prompt } => {
            let client = match rig_bedrock::client::Client::from_env() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Failed to create Bedrock client. Ensure AWS_REGION, \
                         AWS_ACCESS_KEY_ID, and AWS_SECRET_ACCESS_KEY are set: {e}"
                    );
                    std::process::exit(1);
                }
            };

            let agent = client.agent(DEFAULT_MODEL).build();

            match agent.prompt(&prompt).await {
                Ok(response) => println!("{response}"),
                Err(e) => {
                    eprintln!("LLM request failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
