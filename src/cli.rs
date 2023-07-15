use std::fs;

use clap::Parser;

use crate::model::Task;
use crate::ui::ui_ask;
use crate::{Config, process_config};

#[derive(Parser)] // requires `derive` feature
#[command(name = "cli-ai-assistant")]
enum CliAssistant {
    Ask(AskArgs),
    Explain(ExplainArgs),
    Buffer(BufferArgs),
}

#[derive(clap::ValueEnum, Clone)]
enum ArgModelKind {
    OpenAIGPT35turbo,
    OpenAIGPT35turbo16k,
}

#[derive(clap::Args)]
#[command(author, version, about, long_about = None)]
struct AskArgs {
    #[arg(long)]
    pwd: bool,

    #[arg(long, default_value=None)]
    depth: Option<u32>,

    #[arg(long, default_value = None)]
    environment: Option<Vec<String>>,

    #[arg(long, default_value = None)]
    programs: Option<Vec<String>>,

    #[arg(long, value_enum)]
    model: ArgModelKind,

    #[arg(long)]
    edit_file: Option<std::path::PathBuf>,
}
#[derive(clap::Args)]
#[command(author, version, about, long_about = None)]
struct ExplainArgs {
    #[arg(long)]
    pwd: bool,

    #[arg(long, default_value=None)]
    depth: Option<u32>,

    #[arg(long, default_value = None)]
    environment: Option<Vec<String>>,

    #[arg(long, value_enum)]
    model: ArgModelKind,

    #[arg(long)]
    write_stdout: bool,

    #[arg(long)]
    edit_file: Option<std::path::PathBuf>,

}
#[derive(clap::Args)]
#[command(author, version, about, long_about = None)]
struct BufferArgs {}


fn ask(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
    let cli_text = args.edit_file.as_ref().and_then(|file| fs::read_to_string(file).ok()).unwrap_or_default();
    let user_prompt = ui_ask(&cli_text)?;

    // let (context, model) = process_config(&config);
    let (context, model) = process_config(&Config::default());

    let response = model.send(user_prompt, context, Task::GenerateCommand).unwrap();

    // TODO: check response is just commands
    // if not just commands print explanation in and allow user to scrape commands
    // or give another opportunity for input

    if let Some(file) = args.edit_file {
        fs::write(file, response)?;
    } else {
        println!("$ {response}");
    }
    Ok(())
}

fn explain(args: ExplainArgs) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Specialize explain ui
    let cli_text = args.edit_file.and_then(|file| fs::read_to_string(file).ok()).unwrap_or_default();
    let user_prompt = ui_ask(&cli_text)?;

    // let (context, model) = process_config(&config);
    let (context, model) = process_config(&Config::default());

    let response = model.send(user_prompt, context, Task::Explain).unwrap();

    // TODO: repeat and update UI

    println!("{response}");
    Ok(())
}

fn buffer(_args: BufferArgs) -> Result<(), Box<dyn std::error::Error>>{
    todo!()
}

pub fn run() -> Result<(), Box<dyn std::error::Error>>{
    let args = CliAssistant::parse();
    match args {
        CliAssistant::Ask(ask_args) => ask(ask_args)?,
        CliAssistant::Explain(explain_args) => explain(explain_args)?,
        CliAssistant::Buffer(buffer_args) => buffer(buffer_args)?,
    }
    Ok(())
}
