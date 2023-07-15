mod openai;
mod context;
mod model;
mod ui;
mod prompts;
pub mod cli;

use serde::Deserialize;
use context::Context;
use openai::OpenAIGPTModel;
use model::{Model, Task};


#[derive(Deserialize)]
struct Config {
    pwd: Option<()>,
    depth: Option<u32>,
    environment: Option<Vec<String>>,
    programs: Option<Vec<String>>,
    model: ModelKind,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pwd: None,
            depth: None,
            environment: None,
            programs: None,
            model: ModelKind::OpenAIGPT(OpenAIGPTModel {}),
        }
    }
}

fn process_config(config: &Config) -> (Context, Box<dyn Model>) {
    // TODO: this could be a method
    (Context::new(config), Box::new(config.model.clone()))
}

#[derive(Deserialize, Clone)]
enum ModelKind {
    OpenAIGPT(OpenAIGPTModel),
    // OpenAssistant // waiting for a minimal API, go guys :D
    // Local // ?
}

impl model::Model for ModelKind {
    fn send(
        &self,
        request: String,
        context: Context,
        task: Task,
    ) -> Result<String, Box<dyn std::error::Error>> {
        use ModelKind::*;
        match self {
            OpenAIGPT(model) => model.send(request, context, task),
        }
    }
}

fn build_context_request(request: String, context: Context) -> String {
    String::from(context) + &format!("Here is your <task>: \n <task>{request}</task>")
}
