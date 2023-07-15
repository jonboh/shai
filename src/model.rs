use crate::context::Context;

pub(crate) trait Model {
    fn send(&self, request: String, context: Context, task: Task)
        -> Result<String, Box<dyn std::error::Error>>;
}

pub(crate) enum Task {
    GenerateCommand,
    Explain
}
