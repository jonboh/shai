// pub(crate) trait Model {
//     fn send(&self, request: String, context: Context, task: Task)
//         -> Result<String, Box<dyn std::error::Error>>;
// }

pub enum Task {
    GenerateCommand,
    Explain,
}
