use futures::Stream;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

use thiserror::Error;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, ClientBuilder};

use eventsource_stream::Eventsource;
use futures_util::StreamExt;

use crate::build_context_request;
use crate::context::Context;
use crate::model::Task;
use crate::prompts;

#[derive(Deserialize)]
struct Message {
    #[allow(unused)] // needed for deserialization
    pub role: GPTRole,
    pub content: String,
}

#[derive(Deserialize)]
struct MessageEntry {
    #[allow(unused)] // needed for deserialization
    pub index: u64,
    pub message: Message,
}

#[derive(Deserialize)]
struct Response {
    #[allow(unused)] // needed for deserialization
    pub id: String,
    #[allow(unused)] // needed for deserialization
    pub object: String,
    #[allow(unused)] // needed for deserialization
    pub created: u64,
    #[allow(unused)] // needed for deserialization
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
    GPT35Turbo,
    GPT35Turbo_16k,
    GPT4,
    GPT4_32k
}

impl OpenAIGPTModel {
    fn api_name(&self) -> String {
        match self {
            OpenAIGPTModel::GPT35Turbo => "gpt-3.5-turbo".to_string(),
            OpenAIGPTModel::GPT35Turbo_16k => "gpt-3.5-turbo-16k".to_string(),
            OpenAIGPTModel::GPT4 => "gpt-4".to_string(),
            OpenAIGPTModel::GPT4_32k => "gpt-4-32k".to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub enum OpenAIError {
    #[error("{0}")]
    Authentication(String),
    #[error("Client failed to initialize: {0}")]
    Client(#[from]  reqwest::Error),
    #[error("Stream was interrupted: {0}")]
    Stream(String),
    #[error("Failed to deserialize OpenAI model response: {0}")]
    Deserialization(String),
    #[error("An unknown error happened: {0}")]
    Unknown(String),

    // TODO: handle errors like billing limit reached
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
            HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|err| {
                OpenAIError::Authentication(
                    format!("Failed to create authentication header: {err}"),
                )
            })?,
        );

        let context_request = build_context_request(request, context);

        let system_content = match task {
            Task::GenerateCommand => prompts::ASK_MODEL_TASK,
            Task::Explain => prompts::EXPLAIN_MODEL_TASK,
        };
        let body = json!({
            "model": self.api_name(),
            "messages": [
                {"role": "system", "content": system_content},
                {"role": "user", "content": context_request}
            ],
            "temperature": 0,
            "stream": streaming,
        });

        client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|err| OpenAIError::Unknown(format!("Unknown request error: {}", err.without_url())))
    }

    pub async fn send(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<String, OpenAIError> {
        let response = self.send_request(request, context, task, false).await?;

        let response: Response = response.json().await.map_err(|err| OpenAIError::Unknown(err.to_string()))?;
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

impl OpenAIGPTModel {
    pub async fn send_streaming(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<impl Stream<Item = Result<String, OpenAIError>>, OpenAIError> {
        let response = self
            .send_request(request, context, task, true)
            .await?
            .bytes_stream()
            .eventsource();
        let message_stream = response.map(|response| {
            let data = response.map_err(|err| OpenAIError::Stream(err.to_string()))?.data;
            if data == "[DONE]" {
                Ok("".to_string())
            } else {
                Ok(match &serde_json::from_str::<ResponseChunk>(&data)
                    .map_err(|err| OpenAIError::Deserialization(err.to_string()))?
                    .choices[0]
                    .delta
                {
                    MessageChunk::Content { content: msg } => msg.to_string(),
                    _ => "".to_string(),
                })
            }
        });
        Ok(message_stream)
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, MessageChunk, ResponseChunk};

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
}
