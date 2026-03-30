#![allow(clippy::future_not_send)]

pub(crate) mod anthropic;
pub mod cli;
mod context;
mod model;
mod openai;
mod prompts;
pub(crate) mod sse_parser;

use anthropic::AnthropicModel;
use context::Context;
use futures::Stream;
use model::Task;
use openai::OpenAIGPTModel;
use serde::Deserialize;
use thiserror::Error;

enum ConfigKind {
    Ask(AskConfig),
    Explain(ExplainConfig),
}

impl ConfigKind {
    const fn model(&self) -> &ModelKind {
        match self {
            Self::Ask(config) => &config.model,
            Self::Explain(config) => &config.model,
        }
    }
}

#[derive(Deserialize)]
struct AskConfig {
    operating_system: String,
    shell: String,
    environment: Option<Vec<String>>,
    programs: Option<Vec<String>>,
    cwd: Option<()>,
    depth: Option<u32>,
    model: ModelKind,
}

#[derive(Deserialize)]
struct ExplainConfig {
    operating_system: String,
    shell: String,
    environment: Option<Vec<String>>,
    model: ModelKind,
    cwd: Option<()>,
    depth: Option<u32>,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            operating_system: "Linux".to_string(),
            shell: "Bash".to_string(),
            environment: None,
            programs: None,
            cwd: None,
            depth: None,
            model: ModelKind::OpenAIGPT(OpenAIGPTModel::GPT4oMini),
        }
    }
}

impl Default for ExplainConfig {
    fn default() -> Self {
        Self {
            operating_system: "Linux".to_string(),
            shell: "Bash".to_string(),
            environment: None,
            cwd: None,
            depth: None,
            model: ModelKind::OpenAIGPT(OpenAIGPTModel::GPT4oMini),
        }
    }
}

#[derive(Deserialize, Clone)]
enum ModelKind {
    OpenAIGPT(OpenAIGPTModel),
    Anthropic(AnthropicModel),
    // OpenAssistant // waiting for a minimal API, go guys :D
    // Local // ?
}

#[derive(Debug, Error)]
pub(crate) enum ModelError {
    #[error("{0}")]
    Error(String),
}

impl From<Box<dyn std::error::Error + Send>> for ModelError {
    fn from(e: Box<dyn std::error::Error + Send>) -> Self {
        Self::Error(e.to_string())
    }
}

#[allow(unused)]
async fn model_request(
    model: ModelKind,
    request: String,
    context: Context,
    task: Task,
) -> Result<String, ModelError> {
    match model {
        ModelKind::OpenAIGPT(model) => model
            .send(request, context, task)
            .await
            .map_err(|err| ModelError::Error(err.to_string())),
        ModelKind::Anthropic(model) => model
            .send(request, context, task)
            .await
            .map_err(|err| ModelError::Error(err.to_string())),
    }
}

async fn model_stream_request(
    model: ModelKind,
    request: String,
    context: Context,
    task: Task,
) -> Result<impl Stream<Item = Result<String, ModelError>> + Send, ModelError> {
    match model {
        ModelKind::OpenAIGPT(model) => model
            .send_streaming(request, context, task)
            .await
            .map_err(|e| ModelError::Error(e.to_string())),
        ModelKind::Anthropic(model) => model
            .send_streaming(request, context, task)
            .await
            .map_err(|e| ModelError::Error(e.to_string())),
    }
}

fn build_context_request(request: &str, context: Context) -> String {
    String::from(context) + &format!("Here is your <task>: \n <task>{request}</task>")
}

// #[cfg(test)]
// mod tests {
//     use crate::{
//         context::Context, model::Task, model_stream_request, openai::OpenAIGPTModel::GPT35Turbo,
//         AskConfig, ConfigKind, ModelKind::OpenAIGPT,
//     };
//     use futures_util::StreamExt;
//
//     #[tokio::test]
//     async fn ssh_tunnel() {
//         let mut  response_stream = model_stream_request(OpenAIGPT(GPT35Turbo), 
//             "make an ssh tunnel between port 8080 in this machine and port 1243 in the machine with IP 192.168.0.42".to_string(), 
//             Context::from(ConfigKind::Ask(AskConfig::default())),
//             Task::GenerateCommand
//             ).await.unwrap();
//         while response_stream.next().await.is_some() {
//         }
//     }
// }
