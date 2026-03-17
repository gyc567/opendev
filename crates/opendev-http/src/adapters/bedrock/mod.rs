//! AWS Bedrock provider adapter.
//!
//! Transforms OpenAI Chat Completions payloads to Amazon Bedrock's
//! `InvokeModel` format and converts responses back.
//!
//! Bedrock uses SigV4 request signing. Since `aws-sigv4` is not available
//! as a dependency, the signing logic is stubbed with a TODO. In production,
//! either add `aws-sigv4`/`aws-credential-types` crates or implement
//! minimal HMAC-SHA256 signing with the `hmac` and `sha2` crates.
//!
//! Environment variables:
//! - `AWS_ACCESS_KEY_ID` — IAM access key
//! - `AWS_SECRET_ACCESS_KEY` — IAM secret key
//! - `AWS_REGION` — AWS region (defaults to `us-east-1`)
//! - `AWS_SESSION_TOKEN` — optional session token for temporary credentials

mod request;
mod response;

use serde_json::{Value, json};

/// Default AWS region when `AWS_REGION` is not set.
const DEFAULT_REGION: &str = "us-east-1";

/// Adapter for Amazon Bedrock's InvokeModel API.
///
/// Bedrock wraps foundation models behind a REST API at:
/// `https://bedrock-runtime.{region}.amazonaws.com/model/{model_id}/invoke`
///
/// This adapter handles:
/// - Converting Chat Completions messages to Bedrock's Anthropic-style format
/// - Building the correct endpoint URL from region + model
/// - SigV4 header generation (TODO: requires `hmac`/`sha2` crates)
#[derive(Debug, Clone)]
pub struct BedrockAdapter {
    region: String,
    model_id: String,
    api_url: String,
}

impl BedrockAdapter {
    /// Create a new Bedrock adapter for the given model.
    ///
    /// Reads `AWS_REGION` from the environment (defaults to `us-east-1`).
    pub fn new(model_id: impl Into<String>) -> Self {
        let model_id = model_id.into();
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| DEFAULT_REGION.to_string());
        let api_url = Self::build_url(&region, &model_id);
        Self {
            region,
            model_id,
            api_url,
        }
    }

    /// Create a new Bedrock adapter with a custom region.
    pub fn with_region(model_id: impl Into<String>, region: impl Into<String>) -> Self {
        let model_id = model_id.into();
        let region = region.into();
        let api_url = Self::build_url(&region, &model_id);
        Self {
            region,
            model_id,
            api_url,
        }
    }

    /// Build the Bedrock InvokeModel URL.
    fn build_url(region: &str, model_id: &str) -> String {
        format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model_id}/invoke")
    }

    /// Get the configured AWS region.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// Get the model ID.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Generate SigV4 authorization headers for the request.
    ///
    /// TODO: Implement SigV4 signing. Requires either:
    /// - Adding `aws-sigv4` + `aws-credential-types` crates, or
    /// - Adding `hmac` + `sha2` crates for minimal manual signing.
    ///
    /// For now, this returns empty headers. To use Bedrock in production,
    /// implement the SigV4 signing algorithm:
    /// 1. Create canonical request (method, URI, query, headers, payload hash)
    /// 2. Create string-to-sign (algorithm, date, scope, canonical request hash)
    /// 3. Derive signing key via HMAC-SHA256 chain (date → region → service → signing)
    /// 4. Calculate signature = HMAC-SHA256(signing_key, string_to_sign)
    /// 5. Build Authorization header
    #[allow(dead_code)]
    fn sigv4_headers(&self, _payload: &[u8]) -> Vec<(String, String)> {
        let _access_key = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
        let _secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
        let _session_token = std::env::var("AWS_SESSION_TOKEN").ok();

        // TODO: Implement SigV4 signing when `hmac` and `sha2` crates are available.
        vec![
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
        ]
    }
}

#[async_trait::async_trait]
impl super::base::ProviderAdapter for BedrockAdapter {
    fn provider_name(&self) -> &str {
        "bedrock"
    }

    fn convert_request(&self, mut payload: Value) -> Value {
        // Strip internal reasoning effort field (Bedrock doesn't support it)
        payload
            .as_object_mut()
            .map(|obj| obj.remove("_reasoning_effort"));

        request::extract_system(&mut payload);
        request::convert_tools(&mut payload);
        request::convert_tool_messages(&mut payload);
        request::ensure_max_tokens(&mut payload);

        // Bedrock wraps the model in the URL, not the payload.
        // Remove fields Bedrock does not accept.
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("model");
            obj.remove("n");
            obj.remove("frequency_penalty");
            obj.remove("presence_penalty");
            obj.remove("logprobs");
            obj.remove("stream");
        }

        // Set anthropic_version required by Bedrock's Anthropic models.
        payload["anthropic_version"] = json!("bedrock-2023-05-31");

        payload
    }

    fn convert_response(&self, response: Value) -> Value {
        response::response_to_chat_completions(response, &self.model_id)
    }

    fn api_url(&self) -> &str {
        &self.api_url
    }

    fn extra_headers(&self) -> Vec<(String, String)> {
        // TODO: SigV4 headers should be generated per-request with the payload.
        // The ProviderAdapter trait's `extra_headers()` is called without payload
        // context, so full SigV4 signing would require a trait extension.
        // For now, return content-type headers only.
        vec![
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::base::ProviderAdapter;

    #[test]
    fn test_provider_name() {
        let adapter = BedrockAdapter::new("anthropic.claude-3-sonnet-20240229-v1:0");
        assert_eq!(adapter.provider_name(), "bedrock");
    }

    #[test]
    fn test_api_url_format() {
        let adapter =
            BedrockAdapter::with_region("anthropic.claude-3-sonnet-20240229-v1:0", "us-west-2");
        assert_eq!(
            adapter.api_url(),
            "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-sonnet-20240229-v1:0/invoke"
        );
    }

    #[test]
    fn test_api_url_default_region() {
        let adapter =
            BedrockAdapter::with_region("anthropic.claude-3-haiku-20240307-v1:0", "us-east-1");
        assert!(adapter.api_url().contains("us-east-1"));
    }

    #[test]
    fn test_model_id() {
        let adapter = BedrockAdapter::new("anthropic.claude-3-sonnet-20240229-v1:0");
        assert_eq!(
            adapter.model_id(),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );
    }

    #[test]
    fn test_region() {
        let adapter = BedrockAdapter::with_region("model", "eu-west-1");
        assert_eq!(adapter.region(), "eu-west-1");
    }

    #[test]
    fn test_convert_request_removes_unsupported_fields() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "model": "model-id",
            "messages": [{"role": "user", "content": "Hi"}],
            "n": 1,
            "frequency_penalty": 0.5,
            "presence_penalty": 0.5,
            "logprobs": true,
            "stream": true
        });
        let result = adapter.convert_request(payload);
        assert!(result.get("model").is_none());
        assert!(result.get("n").is_none());
        assert!(result.get("frequency_penalty").is_none());
        assert!(result.get("presence_penalty").is_none());
        assert!(result.get("logprobs").is_none());
        assert!(result.get("stream").is_none());
        assert_eq!(result["anthropic_version"], "bedrock-2023-05-31");
    }

    #[test]
    fn test_convert_request_sets_max_tokens() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let result = adapter.convert_request(payload);
        assert_eq!(result["max_tokens"], 4096);
    }

    #[test]
    fn test_convert_request_preserves_custom_max_tokens() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "messages": [{"role": "user", "content": "Hi"}],
            "max_tokens": 8192
        });
        let result = adapter.convert_request(payload);
        assert_eq!(result["max_tokens"], 8192);
    }

    #[test]
    fn test_convert_request_converts_max_completion_tokens() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "messages": [{"role": "user", "content": "Hi"}],
            "max_completion_tokens": 2048
        });
        let result = adapter.convert_request(payload);
        assert_eq!(result["max_tokens"], 2048);
        assert!(result.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_extra_headers() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let headers = adapter.extra_headers();
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json")
        );
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "Accept" && v == "application/json")
        );
    }

    #[test]
    fn test_build_url() {
        let url = BedrockAdapter::build_url("ap-southeast-1", "anthropic.claude-v2");
        assert_eq!(
            url,
            "https://bedrock-runtime.ap-southeast-1.amazonaws.com/model/anthropic.claude-v2/invoke"
        );
    }
}
