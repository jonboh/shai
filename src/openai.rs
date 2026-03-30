use serde::Deserialize;
use serde_json::json;
use std::fmt::Display;
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
    pub role: GPTRole,
    pub content: String,
}

#[derive(Deserialize)]
struct MessageEntry {
    #[allow(unused)]
    pub index: u64,
    pub message: Message,
}

#[derive(Deserialize)]
struct Response {
    #[allow(unused)]
    pub id: String,
    #[allow(unused)]
    pub object: String,
    #[allow(unused)]
    pub created: u64,
    #[allow(unused)]
    pub model: String,
    pub choices: Vec<MessageEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum GPTRole {
    System,
    Assistant,
    User,
}

#[derive(Deserialize, Clone)]
#[allow(non_camel_case_types)]
pub(crate) enum OpenAIGPTModel {
    // GPT-4.1 series
    GPT4_1,
    GPT4_1Mini,
    GPT4_1Nano,
    // GPT-4o series
    GPT4o,
    GPT4oMini,
    // o-series reasoning models (do not support the temperature parameter)
    O3,
    O3Mini,
    O4Mini,
    O1,
    // GPT-4 Turbo
    GPT4Turbo,
    GPT4,
}

impl OpenAIGPTModel {
    fn api_name(&self) -> String {
        match self {
            Self::GPT4_1 => "gpt-4.1".to_string(),
            Self::GPT4_1Mini => "gpt-4.1-mini".to_string(),
            Self::GPT4_1Nano => "gpt-4.1-nano".to_string(),
            Self::GPT4o => "gpt-4o".to_string(),
            Self::GPT4oMini => "gpt-4o-mini".to_string(),
            Self::O3 => "o3".to_string(),
            Self::O3Mini => "o3-mini".to_string(),
            Self::O4Mini => "o4-mini".to_string(),
            Self::O1 => "o1".to_string(),
            Self::GPT4Turbo => "gpt-4-turbo".to_string(),
            Self::GPT4 => "gpt-4".to_string(),
        }
    }

    /// Returns true for o-series reasoning models that do not accept a `temperature` parameter.
    const fn is_o_series(&self) -> bool {
        matches!(self, Self::O1 | Self::O3 | Self::O3Mini | Self::O4Mini)
    }
}

#[derive(Debug, Error)]
pub(crate) enum OpenAIError {
    #[error("{0}")]
    Authentication(String),
    #[error("Client failed to initialize: {0}")]
    Client(#[from] reqwest::Error),
    #[error("Stream was interrupted: {0}")]
    Stream(String),
    #[error("Error Response: {0}")]
    ErrorResponse(OpenAIErrorResponse),
    #[error("An unknown error happened: {0}")]
    Unknown(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIErrorResponse {
    error: OpenAIErrorResponseContent,
}

impl Display for OpenAIErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error.message)
    }
}

#[derive(Debug, Deserialize)]
struct OpenAIErrorResponseContent {
    message: String,
    #[allow(unused)]
    r#type: Option<String>,
    #[allow(unused)]
    param: Option<String>,
    #[allow(unused)]
    code: Option<String>,
}

impl OpenAIGPTModel {
    async fn send_request(
        &self,
        request: String,
        context: Context,
        task: Task,
        streaming: bool,
    ) -> Result<reqwest::Response, OpenAIError> {
        let client: Client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(OpenAIError::Client)?;

        let url = "https://api.openai.com/v1/chat/completions";
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            OpenAIError::Authentication(
                "You need to set OPENAI_API_KEY env variable to use this model".to_string(),
            )
        })?;

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|err| {
                OpenAIError::Authentication(format!(
                    "Failed to create authentication header: {err}"
                ))
            })?,
        );

        let context_request = build_context_request(&request, context);

        let system_content = match task {
            Task::GenerateCommand => prompts::ASK_MODEL_TASK,
            Task::Explain => prompts::EXPLAIN_MODEL_TASK,
        };

        let mut body = json!({
            "model": self.api_name(),
            "messages": [
                {"role": "system", "content": system_content},
                {"role": "user", "content": context_request}
            ],
            "stream": streaming,
        });
        if !self.is_o_series() {
            body["temperature"] = json!(0);
        }

        client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                OpenAIError::Unknown(format!("Unknown request error: {}", err.without_url()))
            })
    }

    pub(crate) async fn send(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<String, OpenAIError> {
        let response = self.send_request(request, context, task, false).await?;

        if response.status() != StatusCode::OK {
            let error: OpenAIErrorResponse = response
                .json()
                .await
                .map_err(|err| OpenAIError::Unknown(err.to_string()))?;
            return Err(OpenAIError::ErrorResponse(error));
        }

        let response: Response = response
            .json()
            .await
            .map_err(|err| OpenAIError::Unknown(err.to_string()))?;
        let response_text = response.choices[0].message.content.clone();
        Ok(response_text)
    }
}

#[derive(Deserialize)]
struct Choice {
    #[allow(unused)]
    index: u64,
    delta: MessageChunk,
    #[allow(unused)]
    finish_reason: Option<FinishReason>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum FinishReason {
    Stop,

    /// Will be emitted when max_tokens is reached
    Length,
}

#[derive(Deserialize)]
struct ResponseChunk {
    #[allow(unused)]
    id: String,
    #[allow(unused)]
    object: String,
    #[allow(unused)]
    created: u64,
    #[allow(unused)]
    model: String,
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase", untagged)]
enum MessageChunk {
    #[allow(unused)]
    Role {
        role: String,
    },
    Content {
        content: String,
    },
    Stop {},
}

/// Provider-specific parser for OpenAI SSE data payloads.
/// Extracts text content from streaming chat completion chunks.
fn parse_openai_message(json_str: &str) -> Result<Vec<String>, String> {
    let chunk: ResponseChunk =
        serde_json::from_str(json_str).map_err(|e| format!("OpenAI JSON parse error: {e}"))?;
    let texts = chunk
        .choices
        .iter()
        .filter_map(|c| {
            if let MessageChunk::Content { content } = &c.delta {
                if !content.is_empty() {
                    return Some(content.clone());
                }
            }
            None
        })
        .collect();
    Ok(texts)
}

impl From<String> for OpenAIError {
    fn from(s: String) -> Self {
        Self::Stream(s)
    }
}

impl OpenAIGPTModel {
    pub(crate) async fn send_streaming(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<ModelStream<ModelError>, OpenAIError> {
        let response = self.send_request(request, context, task, true).await?;
        if response.status() == StatusCode::OK {
            let byte_stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, String>> + Send>> =
                Box::pin(response.bytes_stream().map(|r| r.map_err(|e| e.to_string())));
            let err_map: fn(String) -> ModelError = |s| ModelError::Error(s);
            Ok(ModelStream::new(byte_stream, parse_openai_message, err_map))
        } else {
            let error: OpenAIErrorResponse = response
                .json()
                .await
                .map_err(|err| OpenAIError::Unknown(err.to_string()))?;
            Err(OpenAIError::ErrorResponse(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, MessageChunk, ResponseChunk};
    #[cfg(feature = "live-api-tests")]
    use super::OpenAIGPTModel;

    #[test]
    fn empty_delta_deserialization() {
        let raw_response = r#"{}"#;
        serde_json::from_str::<MessageChunk>(raw_response).unwrap();
    }

    #[test]
    fn choice_deserialization() {
        let raw_response = r#"{"index":0,"delta":{},"finish_reason":"stop"}"#;
        serde_json::from_str::<Choice>(raw_response).unwrap();
    }

    #[test]
    fn stop_message() {
        let raw_response = r#"{"id":"chatcmpl","object":"chat.completion.chunk","created":9999,"model":"gpt-3.5-turbo-0613","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        serde_json::from_str::<ResponseChunk>(raw_response).unwrap();
    }

    /// Integration tests that make real OpenAI API calls.
    /// Requires the `live-api-tests` feature and a valid `OPENAI_API_KEY` env var.
    ///
    ///   cargo test --features live-api-tests openai_live
    #[cfg(feature = "live-api-tests")]
    mod live {
        use super::OpenAIGPTModel;
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
        async fn assert_send(model: OpenAIGPTModel) {
            dotenvy::dotenv().ok();
            let result = model
                .send(PROMPT.to_string(), default_context(), Task::GenerateCommand)
                .await;
            assert!(
                result.is_ok(),
                "model {} send failed: {:?}",
                model.api_name(),
                result.err()
            );
            assert!(
                !result.unwrap().is_empty(),
                "model {} returned an empty response",
                model.api_name()
            );
        }

        /// Helper that calls `send_streaming`, drains the stream, and asserts the
        /// concatenated response is non-empty.
        async fn assert_send_streaming(model: OpenAIGPTModel) {
            dotenvy::dotenv().ok();
            let stream = model
                .send_streaming(PROMPT.to_string(), default_context(), Task::GenerateCommand)
                .await;
            assert!(
                stream.is_ok(),
                "model {} streaming failed to start: {:?}",
                model.api_name(),
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
                "model {} returned an empty streaming response",
                model.api_name()
            );
        }

        // --- GPT-4.1 series ---

        #[tokio::test]
        async fn gpt4_1_send() {
            assert_send(OpenAIGPTModel::GPT4_1).await;
        }

        #[tokio::test]
        async fn gpt4_1_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4_1).await;
        }

        #[tokio::test]
        async fn gpt4_1_mini_send() {
            assert_send(OpenAIGPTModel::GPT4_1Mini).await;
        }

        #[tokio::test]
        async fn gpt4_1_mini_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4_1Mini).await;
        }

        #[tokio::test]
        async fn gpt4_1_nano_send() {
            assert_send(OpenAIGPTModel::GPT4_1Nano).await;
        }

        #[tokio::test]
        async fn gpt4_1_nano_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4_1Nano).await;
        }

        // --- GPT-4o series ---

        #[tokio::test]
        async fn gpt4o_send() {
            assert_send(OpenAIGPTModel::GPT4o).await;
        }

        #[tokio::test]
        async fn gpt4o_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4o).await;
        }

        #[tokio::test]
        async fn gpt4o_mini_send() {
            assert_send(OpenAIGPTModel::GPT4oMini).await;
        }

        #[tokio::test]
        async fn gpt4o_mini_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4oMini).await;
        }

        // --- o-series reasoning models ---

        #[tokio::test]
        async fn o1_send() {
            assert_send(OpenAIGPTModel::O1).await;
        }

        #[tokio::test]
        async fn o1_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::O1).await;
        }

        #[tokio::test]
        async fn o3_send() {
            assert_send(OpenAIGPTModel::O3).await;
        }

        #[tokio::test]
        async fn o3_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::O3).await;
        }

        #[tokio::test]
        async fn o3_mini_send() {
            assert_send(OpenAIGPTModel::O3Mini).await;
        }

        #[tokio::test]
        async fn o3_mini_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::O3Mini).await;
        }

        #[tokio::test]
        async fn o4_mini_send() {
            assert_send(OpenAIGPTModel::O4Mini).await;
        }

        #[tokio::test]
        async fn o4_mini_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::O4Mini).await;
        }

        // --- GPT-4 Turbo / legacy ---

        #[tokio::test]
        async fn gpt4_turbo_send() {
            assert_send(OpenAIGPTModel::GPT4Turbo).await;
        }

        #[tokio::test]
        async fn gpt4_turbo_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4Turbo).await;
        }

        #[tokio::test]
        async fn gpt4_send() {
            assert_send(OpenAIGPTModel::GPT4).await;
        }

        #[tokio::test]
        async fn gpt4_send_streaming() {
            assert_send_streaming(OpenAIGPTModel::GPT4).await;
        }
    }
}
