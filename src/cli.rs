use std::fs;
use std::io::{self, StdoutLock};

use clap::Parser;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, Borders, Paragraph, Wrap};
use tui::Terminal;
use tui_textarea::{Input, Key, TextArea};

use crate::context::Context;
use crate::model::{Model, Task};
use crate::openai::OpenAIGPTModel;
use crate::{AskConfig, ConfigKind, ExplainConfig, ModelKind};

#[derive(Parser)] // requires `derive` feature
#[command(name = "cli-ai-assistant")]
enum CliAssistant {
    Ask(AskArgs),
    Explain(ExplainArgs),
}

#[derive(clap::ValueEnum, Clone)]
enum ArgModelKind {
    OpenAIGPT35turbo,
    OpenAIGPT35turbo16k,
}

impl From<ArgModelKind> for ModelKind {
    fn from(value: ArgModelKind) -> Self {
        use ArgModelKind::*;
        match value {
            OpenAIGPT35turbo => ModelKind::OpenAIGPT(OpenAIGPTModel::GPT35Turbo),
            OpenAIGPT35turbo16k => ModelKind::OpenAIGPT(OpenAIGPTModel::GPT35Turbo16k),
        }
    }
}

#[derive(clap::Args, Clone)]
#[command(author, version, about, long_about = None)]
pub struct AskArgs {
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

#[derive(clap::Args, Clone)]
#[command(author, version, about, long_about = None)]
pub struct ExplainArgs {
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

impl From<AskArgs> for AskConfig {
    fn from(value: AskArgs) -> Self {
        let pwd = if value.pwd { Some(()) } else { None };
        let model = value.model.into();
        AskConfig {
            pwd,
            depth: value.depth,
            environment: value.environment,
            programs: value.programs,
            model,
        }
    }
}

impl From<ExplainArgs> for ExplainConfig {
    fn from(value: ExplainArgs) -> Self {
        let pwd = if value.pwd { Some(()) } else { None };
        let model = value.model.into();
        ExplainConfig {
            pwd,
            depth: value.depth,
            environment: value.environment,
            model,
        }
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let app_args = CliAssistant::parse();
    match app_args {
        CliAssistant::Ask(args) => {
            let mut ui = AskUI::new(args)?;
            ui.run()?
        }
        CliAssistant::Explain(args) => {
            let mut ui = ExplainUI::new(args)?;
            ui.run()?
        }
    }
    Ok(())
}

pub struct AskUI<'t> {
    args: AskArgs,
    // state: <WaitingInput|WritingResponse>
    term: Terminal<CrosstermBackend<StdoutLock<'t>>>,
    layout: Layout,
    textarea: TextArea<'t>,
}

impl<'t> AskUI<'t> {
    pub fn new(args: AskArgs) -> Result<AskUI<'t>, Box<dyn std::error::Error>> {
        let mut stdout = io::stdout().lock();

        enable_raw_mode()?;
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;

        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("What should shai's command do?"),
        );
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5)].as_ref());
        Ok(AskUI {
            args,
            layout,
            term,
            textarea,
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let cli_text = self
            .args
            .edit_file
            .as_ref()
            .and_then(|file| fs::read_to_string(file).ok())
            .unwrap_or_default();
        self.textarea.insert_str(&cli_text);
        self.mainloop()?;

        disable_raw_mode()?;
        crossterm::execute!(
            self.term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.term.show_cursor()?;
        Ok(())
    }

    fn mainloop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            self.term.draw(|f| {
                let chunks = self.layout.split(f.size());
                let widget = self.textarea.widget();
                f.render_widget(widget, chunks[0]);
            })?;

            match crossterm::event::read()?.into() {
                Input { key: Key::Esc, .. } => break,
                // TODO: add \n on crtl+Enter
                Input {
                    key: Key::Enter, ..
                } => {
                    self.send_prompt()?;
                    break;
                }
                input => {
                    // TextArea::input returns if the input modified its text
                    self.textarea.input(input);
                }
            }
        }
        Ok(())
    }

    fn send_prompt(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config = AskConfig::from(self.args.clone());
        let model = config.model.clone();
        let context = Context::from(ConfigKind::Ask(config));

        let user_prompt = self.textarea.lines().join("\n");

        let response = model
            .send(user_prompt, context, Task::GenerateCommand)
            .unwrap();

        // TODO: check response is just commands
        // if not just commands print explanation in and allow user to scrape commands
        // or give another opportunity for input

        if let Some(file) = &self.args.edit_file {
            fs::write(file, response)?;
        } else {
            println!("$ {response}");
        }
        Ok(())
    }
}

pub struct ExplainUI<'t> {
    args: ExplainArgs,
    // state: <WaitingInput|WritingResponse>
    term: Terminal<CrosstermBackend<StdoutLock<'t>>>,
    layout: Layout,
    textarea: TextArea<'t>,
    explanation_paragraph: Paragraph<'t>,
}

impl<'t> ExplainUI<'t> {
    pub fn new(args: ExplainArgs) -> Result<Self, Box<dyn std::error::Error>> {
        let mut stdout = io::stdout().lock();

        enable_raw_mode()?;
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;

        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("What command should shai explain?"),
        );
        textarea.set_cursor_line_style(Style::default());
        let explanation_paragraph = Self::create_explanation_paragraph("".to_string());

        // activate(&mut textarea);
        // inactivate(&mut textarea.1);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(20)].as_ref());
        Ok(ExplainUI {
            args,
            layout,
            term,
            textarea,
            explanation_paragraph,
        })
    }

    fn create_explanation_paragraph(text: String) -> Paragraph<'t> {
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title("Shai:"))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let cli_text = self
            .args
            .edit_file
            .as_ref()
            .and_then(|file| fs::read_to_string(file).ok())
            .unwrap_or_default();
        self.textarea.insert_str(&cli_text);
        self.mainloop()?;

        disable_raw_mode()?;
        crossterm::execute!(
            self.term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.term.show_cursor()?;
        Ok(())
    }

    fn mainloop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            self.term.draw(|f| {
                let chunks = self.layout.split(f.size());
                let widget = self.textarea.widget();
                f.render_widget(widget, chunks[0]);
                f.render_widget(self.explanation_paragraph.clone(), chunks[1]);
            })?;

            match crossterm::event::read()?.into() {
                Input { key: Key::Esc, .. } => break,
                // TODO: add \n on crtl+Enter
                Input {
                    key: Key::Enter, ..
                } => {
                    let response = self.send_prompt()?;
                    self.show_response(&response)?;
                }
                input => {
                    // TextArea::input returns if the input modified its text
                    self.textarea.input(input);
                }
            }
        }
        Ok(())
    }

    fn send_prompt(&self) -> Result<String, Box<dyn std::error::Error>> {
        let config = ExplainConfig::from(self.args.clone());
        let model = config.model.clone();
        let context = Context::from(ConfigKind::Explain(config));

        let user_prompt = self.textarea.lines().join("\n");

        Ok(model.send(user_prompt, context, Task::Explain).unwrap())
    }

    fn show_response(&mut self, response: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.explanation_paragraph = Self::create_explanation_paragraph(response.to_string());
        if self.args.write_stdout {
            println!("{response}");
        }
        Ok(())
    }
}
