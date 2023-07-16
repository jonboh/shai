mod openai;
mod context;
mod model;
mod prompts;
pub mod cli;

use serde::Deserialize;
use context::Context;
use openai::OpenAIGPTModel;
use model::Task;
use thiserror::Error;

enum ConfigKind {
    Ask(AskConfig),
    Explain(ExplainConfig)
}

impl ConfigKind {
    fn model(&self) -> &ModelKind {
        match self {
            ConfigKind::Ask(config) => &config.model,
            ConfigKind::Explain(config) => &config.model,
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

#[derive(Debug, Clone, Error)]
enum ModelError {
    #[error("ModelError")]
    Error // TODO:
}

async fn model_request(
    model: ModelKind,
    request: String,
    context: Context,
    task: Task,
) -> Result<String, ModelError> {
    use ModelKind::*;
    match model {
        OpenAIGPT(model) => model.send(request, context, task).await.map_err(|_| ModelError::Error),
    }
}

fn build_context_request(request: String, context: Context) -> String {
    String::from(context) + &format!("Here is your <task>: \n <task>{request}</task>")
}
