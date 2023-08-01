use std::fmt::Display;
use std::fs;
use std::io::{self, StdoutLock};
use std::time::Duration;

use clap::Parser;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::{Stream, StreamExt};
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Constraint, Direction};
use tui::style::Style;
use tui::widgets::{Block, Borders, Paragraph, Wrap};
use tui::Terminal;
use tui_textarea::{Input, Key, TextArea};

use crate::context::Context;
use crate::model::Task;
use crate::openai::OpenAIGPTModel;
use crate::{model_stream_request, AskConfig, ConfigKind, ExplainConfig, ModelError, ModelKind};

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
    No,
}

enum RequestState {
    WaitRequest,
    Cancel,
    Exit,
    Streaming,
}

#[derive(Copy, Clone)]
enum ShaiProgress {
    Waiting,
    S0,
    S1,
    S2,
    S3,
}

#[derive(Clone, Copy)]
enum RequestType {
    // stdin -> main_response
    Normal,
    // main_response(command) -> auxiliary_response
    Auxiliary,
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

struct ModelWindow<'t> {
    pub response: String,
    pub paragraph: Paragraph<'t>,
    fidget: ShaiProgress,
}

impl ModelWindow<'_> {
    fn update(&mut self, new: String, fidget: ShaiProgress) {
        self.response = new.clone();
        self.paragraph = create_explanation_paragraph(new, fidget);
    }

    fn spin_fidget(&mut self) {
        self.fidget = self.fidget.next_state();
        self.update(self.response.clone(), self.fidget)
    }
}
fn create_explanation_paragraph<'t>(text: String, thinking: ShaiProgress) -> Paragraph<'t> {
    let title = format!("Shai {thinking}");
    Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(title))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
}

pub struct ShaiUI<'t> {
    args: ShaiArgs,
    term: Terminal<CrosstermBackend<StdoutLock<'t>>>,
    layout: Layout,
    textarea: TextArea<'t>,
    main_response: ModelWindow<'t>,
    auxiliary_response: ModelWindow<'t>,
}

/// Checks whether the response contains ``` and other indicators that
/// the model ignored the instruction to just return the commands
#[allow(unused)]
fn is_just_command(response: &str) -> bool {
    true // TODO: implement
}

/// Tries to remove all text that is not just a command
#[allow(unused)]
fn extract_commands(response: &str) -> String {
    response.to_string()
}

enum Layout {
    InputResponse,
    InputResponseExplanation,
}

impl Layout {
    fn create(&self) -> tui::layout::Layout {
        match self {
            Layout::InputResponse => tui::layout::Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(20)].as_ref()),
            Layout::InputResponseExplanation => tui::layout::Layout::default()
                .direction(Direction::Vertical)
                // .constraints([Constraint::Length(3), Constraint::Min(20), Constraint::Min(20)].as_ref())
                .constraints([
                    Constraint::Length(3),
                    Constraint::Percentage(10),
                    Constraint::Percentage(80),
                ]),
        }
    }
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
        textarea.set_block(Block::default().borders(Borders::ALL).title(title));
        textarea.set_cursor_line_style(Style::default());
        let main_response = ModelWindow {
            response: String::new(),
            paragraph: create_explanation_paragraph(String::new(), ShaiProgress::Waiting),
            fidget: ShaiProgress::Waiting,
        };
        let auxiliary_response = ModelWindow {
            response: String::new(),
            paragraph: create_explanation_paragraph(String::new(), ShaiProgress::Waiting),
            fidget: ShaiProgress::Waiting,
        };

        let layout = Layout::InputResponse;
        Ok(ShaiUI {
            args,
            term,
            layout,
            textarea,
            main_response,
            auxiliary_response,
        })
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
                    fs::write(file, &self.main_response.response)?
                }
            }
        }

        // restore terminal mode
        disable_raw_mode()?;
        crossterm::execute!(
            self.term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.term.show_cursor()?;

        if self.args.write_stdout() {
            let response = &self.main_response.response;
            println!("{response}");
        }
        Ok(())
    }

    async fn mainloop(&mut self) -> Result<WriteBuffer, Box<dyn std::error::Error>> {
        loop {
            self.draw()?;

            match crossterm::event::read()?.into() {
                Input { key: Key::Esc, .. } => return Ok(WriteBuffer::No),
                Input {
                    key: Key::Char('a'),
                    ctrl: true,
                    ..
                } => return Ok(WriteBuffer::Yes),
                Input {
                    key: Key::Enter, ..
                } => {
                    if let RequestState::Exit = self.send_request(RequestType::Normal).await? {
                        return Ok(WriteBuffer::No);
                    }
                }
                Input {
                    key: Key::Char('e'),
                    ctrl: true,
                    ..
                } => {
                    if let ShaiArgs::Ask(_) = self.args {
                        if !self.main_response.response.is_empty() {
                            // there's already something to explain
                            self.layout = Layout::InputResponseExplanation;
                            self.send_request(RequestType::Auxiliary).await?;
                        }
                    }
                }

                input => {
                    self.textarea.input(input);
                }
            }
        }
    }

    fn draw(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: draw auxiliary if active
        self.term.draw(|f| match &self.layout {
            Layout::InputResponse => {
                let layout = self.layout.create();
                let chunks = layout.split(f.size());
                f.render_widget(self.textarea.widget(), chunks[0]);
                f.render_widget(self.main_response.paragraph.clone(), chunks[1]);
            }
            Layout::InputResponseExplanation => {
                let layout = self.layout.create();
                let chunks = layout.split(f.size());
                f.render_widget(self.textarea.widget(), chunks[0]);
                f.render_widget(self.main_response.paragraph.clone(), chunks[1]);
                f.render_widget(self.auxiliary_response.paragraph.clone(), chunks[2]);
            }
        })?;
        Ok(())
    }

    // Source = {stdin, main_response}
    // Destination = {main_response, auxiliary_response}
    // async fn send_request(&mut self, rou) -> Result<RequestState, Box<dyn std::error::Error>> {
    async fn send_request(
        &mut self,
        request_type: RequestType,
    ) -> Result<RequestState, Box<dyn std::error::Error>> {
        let config = ConfigKind::from(self.args.clone());
        let model = config.model().clone();
        let task = match config {
            ConfigKind::Ask(_) => match request_type {
                RequestType::Normal => Task::GenerateCommand,
                RequestType::Auxiliary => Task::Explain,
            },
            ConfigKind::Explain(_) => Task::Explain,
        };
        let context = Context::from(config);
        let user_prompt = match request_type {
            RequestType::Normal => self.textarea.lines().join("\n"),
            RequestType::Auxiliary => self.main_response.response.clone(),
        };
        let request_task = tokio::spawn(model_stream_request(
            model.clone(),
            user_prompt,
            context.clone(),
            task,
        ));
        let mut state = RequestState::WaitRequest;

        loop {
            self.draw()?;
            match state {
                RequestState::WaitRequest => {
                    if crossterm::event::poll(Duration::from_millis(100))? {
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
                        state = RequestState::Streaming;
                        self.clear_response(request_type);
                    }
                }
                RequestState::Streaming => {
                    return self
                        .stream_response(
                            request_task.await?.map_err(|_| ModelError::Error)?,
                            request_type,
                        )
                        .await
                } // FIX: error handling
                RequestState::Cancel => return Ok(RequestState::Cancel),
                RequestState::Exit => return Ok(RequestState::Exit),
            }
            match request_type {
                RequestType::Normal => self.main_response.spin_fidget(),
                RequestType::Auxiliary => self.auxiliary_response.spin_fidget(),
            }
        }
    }

    async fn stream_response(
        &mut self,
        mut response_stream: impl Stream<Item = String> + Unpin,
        request_type: RequestType,
    ) -> Result<RequestState, Box<dyn std::error::Error>> {
        while let Some(message) = response_stream.next().await {
            // TODO: dont block on await
            self.append_message_response(&message, request_type);
            self.draw()?;
            if crossterm::event::poll(Duration::from_millis(100))? {
                match crossterm::event::read()?.into() {
                    Input {
                        key: Key::Char('c'),
                        ctrl: true,
                        ..
                    } => return Ok(RequestState::Exit),
                    Input { key: Key::Esc, .. } => return Ok(RequestState::Cancel),
                    _ => (),
                }
            }
        }
        Ok(RequestState::Streaming)
    }

    fn clear_response(&mut self, request_type: RequestType) {
        // TODO: on normal clear and make auxiliary invisible
        match request_type {
            RequestType::Normal => self
                .main_response
                .update(String::new(), ShaiProgress::Waiting),
            RequestType::Auxiliary => self
                .auxiliary_response
                .update(String::new(), ShaiProgress::Waiting),
        }
    }

    fn append_message_response(&mut self, response: &str, request_type: RequestType) {
        let old_text = match request_type {
            RequestType::Normal => &self.main_response.response,
            RequestType::Auxiliary => &self.auxiliary_response.response,
        };
        let new = format!("{}{}", old_text, response);
        match request_type {
            RequestType::Normal => self.main_response.update(new, ShaiProgress::Waiting),
            RequestType::Auxiliary => self.auxiliary_response.update(new, ShaiProgress::Waiting),
        }
    }
}
