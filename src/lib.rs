#![allow(clippy::future_not_send)]

pub mod cli;
mod context;
mod model;
mod openai;
mod prompts;

use context::Context;
use futures::Stream;
use model::Task;
use openai::{OpenAIError, OpenAIGPTModel};
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
    pwd: Option<()>,
    depth: Option<u32>,
    environment: Option<Vec<String>>,
    programs: Option<Vec<String>>,
    model: ModelKind,
}

#[derive(Deserialize)]
struct ExplainConfig {
    pwd: Option<()>,
    depth: Option<u32>,
    environment: Option<Vec<String>>,
    model: ModelKind,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            pwd: None,
            depth: None,
            environment: None,
            programs: None,
            model: ModelKind::OpenAIGPT(OpenAIGPTModel::GPT35Turbo),
        }
    }
}

impl Default for ExplainConfig {
    fn default() -> Self {
        Self {
            pwd: None,
            depth: None,
            environment: None,
            model: ModelKind::OpenAIGPT(OpenAIGPTModel::GPT35Turbo),
        }
    }
}

#[derive(Deserialize, Clone)]
enum ModelKind {
    OpenAIGPT(OpenAIGPTModel),
    // OpenAssistant // waiting for a minimal API, go guys :D
    // Local // ?
}

#[derive(Debug, Error)]
enum ModelError {
    #[error("ModelError: {0}")]
    Error(#[from] Box<dyn std::error::Error>),
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
            .map_err(|err| ModelError::Error(Box::new(err))),
    }
}

async fn model_stream_request(
    model: ModelKind,
    request: String,
    context: Context,
    task: Task,
) -> Result<impl Stream<Item = Result<String, OpenAIError>>, OpenAIError> {
    match model {
        ModelKind::OpenAIGPT(model) => model.send_streaming(request, context, task).await,
    }
}

fn build_context_request(request: &str, context: Context) -> String {
    String::from(context) + &format!("Here is your <task>: \n <task>{request}</task>")
}

#[cfg(test)]
mod tests {
    use crate::{
        context::Context, model::Task, model_stream_request, openai::OpenAIGPTModel::GPT35Turbo,
        AskConfig, ConfigKind, ModelKind::OpenAIGPT,
    };
    use futures_util::StreamExt;

    #[tokio::test]
    async fn ssh_tunnel() {
        let mut  response_stream = model_stream_request(OpenAIGPT(GPT35Turbo), 
            "make an ssh tunnel between port 8080 in this machine and port 1243 in the machine with IP 192.168.0.42".to_string(), 
            Context::from(ConfigKind::Ask(AskConfig::default())),
            Task::GenerateCommand
            ).await.unwrap();
        while response_stream.next().await.is_some() {
        }
    }
}
