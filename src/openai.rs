use std::time::Duration;
use serde::Deserialize;
use serde_json::json;

use reqwest::{Client, ClientBuilder};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::build_context_request;
use crate::model::{Task};
use crate::context::Context;
use crate::prompts;

#[derive(Deserialize)]
struct OpenAIGPTMessage {
    #[allow(unused)] // needed for deserialization
    pub role: OpenAIGPTRole,
    pub content: String,
}

#[derive(Deserialize)]
struct OpenAIGPTMessageEntry {
    #[allow(unused)] // needed for deserialization
    pub index: u64,
    pub message: OpenAIGPTMessage,
}

#[derive(Deserialize)]
struct OpenAIGPTResponse {
    #[allow(unused)] // needed for deserialization
    pub id: String,
    #[allow(unused)] // needed for deserialization
    pub object: String,
    #[allow(unused)] // needed for deserialization
    pub created: u64,
    #[allow(unused)] // needed for deserialization
    pub model: String,
    pub choices: Vec<OpenAIGPTMessageEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum OpenAIGPTRole {
    System,
    Assistant,
    User,
}

#[derive(Deserialize, Clone)]
pub(crate) enum OpenAIGPTModel {
    GPT35Turbo,
    GPT35Turbo16k,
}

impl OpenAIGPTModel {
   fn api_name(&self) -> String {
        match self {
            OpenAIGPTModel::GPT35Turbo => "gpt-3.5-turbo".to_string(),
            OpenAIGPTModel::GPT35Turbo16k=>"gpt-3.5-turbo-16k".to_string(),
        }
   }
}

// struct OpenAIGPTCoversation {
//     system_msg: String,
//     user_msg: String
// }

#[derive(Debug, Clone)]
pub enum OpenAIError {
    Error // TODO:
}

impl OpenAIGPTModel {
    // TODO: switch ask/explain logic further up
    pub async fn send(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<String, OpenAIError> {
        let client: Client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build().map_err(|_| OpenAIError::Error)?;

        let url = "https://api.openai.com/v1/chat/completions";
        let api_key = std::env::var("OPENAI_API_KEY")
            .expect("You need to set OPENAI_API_KEY to use this model");

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", api_key)).map_err(|_| OpenAIError::Error)?,
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
            "temperature": 0

        });

        let response = client.post(url).headers(headers).json(&body).send().await.map_err(|_| OpenAIError::Error)?;

        let response: OpenAIGPTResponse = response.json().await.map_err(|_| OpenAIError::Error)?;
        let response_text = response.choices[0].message.content.clone();
        Ok(response_text)
    }
}
