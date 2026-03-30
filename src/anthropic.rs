use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

use thiserror::Error;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, ClientBuilder, StatusCode};

use futures_util::StreamExt;

use crate::build_context_request;
use crate::context::Context;
use crate::model::Task;
use crate::prompts;
use crate::sse_parser::ModelStream;
use crate::ModelError;

#[derive(Deserialize)]
struct Message {
    #[allow(unused)]
    role: String,
    #[allow(unused)]
    content: String,
}

#[derive(Deserialize)]
struct MessageEntry {
    #[allow(unused)]
    index: u64,
    #[allow(unused)]
    message: Message,
}

#[derive(Deserialize)]
struct Response {
    #[allow(unused)]
    id: String,
    #[serde(rename = "type")]
    #[allow(unused)]
    type_: String,
    #[allow(unused)]
    role: String,
    content: Vec<ResponseContent>,
    #[allow(unused)]
    model: String,
    #[allow(unused)]
    stop_reason: Option<String>,
    #[allow(unused)]
    stop_sequence: Option<()>,
    #[allow(unused)]
    usage: Usage,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ResponseContent {
    Text { text: String },
    #[allow(unused)]
    Block { #[serde(rename = "type")] type_: String },
}

#[derive(Deserialize)]
struct Usage {
    #[allow(unused)]
    input_tokens: u64,
    #[allow(unused)]
    output_tokens: u64,
}

#[derive(Deserialize, Clone)]
#[allow(non_camel_case_types)]
pub(crate) enum AnthropicModel {
    // Claude 4.6 (latest)
    ClaudeOpus46,
    ClaudeSonnet46,
    ClaudeHaiku45,
    // Claude 4.5
    ClaudeOpus45,
    ClaudeSonnet45,
    // Claude 4 / 4.1
    ClaudeOpus4,
    ClaudeSonnet4,
    ClaudeOpus41,
}

impl AnthropicModel {
    fn api_name(&self) -> String {
        match self {
            Self::ClaudeOpus46 => "claude-opus-4-6".to_string(),
            Self::ClaudeSonnet46 => "claude-sonnet-4-6".to_string(),
            Self::ClaudeHaiku45 => "claude-haiku-4-5".to_string(),
            Self::ClaudeOpus45 => "claude-opus-4-5".to_string(),
            Self::ClaudeSonnet45 => "claude-sonnet-4-5".to_string(),
            Self::ClaudeOpus4 => "claude-opus-4-0".to_string(),
            Self::ClaudeSonnet4 => "claude-sonnet-4-0".to_string(),
            Self::ClaudeOpus41 => "claude-opus-4-1".to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum AnthropicError {
    #[error("{0}")]
    Authentication(String),
    #[error("Client failed to initialize: {0}")]
    Client(#[from] reqwest::Error),
    #[error("Stream was interrupted: {0}")]
    Stream(String),
    #[error("Error Response: {0}")]
    ErrorResponse(String),
    #[error("An unknown error happened: {0}")]
    Unknown(String),
}

impl AnthropicModel {
    async fn send_request(
        &self,
        request: String,
        context: Context,
        task: Task,
        streaming: bool,
    ) -> Result<reqwest::Response, AnthropicError> {
        let client: Client = ClientBuilder::new()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(AnthropicError::Client)?;

        let url = "https://api.anthropic.com/v1/messages";
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            AnthropicError::Authentication(
                "You need to set ANTHROPIC_API_KEY env variable to use this model".to_string(),
            )
        })?;

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|err| {
                AnthropicError::Authentication(format!(
                    "Failed to create authentication header: {err}"
                ))
            })?,
        );
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&api_key).map_err(|err| {
                AnthropicError::Authentication(format!("Failed to create API key header: {err}"))
            })?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        );

        let context_request = build_context_request(&request, context);

        let system_content = match task {
            Task::GenerateCommand => prompts::ASK_MODEL_TASK,
            Task::Explain => prompts::EXPLAIN_MODEL_TASK,
        };

        let body = json!({
            "model": self.api_name(),
            "messages": [
                {"role": "user", "content": format!("{system_content}\n\n{context_request}")}
            ],
            "max_tokens": 1024,
            "stream": streaming,
        });

        client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AnthropicError::Unknown(format!("Unknown request error: {}", err.without_url()))
            })
    }

    pub(crate) async fn send(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<String, AnthropicError> {
        let response = self.send_request(request, context, task, false).await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AnthropicError::ErrorResponse(format!(
                "API error {}: {}",
                status, body
            )));
        }

        let response: Response = response
            .json()
            .await
            .map_err(|err| AnthropicError::Unknown(err.to_string()))?;

        let response_text = response.content
            .iter()
            .filter_map(|c| {
                if let ResponseContent::Text { text } = c {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(response_text)
    }
}

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    #[allow(unused)]
    event_type: String,
    #[allow(unused)]
    index: Option<u64>,
    #[allow(unused)]
    content_block: Option<ContentBlock>,
    #[allow(unused)]
    delta: Option<Delta>,
    #[allow(unused)]
    usage: Option<StreamUsage>,
    #[allow(unused)]
    message: Option<StreamMessage>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    #[allow(unused)]
    block_type: String,
    #[allow(unused)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct Delta {
    #[allow(unused)]
    type_: Option<String>,
    pub text: Option<String>,
}

#[derive(Deserialize)]
struct StreamUsage {
    #[allow(unused)]
    input_tokens: Option<u64>,
    #[allow(unused)]
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct StreamMessage {
    #[allow(unused)]
    id: Option<String>,
    #[allow(unused)]
    type_: Option<String>,
    #[allow(unused)]
    role: Option<String>,
    #[allow(unused)]
    content: Option<Vec<StreamContent>>,
}

#[derive(Deserialize)]
struct StreamContent {
    #[serde(rename = "type")]
    #[allow(unused)]
    content_type: String,
    #[allow(unused)]
    text: Option<String>,
}

/// Provider-specific parser for Anthropic SSE data payloads.
/// Extracts text content from streaming message events.
fn parse_anthropic_message(json_str: &str) -> Result<Vec<String>, String> {
    let event: StreamEvent =
        serde_json::from_str(json_str).map_err(|e| format!("Anthropic JSON parse error: {e}"))?;
    if let Some(delta) = event.delta {
        if let Some(text) = delta.text {
            if !text.is_empty() {
                return Ok(vec![text]);
            }
        }
    }
    Ok(vec![])
}

impl From<String> for AnthropicError {
    fn from(s: String) -> Self {
        Self::Stream(s)
    }
}

impl AnthropicModel {
    pub(crate) async fn send_streaming(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<ModelStream<ModelError>, AnthropicError> {
        let response = self.send_request(request, context, task, true).await?;
        if response.status() == StatusCode::OK {
            let byte_stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, String>> + Send>> =
                Box::pin(response.bytes_stream().map(|r| r.map_err(|e| e.to_string())));
            let err_map: fn(String) -> ModelError = |s| ModelError::Error(s);
            Ok(ModelStream::new(byte_stream, parse_anthropic_message, err_map))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(AnthropicError::ErrorResponse(format!(
                "API error: {}",
                body
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ResponseContent;

    #[test]
    fn text_content_deserialization() {
        let raw_response = r#"{"text": "Hello, World!"}"#;
        let content = serde_json::from_str::<ResponseContent>(raw_response).unwrap();
        match content {
            ResponseContent::Text { text } => assert_eq!(text, "Hello, World!"),
            _ => panic!("Expected Text variant"),
        }
    }

    /// Integration tests that make real Anthropic API calls.
    /// Requires the `live-api-tests` feature and a valid `ANTHROPIC_API_KEY` env var.
    ///
    ///   cargo test --features live-api-tests anthropic_live
    #[cfg(feature = "live-api-tests")]
    mod live {
        use super::super::AnthropicModel;
        use crate::context::Context;
        use crate::model::Task;
        use crate::{AskConfig, ConfigKind};
        use futures_util::StreamExt;

        const PROMPT: &str = "list files in current directory";

        fn default_context() -> Context {
            dotenvy::dotenv().ok();
            Context::from(ConfigKind::Ask(AskConfig::default()))
        }

        /// Helper that calls `send` and asserts the response is non-empty.
        async fn assert_send(model: AnthropicModel) {
            dotenvy::dotenv().ok();
            let name = model.api_name();
            let result = model
                .send(PROMPT.to_string(), default_context(), Task::GenerateCommand)
                .await;
            assert!(
                result.is_ok(),
                "model {name} send failed: {:?}",
                result.err()
            );
            assert!(
                !result.unwrap().is_empty(),
                "model {name} returned an empty response"
            );
        }

        /// Helper that calls `send_streaming`, drains the stream, and asserts the
        /// concatenated response is non-empty.
        async fn assert_send_streaming(model: AnthropicModel) {
            dotenvy::dotenv().ok();
            let name = model.api_name();
            let stream = model
                .send_streaming(PROMPT.to_string(), default_context(), Task::GenerateCommand)
                .await;
            assert!(
                stream.is_ok(),
                "model {name} streaming failed to start: {:?}",
                stream.err()
            );
            let response: String = stream
                .unwrap()
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .map(|chunk| chunk.expect("stream chunk error"))
                .collect();
            assert!(
                !response.is_empty(),
                "model {name} returned an empty streaming response"
            );
        }

        // --- Claude 4.6 (latest) ---

        #[tokio::test]
        async fn claude_opus_46_send() {
            assert_send(AnthropicModel::ClaudeOpus46).await;
        }

        #[tokio::test]
        async fn claude_opus_46_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeOpus46).await;
        }

        #[tokio::test]
        async fn claude_sonnet_46_send() {
            assert_send(AnthropicModel::ClaudeSonnet46).await;
        }

        #[tokio::test]
        async fn claude_sonnet_46_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeSonnet46).await;
        }

        #[tokio::test]
        async fn claude_haiku_45_send() {
            assert_send(AnthropicModel::ClaudeHaiku45).await;
        }

        #[tokio::test]
        async fn claude_haiku_45_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeHaiku45).await;
        }

        // --- Claude 4.5 ---

        #[tokio::test]
        async fn claude_opus_45_send() {
            assert_send(AnthropicModel::ClaudeOpus45).await;
        }

        #[tokio::test]
        async fn claude_opus_45_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeOpus45).await;
        }

        #[tokio::test]
        async fn claude_sonnet_45_send() {
            assert_send(AnthropicModel::ClaudeSonnet45).await;
        }

        #[tokio::test]
        async fn claude_sonnet_45_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeSonnet45).await;
        }

        // --- Claude 4 / 4.1 ---

        #[tokio::test]
        async fn claude_opus_4_send() {
            assert_send(AnthropicModel::ClaudeOpus4).await;
        }

        #[tokio::test]
        async fn claude_opus_4_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeOpus4).await;
        }

        #[tokio::test]
        async fn claude_sonnet_4_send() {
            assert_send(AnthropicModel::ClaudeSonnet4).await;
        }

        #[tokio::test]
        async fn claude_sonnet_4_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeSonnet4).await;
        }

        #[tokio::test]
        async fn claude_opus_41_send() {
            assert_send(AnthropicModel::ClaudeOpus41).await;
        }

        #[tokio::test]
        async fn claude_opus_41_send_streaming() {
            assert_send_streaming(AnthropicModel::ClaudeOpus41).await;
        }
    }
}
