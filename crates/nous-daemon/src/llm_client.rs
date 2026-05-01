use aws_credential_types::Credentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4::SigningParams;
use nous_core::error::NousError;
use std::time::SystemTime;

const DEFAULT_REGION: &str = "us-east-1";
const DEFAULT_MODEL: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";
const SERVICE: &str = "bedrock";

#[derive(Debug)]
pub struct LlmClient {
    http_client: reqwest::Client,
    region: String,
    pub default_model: String,
    credentials: Credentials,
}

impl LlmClient {
    pub fn from_env() -> Result<Self, NousError> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| NousError::Config("AWS_ACCESS_KEY_ID not set".into()))?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| NousError::Config("AWS_SECRET_ACCESS_KEY not set".into()))?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();

        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| DEFAULT_REGION.to_string());

        let default_model =
            std::env::var("NOUS_LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

        let credentials = Credentials::new(
            access_key_id,
            secret_access_key,
            session_token,
            None,
            "nous-daemon",
        );

        Ok(Self {
            http_client: reqwest::Client::new(),
            region,
            default_model,
            credentials,
        })
    }

    pub async fn invoke(&self, model_id: &str, prompt: &str) -> Result<String, NousError> {
        let url = converse_url(&self.region, model_id);
        let body = build_request_body(prompt);
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| NousError::Internal(format!("failed to serialize request: {e}")))?;

        let mut request = http::Request::builder()
            .method("POST")
            .uri(&url)
            .header("content-type", "application/json")
            .body(body_bytes.clone())
            .map_err(|e| NousError::Internal(format!("failed to build HTTP request: {e}")))?;

        sign_request(&mut request, &self.credentials, &self.region)?;

        let mut reqwest_request = self
            .http_client
            .post(&url)
            .header("content-type", "application/json")
            .body(body_bytes);

        for (name, value) in request.headers() {
            reqwest_request = reqwest_request.header(name.as_str(), value);
        }

        let response = reqwest_request
            .send()
            .await
            .map_err(|e| NousError::Internal(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        let response_body = response
            .text()
            .await
            .map_err(|e| NousError::Internal(format!("failed to read response body: {e}")))?;

        if !status.is_success() {
            return Err(NousError::Internal(format!(
                "Bedrock API returned {status}: {response_body}"
            )));
        }

        parse_converse_response(&response_body)
    }
}

fn converse_url(region: &str, model_id: &str) -> String {
    format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model_id}/converse")
}

fn build_request_body(prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "messages": [{
            "role": "user",
            "content": [{"text": prompt}]
        }]
    })
}

fn sign_request(
    request: &mut http::Request<Vec<u8>>,
    credentials: &Credentials,
    region: &str,
) -> Result<(), NousError> {
    let identity = credentials.clone().into();
    let signing_params = SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(SERVICE)
        .time(SystemTime::now())
        .settings(SigningSettings::default())
        .build()
        .map_err(|e| NousError::Internal(format!("failed to build signing params: {e}")))?
        .into();

    let body_bytes = request.body().as_slice();
    let signable = SignableRequest::new(
        request.method().as_str(),
        request.uri().to_string(),
        request
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str(), std::str::from_utf8(v.as_bytes()).unwrap_or(""))),
        SignableBody::Bytes(body_bytes),
    )
    .map_err(|e| NousError::Internal(format!("failed to create signable request: {e}")))?;

    let (instructions, _signature) = sign(signable, &signing_params)
        .map_err(|e| NousError::Internal(format!("SigV4 signing failed: {e}")))?
        .into_parts();

    instructions.apply_to_request_http1x(request);
    Ok(())
}

fn parse_converse_response(body: &str) -> Result<String, NousError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| NousError::Internal(format!("failed to parse response JSON: {e}")))?;

    json["output"]["message"]["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|item| item["text"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| NousError::Internal(format!("unexpected response structure: {body}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_converse_url() {
        let url = converse_url("us-east-1", "anthropic.claude-v2");
        assert_eq!(
            url,
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-v2/converse"
        );
    }

    #[test]
    fn test_converse_url_different_region() {
        let url = converse_url("eu-west-1", "my-model");
        assert_eq!(
            url,
            "https://bedrock-runtime.eu-west-1.amazonaws.com/model/my-model/converse"
        );
    }

    #[test]
    fn test_build_request_body() {
        let body = build_request_body("Hello world");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "Hello world");
    }

    #[test]
    fn test_build_request_body_special_chars() {
        let body = build_request_body("Say \"hello\" & <goodbye>");
        let text = body["messages"][0]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "Say \"hello\" & <goodbye>");
    }

    #[test]
    fn test_parse_converse_response_valid() {
        let body = serde_json::json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{"text": "Hello, I'm Claude!"}]
                }
            },
            "stopReason": "end_turn"
        });
        let result = parse_converse_response(&body.to_string()).unwrap();
        assert_eq!(result, "Hello, I'm Claude!");
    }

    #[test]
    fn test_parse_converse_response_empty_content() {
        let body = serde_json::json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": []
                }
            }
        });
        let result = parse_converse_response(&body.to_string());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unexpected response structure"));
    }

    #[test]
    fn test_parse_converse_response_missing_output() {
        let body = serde_json::json!({"error": "something went wrong"});
        let result = parse_converse_response(&body.to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_converse_response_invalid_json() {
        let result = parse_converse_response("not json");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed to parse response JSON"));
    }

    #[test]
    fn test_sign_request_produces_auth_header() {
        let credentials = Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None::<String>,
            None,
            "test",
        );
        let body = b"{}".to_vec();
        let mut request = http::Request::builder()
            .method("POST")
            .uri("https://bedrock-runtime.us-east-1.amazonaws.com/model/test/converse")
            .header("content-type", "application/json")
            .body(body)
            .unwrap();

        sign_request(&mut request, &credentials, "us-east-1").unwrap();

        assert!(request.headers().contains_key("authorization"));
        let auth = request.headers()["authorization"].to_str().unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256"));
        assert!(auth.contains("bedrock"));
    }

    #[test]
    fn test_sign_request_with_session_token() {
        let credentials = Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            Some("FwoGZXIvYXdzEBY".to_string()),
            None,
            "test",
        );
        let body = b"{}".to_vec();
        let mut request = http::Request::builder()
            .method("POST")
            .uri("https://bedrock-runtime.us-east-1.amazonaws.com/model/test/converse")
            .header("content-type", "application/json")
            .body(body)
            .unwrap();

        sign_request(&mut request, &credentials, "us-east-1").unwrap();

        assert!(request.headers().contains_key("authorization"));
        assert!(request.headers().contains_key("x-amz-security-token"));
    }

    #[test]
    fn test_from_env_missing_access_key() {
        temp_env::with_vars(
            [
                ("AWS_ACCESS_KEY_ID", None::<&str>),
                ("AWS_SECRET_ACCESS_KEY", None::<&str>),
            ],
            || {
                let result = LlmClient::from_env();
                assert!(result.is_err());
                let err = result.unwrap_err().to_string();
                assert!(err.contains("AWS_ACCESS_KEY_ID"));
            },
        );
    }

    #[test]
    fn test_from_env_missing_secret_key() {
        temp_env::with_vars(
            [
                ("AWS_ACCESS_KEY_ID", Some("AKIA...")),
                ("AWS_SECRET_ACCESS_KEY", None::<&str>),
            ],
            || {
                let result = LlmClient::from_env();
                assert!(result.is_err());
                let err = result.unwrap_err().to_string();
                assert!(err.contains("AWS_SECRET_ACCESS_KEY"));
            },
        );
    }

    #[test]
    fn test_from_env_success_with_defaults() {
        temp_env::with_vars(
            [
                ("AWS_ACCESS_KEY_ID", Some("AKIAIOSFODNN7EXAMPLE")),
                ("AWS_SECRET_ACCESS_KEY", Some("wJalrXUtnFEMI")),
                ("AWS_SESSION_TOKEN", None::<&str>),
                ("AWS_REGION", None::<&str>),
                ("AWS_DEFAULT_REGION", None::<&str>),
                ("NOUS_LLM_MODEL", None::<&str>),
            ],
            || {
                let client = LlmClient::from_env().unwrap();
                assert_eq!(client.region, "us-east-1");
                assert_eq!(client.default_model, DEFAULT_MODEL);
            },
        );
    }

    #[test]
    fn test_from_env_custom_region_and_model() {
        temp_env::with_vars(
            [
                ("AWS_ACCESS_KEY_ID", Some("AKIAIOSFODNN7EXAMPLE")),
                ("AWS_SECRET_ACCESS_KEY", Some("wJalrXUtnFEMI")),
                ("AWS_REGION", Some("eu-west-1")),
                ("NOUS_LLM_MODEL", Some("my-custom-model")),
            ],
            || {
                let client = LlmClient::from_env().unwrap();
                assert_eq!(client.region, "eu-west-1");
                assert_eq!(client.default_model, "my-custom-model");
            },
        );
    }

    #[test]
    fn test_from_env_fallback_to_default_region() {
        temp_env::with_vars(
            [
                ("AWS_ACCESS_KEY_ID", Some("AKIAIOSFODNN7EXAMPLE")),
                ("AWS_SECRET_ACCESS_KEY", Some("wJalrXUtnFEMI")),
                ("AWS_REGION", None::<&str>),
                ("AWS_DEFAULT_REGION", Some("ap-southeast-1")),
            ],
            || {
                let client = LlmClient::from_env().unwrap();
                assert_eq!(client.region, "ap-southeast-1");
            },
        );
    }
}
