use std::fmt::Display;
use std::fs;
use std::io::{self, StdoutLock};
use std::time::Duration;

use clap::Parser;

use crossterm::event::{poll, DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::Style;
use tui::widgets::{Block, Borders, Paragraph, Wrap};
use tui::Terminal;
use tui_textarea::{Input, Key, TextArea};

use crate::context::Context;
use crate::model::Task;
use crate::openai::OpenAIGPTModel;
use crate::{model_request, AskConfig, ConfigKind, ExplainConfig, ModelKind};

#[derive(Parser, Clone)]
#[command(name = "shai")]
pub enum ShaiArgs {
    Ask(AskArgs),
    Explain(ExplainArgs),
}

impl ShaiArgs {
    fn edit_file(&self) -> &Option<std::path::PathBuf> {
        match self {
            ShaiArgs::Ask(args) => &args.edit_file,
            ShaiArgs::Explain(args) => &args.edit_file,
        }
    }
    fn write_stdout(&self) -> bool {
        match self {
            ShaiArgs::Ask(args) => args.write_stdout,
            ShaiArgs::Explain(args) => args.write_stdout,
        }
    }
}

impl From<ShaiArgs> for ConfigKind {
    fn from(value: ShaiArgs) -> Self {
        match value {
            ShaiArgs::Ask(args) => ConfigKind::Ask(AskConfig::from(args)),
            ShaiArgs::Explain(args) => ConfigKind::Explain(ExplainConfig::from(args)),
        }
    }
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
    write_stdout: bool,

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

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = ShaiArgs::parse();
    let mut ui = ShaiUI::new(args)?;
    ui.run().await?;
    Ok(())
}

enum WriteBuffer {
    Yes,
    No
}

enum MainLoopAction {
    Exit,
    AcceptStdinInput,
    SendRequest,
}

enum RequestState {
    AcceptMore,
    Cancel,
    Exit,
    Finished,
}

#[derive(Copy, Clone)]
enum ShaiProgress {
    Waiting,
    S0,
    S1,
    S2,
    S3,
}

impl ShaiProgress {
    fn next_state(self) -> ShaiProgress {
        match self {
            ShaiProgress::Waiting => ShaiProgress::S0,
            ShaiProgress::S0 => ShaiProgress::S1,
            ShaiProgress::S1 => ShaiProgress::S2,
            ShaiProgress::S2 => ShaiProgress::S3,
            ShaiProgress::S3 => ShaiProgress::S0,
        }
    }
}

impl Display for ShaiProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaiProgress::Waiting => write!(f, ""),
            ShaiProgress::S0 => write!(f, "-"),
            ShaiProgress::S1 => write!(f, "\\"),
            ShaiProgress::S2 => write!(f, "|"),
            ShaiProgress::S3 => write!(f, "/"),
        }
    }
}

pub struct ShaiUI<'t> {
    args: ShaiArgs,
    term: Terminal<CrosstermBackend<StdoutLock<'t>>>,
    layout: Layout,
    textarea: TextArea<'t>,
    explanation_paragraph: (String, Paragraph<'t>),
}

impl<'t> ShaiUI<'t> {
    pub fn new(args: ShaiArgs) -> Result<Self, Box<dyn std::error::Error>> {
        let mut stdout = io::stdout().lock();

        enable_raw_mode()?;
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;

        let title = match args {
            ShaiArgs::Ask(_) => "What shold shai's command do?",
            ShaiArgs::Explain(_) => "What command should shai explain?",
        };
        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        );
        textarea.set_cursor_line_style(Style::default());
        let text = "";
        let explanation_paragraph = (
            text.to_string(),
            Self::create_explanation_paragraph(text.to_string(), ShaiProgress::Waiting),
        );

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(20)].as_ref());
        Ok(ShaiUI {
            args,
            layout,
            term,
            textarea,
            explanation_paragraph,
        })
    }

    fn create_explanation_paragraph(text: String, thinking: ShaiProgress) -> Paragraph<'t> {
        let title = format!("Shai {thinking}");
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true })
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let cli_text = self
            .args
            .edit_file()
            .as_ref()
            .and_then(|file| fs::read_to_string(file).ok())
            .unwrap_or_default();
        self.textarea.insert_str(&cli_text);
        let response = self.mainloop().await?;

        if let ShaiArgs::Ask(_) = self.args {
            if let Some(file) = &self.args.edit_file() {
                if let WriteBuffer::Yes = response {
                    fs::write(file, &self.explanation_paragraph.0)?
                }
            }
        }
        disable_raw_mode()?;
        crossterm::execute!(
            self.term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.term.show_cursor()?;
        Ok(())
    }

    async fn mainloop(&mut self) -> Result<WriteBuffer, Box<dyn std::error::Error>> {
        let mut state = MainLoopAction::AcceptStdinInput;
        loop {
            self.term.draw(|f| {
                let chunks = self.layout.split(f.size());
                let widget = self.textarea.widget();
                f.render_widget(widget, chunks[0]);
                f.render_widget(self.explanation_paragraph.1.clone(), chunks[1]);
            })?;

            match state {
                MainLoopAction::AcceptStdinInput => {
                    match crossterm::event::read()?.into() {
                        Input { key: Key::Esc, .. } => return Ok(WriteBuffer::No),
                        Input { key: Key::Char('a'), ctrl:true, ..} => return Ok(WriteBuffer::Yes),
                        Input {
                            key: Key::Enter, ..
                        } => state = MainLoopAction::SendRequest,

                        input => {
                            self.textarea.input(input);
                            // action = MainLoopAction::AcceptMore // redundant
                        }
                    }
                }
                MainLoopAction::SendRequest => {
                    state = match self.send_request().await? {
                        RequestState::Exit => MainLoopAction::Exit,
                        _ => MainLoopAction::AcceptStdinInput,
                    };
                }
                MainLoopAction::Exit => return Ok(WriteBuffer::No), // shouldn't here here
                                                                       // anyway
            }
        }
    }

    async fn send_request(&mut self) -> Result<RequestState, Box<dyn std::error::Error>> {
        let config = ConfigKind::from(self.args.clone());
        let model = config.model().clone();
        let task = match config {
            ConfigKind::Ask(_) => Task::GenerateCommand,
            ConfigKind::Explain(_) => Task::Explain,
        };
        let context = Context::from(config);
        let user_prompt = self.textarea.lines().join("\n");
        let request_task = tokio::spawn(model_request(
            model.clone(),
            user_prompt,
            context.clone(),
            task,
        ));
        let mut state = RequestState::AcceptMore;
        let mut fidget = ShaiProgress::Waiting;
        loop {
            self.term.draw(|f| {
                let chunks = self.layout.split(f.size());
                let widget = self.textarea.widget();
                f.render_widget(widget, chunks[0]);
                f.render_widget(self.explanation_paragraph.1.clone(), chunks[1]);
            })?;

            match state {
                RequestState::AcceptMore => {
                    if poll(Duration::from_millis(100))? {
                        match crossterm::event::read()?.into() {
                            Input { key: Key::Esc, .. } => state = RequestState::Exit,
                            Input {
                                key: Key::Char('c'),
                                ctrl: true,
                                ..
                            } => state = RequestState::Cancel,
                            _ => (),
                        }
                    }
                    if request_task.is_finished() {
                        state = RequestState::Finished;
                    }
                }
                RequestState::Cancel => break,
                RequestState::Finished => break,
                RequestState::Exit => break,
            }
            fidget = fidget.next_state();
            self.explanation_paragraph.1 =
                Self::create_explanation_paragraph(self.explanation_paragraph.0.clone(), fidget);
        }
        match state {
            RequestState::Finished => {
                let response = request_task.await??;
                self.show_response(&response)?;
            }
            RequestState::Cancel => {
                self.explanation_paragraph.1 = Self::create_explanation_paragraph(
                    self.explanation_paragraph.0.clone(),
                    ShaiProgress::Waiting,
                );
            }
            _ => (),
        }
        Ok(state)
    }

    fn show_response(&mut self, response: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.explanation_paragraph = (
            response.to_string(),
            Self::create_explanation_paragraph(response.to_string(), ShaiProgress::Waiting),
        );
        if self.args.write_stdout() {
            println!("{response}");
        }
        Ok(())
    }
}
