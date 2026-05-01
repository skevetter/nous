use rig_bedrock::client::{Client, ClientBuilder, DEFAULT_AWS_REGION};

pub type LlmClient = Client;

pub const DEFAULT_MODEL: &str = "anthropic.claude-sonnet-4-20250514-v1:0";

pub async fn build_client() -> Client {
    let region =
        std::env::var("AWS_REGION").unwrap_or_else(|_| DEFAULT_AWS_REGION.to_string());
    ClientBuilder::default().region(&region).build().await
}
