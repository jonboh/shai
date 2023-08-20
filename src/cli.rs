use std::fmt::Display;
use std::fs;
use std::io::{self, StdoutLock};
use std::time::Duration;

use clap::Parser;
use lazy_static::lazy_static;
use regex::Regex;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::{Stream, StreamExt};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::context::Context;
use crate::model::Task;
use crate::openai::OpenAIGPTModel;
use crate::{model_stream_request, AskConfig, ConfigKind, ExplainConfig, ModelError, ModelKind};

#[derive(Parser, Clone)]
#[command(author, version, about, long_about = None)]
pub enum ShaiCLIArgs {
    /// Generate a command
    #[command(arg_required_else_help = true)]
    Ask(AskArgs),
    /// Explain a command
    #[command(arg_required_else_help = true)]
    Explain(ExplainArgs),
    /// Write to stdout the shell integration code for the provided shell
    #[command(arg_required_else_help = true)]
    GenerateScript(IntegrationScriptArgs),
}

#[derive(clap::Args, Clone)]
#[command(author, about, long_about = None)]
pub struct AskArgs {
    /// Tell the model which OS should be assumed. Distro names are also valid.
    #[arg(long, default_value = "Linux")]
    operating_system: String,

    /// Add the name of a defined environment variable. Repeat to list several items
    #[arg(long, short, default_value = None)]
    environment: Option<Vec<String>>,

    /// Add a program to the list of available programs. Repeat to list several items.
    /// If unset the model is free to use any program
    #[arg(long, short, default_value = None)]
    program: Option<Vec<String>>,

    /// Provide the model with the current working directory.
    /// If unset the model does not get any information about what the current directory is.
    #[arg(long)]
    cwd: bool,

    /// Provide the model with the output of the tree command with this depth.
    /// If unset the model does not get any information about the contents of the current
    /// directory
    #[arg(long, default_value=None)]
    depth: Option<u32>,

    #[arg(long, value_enum)]
    model: ArgModelKind,

    /// Write output to stdout
    #[arg(long)]
    write_stdout: bool,

    /// Edit file from which to retrieve the state of ther buffer line and to which to write the
    /// model response
    #[arg(long)]
    edit_file: Option<std::path::PathBuf>,
}

#[derive(clap::Args, Clone)]
#[command(author, about, long_about = None)]
pub struct ExplainArgs {
    /// Tell the model which OS should be assumed. Distro names are also valid.
    #[arg(long, default_value = "Linux")]
    operating_system: String,

    /// Add the name of a defined environment variable. Repeat to list several items
    #[arg(long, default_value = None)]
    environment: Option<Vec<String>>,
    ///
    /// Provide the model with the current working directory.
    /// If unset the model does not get any information about what the current directory is
    #[arg(long)]
    cwd: bool,

    /// Provide the model with the output of the tree command with this depth.
    /// If unset the model does not get any information about the contents of the current
    /// directory
    #[arg(long, default_value=None)]
    depth: Option<u32>,

    #[arg(long, value_enum)]
    model: ArgModelKind,

    /// Write output to stdout
    #[arg(long)]
    write_stdout: bool,

    /// Edit file from which to retrieve the state of ther buffer line
    #[arg(long)]
    edit_file: Option<std::path::PathBuf>,
}

#[derive(clap::Args, Clone)]
#[command(author, about, long_about = None)]
pub struct IntegrationScriptArgs {
    #[arg(long, value_enum)]
    shell: Shell,
}

#[derive(Clone)]
pub enum ShaiArgs {
    Ask(AskArgs),
    Explain(ExplainArgs),
}

impl ShaiArgs {
    const fn edit_file(&self) -> &Option<std::path::PathBuf> {
        match self {
            Self::Ask(args) => &args.edit_file,
            Self::Explain(args) => &args.edit_file,
        }
    }
    const fn write_stdout(&self) -> bool {
        match self {
            Self::Ask(args) => args.write_stdout,
            Self::Explain(args) => args.write_stdout,
        }
    }
}

impl From<ShaiArgs> for ConfigKind {
    fn from(value: ShaiArgs) -> Self {
        match value {
            ShaiArgs::Ask(args) => Self::Ask(AskConfig::from(args)),
            ShaiArgs::Explain(args) => Self::Explain(ExplainConfig::from(args)),
        }
    }
}

#[derive(clap::ValueEnum, Clone)]
#[allow(non_camel_case_types)]
enum ArgModelKind {
    OpenAIGPT35Turbo,
    OpenAIGPT35Turbo_16k,
    OpenAIGPT4,
    OpenAIGPT4_32k,
}

impl From<ArgModelKind> for ModelKind {
    fn from(value: ArgModelKind) -> Self {
        match value {
            ArgModelKind::OpenAIGPT35Turbo => Self::OpenAIGPT(OpenAIGPTModel::GPT35Turbo),
            ArgModelKind::OpenAIGPT35Turbo_16k => Self::OpenAIGPT(OpenAIGPTModel::GPT35Turbo_16k),
            ArgModelKind::OpenAIGPT4 => Self::OpenAIGPT(OpenAIGPTModel::GPT4),
            ArgModelKind::OpenAIGPT4_32k => Self::OpenAIGPT(OpenAIGPTModel::GPT4_32k),
        }
    }
}

#[derive(clap::ValueEnum, Clone)]
enum Shell {
    Bash,
    Zsh,
    Fish,
    Nushell,
    PowerShell,
}

impl From<AskArgs> for AskConfig {
    fn from(value: AskArgs) -> Self {
        let cwd = if value.cwd { Some(()) } else { None };
        let model = value.model.into();
        Self {
            operating_system: value.operating_system,
            environment: value.environment,
            programs: value.program,
            cwd,
            depth: value.depth,
            model,
        }
    }
}

impl From<ExplainArgs> for ExplainConfig {
    fn from(value: ExplainArgs) -> Self {
        let cwd = if value.cwd { Some(()) } else { None };
        let model = value.model.into();
        Self {
            operating_system: value.operating_system,
            environment: value.environment,
            cwd,
            depth: value.depth,
            model,
        }
    }
}

#[allow(clippy::missing_errors_doc)]
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = ShaiCLIArgs::parse();
    match args {
        ShaiCLIArgs::Ask(shai_args) => {
            let mut ui = ShaiUI::new(ShaiArgs::Ask(shai_args))?;
            ui.run().await?;
        }
        ShaiCLIArgs::Explain(shai_args) => {
            let mut ui = ShaiUI::new(ShaiArgs::Explain(shai_args))?;
            ui.run().await?;
        }
        ShaiCLIArgs::GenerateScript(integration_args) => match integration_args.shell {
            Shell::Bash => println!("{}", include_str!("../scripts//bash_assistant.sh")),
            Shell::Zsh => println!("{}", include_str!("../scripts/zsh_assistant.zsh")),
            Shell::Fish => println!("{}", include_str!("../scripts/fish_assistant.fish")),
            Shell::Nushell => println!("{}", include_str!("../scripts/nushell_assistant.nu")),
            Shell::PowerShell => {
                println!("{}", include_str!("../scripts/powershell_assistant.ps1"));
            }
        },
    }
    Ok(())
}

enum WriteBuffer {
    Yes,
    Raw,
    No,
}

enum RequestState {
    WaitRequest,
    Streaming,
}

enum RequestExit {
    Cancel,
    Exit,
    Finished,
}

#[derive(Copy, Clone)]
enum ShaiRequestProgress {
    None,
    S0,
    S1,
    S2,
    S3,
}

impl Default for ShaiRequestProgress {
    fn default() -> Self {
        ShaiRequestProgress::None
    }
}

#[derive(Clone, Copy)]
enum ShaiState {
    // TODO: this should set the actual controls available
    Started,
    Processing,
    ExplanationGenerated,
    CommandGenerated,
    AuxExplanationGenerated,
}

#[derive(Clone, Copy)]
enum RequestType {
    // stdin -> main_response
    Normal,
    // main_response(command) -> auxiliary_response
    Auxiliary,
}

impl ShaiRequestProgress {
    const fn next_state(self) -> Self {
        match self {
            Self::None | Self::S3 => Self::S0,
            Self::S0 => Self::S1,
            Self::S1 => Self::S2,
            Self::S2 => Self::S3,
        }
    }
}

impl Display for ShaiRequestProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, ""),
            Self::S0 => write!(f, "-"),
            Self::S1 => write!(f, "\\"),
            Self::S2 => write!(f, "|"),
            Self::S3 => write!(f, "/"),
        }
    }
}

fn create_explanation_paragraph<'t>(
    text: String,
    thinking: ShaiRequestProgress,
    focus: bool,
) -> Paragraph<'t> {
    let focus_indicator = if focus { "*" } else { "" };
    let title = format!("Shai {thinking} {focus_indicator}");
    Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(title))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
}

fn create_input_paragraph<'t>(text: String, title: String) -> Paragraph<'t> {
    Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(title))
        .alignment(Alignment::Left)
}

fn create_controls_paragraph<'t>(state: ShaiState) -> Paragraph<'t> {
    let text = match state {
        ShaiState::Started=>  "<C-c>: Exit | Enter: Send Prompt".to_string(),
        ShaiState::Processing => "<C-c>: Exit | Esc: Cancel ".to_string(),
        ShaiState::ExplanationGenerated => "<C-c>: Exit | Enter: Send Prompt | <C-u|d>: Scroll".to_string(),
        ShaiState::CommandGenerated => "<C-c>: Exit | Enter: Send Prompt | <C-a>: Accept | <C-A>: Accept (raw) | <C-e>: Explain".to_string(),
        ShaiState::AuxExplanationGenerated =>"<C-c>: Exit | Enter: Send Prompt | <C-a>: Accept | <C-A>: Accept (raw) | <C-e>: Explain | <Tab>: Toggle Focus | <C-u|d>: Scroll | <S-Up|Down>: Resize explanation".to_string(),
    };
    Paragraph::new(text)
        .block(Block::default().borders(Borders::TOP))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
}

struct Response {
    text: String,
    scroll: u16,
    request_state: ShaiRequestProgress,
}

impl Default for Response {
    fn default() -> Self {
        Self { text: Default::default(), scroll: Default::default(), request_state: Default::default() }
    }
}

pub struct ShaiUI<'t> {
    args: ShaiArgs,
    term: Terminal<CrosstermBackend<StdoutLock<'t>>>,
    layout: Layout,
    input_text: String,
    input: Input,
    main_response: Response,
    auxiliary_response: Response,
    main_response_size: u16,
    response_focus: Focus,
}

fn extract_code_blocks(text: &str) -> Vec<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"(?s)```(?:\w+)?\n(.*?)\n```")
            .expect("The regex expression should be valid");
    }

    let mut code_blocks = Vec::new();
    for capture in RE.captures_iter(text) {
        if let Some(code_block) = capture.get(1) {
            code_blocks.push(code_block.as_str().to_string());
        }
    }
    code_blocks
}

enum Layout {
    InputResponse,
    InputResponseExplanation,
}

enum Focus {
    MainResponse,
    AuxiliaryResponse,
}

impl Layout {
    fn create(&self, main_response_size: u16) -> ratatui::layout::Layout {
        match self {
            Self::InputResponse => ratatui::layout::Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(main_response_size),
                        Constraint::Length(2),
                    ]
                    .as_ref(),
                ),
            Self::InputResponseExplanation => ratatui::layout::Layout::default()
                .direction(Direction::Vertical)
                // .constraints([Constraint::Length(2), Constraint::Min(20), Constraint::Min(20)].as_ref())
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(main_response_size),
                    Constraint::Min(3),
                    Constraint::Length(2),
                ]),
        }
    }
}

impl<'t> ShaiUI<'t> {
    /// This function initializes Shai and eases disabling terminal raw mode in all circumstances
    fn initialization(args: ShaiArgs) -> Result<Self, Box<dyn std::error::Error>> {
        let mut stdout = io::stdout().lock();
        crossterm::execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;

        let cli_text = args
            .edit_file()
            .as_ref()
            .and_then(|file| fs::read_to_string(file).ok())
            .map(|bufstr| bufstr.trim().to_string())
            .unwrap_or_default();


        Ok(ShaiUI {
            args,
            term,
            layout: Layout::InputResponse,
            input_text: cli_text.clone(),
            input: Input::default().with_value(cli_text),
            main_response: Default::default(),
            auxiliary_response: Default::default(),
            main_response_size: 3,
            response_focus: Focus::MainResponse,
        })
    }

    fn new(args: ShaiArgs) -> Result<Self, Box<dyn std::error::Error>> {
        enable_raw_mode().expect("Terminal needs to be set in raw mode for Shai UI to work");
        match Self::initialization(args) {
            Ok(shai) => Ok(shai),
            Err(err) => {
                disable_raw_mode()?;
                Err(err)
            }
        }
    }

    async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let write_mode = self.mainloop().await;

        // restore terminal mode
        disable_raw_mode()?;
        crossterm::execute!(self.term.backend_mut(), LeaveAlternateScreen,)?;
        self.term.show_cursor()?;

        if let ShaiArgs::Ask(_) = self.args {
            if let Some(file) = &self.args.edit_file() {
                match write_mode? {
                    WriteBuffer::Yes => {
                        let code_blocks = extract_code_blocks(&self.main_response.text);
                        if code_blocks.is_empty() {
                            // the model probably obeyed the instructions
                            fs::write(file, &self.main_response.text)?;
                        } else {
                            fs::write(file, code_blocks.join("\n"))?;
                        }
                    }
                    WriteBuffer::Raw => fs::write(file, &self.main_response.text)?,
                    WriteBuffer::No => (),
                }
            }
        }
        if self.args.write_stdout() {
            let response = &self.main_response.text;
            println!("{response}");
        }
        Ok(())
    }

    fn state(&self) -> ShaiState {
        match (self.main_response.request_state, self.auxiliary_response.request_state) {
            (ShaiRequestProgress::None, ShaiRequestProgress::None) => match self.args {
                ShaiArgs::Ask(_) => {
                    if self.main_response.text.is_empty() {
                        ShaiState::Started
                    } else if self.auxiliary_response.text.is_empty() {
                        ShaiState::CommandGenerated
                    } else {
                        ShaiState::AuxExplanationGenerated
                    }
                }
                ShaiArgs::Explain(_) => {
                    if self.main_response.text.is_empty() {
                        ShaiState::Started
                    } else {
                        ShaiState::ExplanationGenerated
                    }
                }
            },
            _ => ShaiState::Processing,
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn mainloop(&mut self) -> Result<WriteBuffer, Box<dyn std::error::Error>> {
        loop {
            self.draw()?;

            if let Event::Key(key) = crossterm::event::read()? {
                match key {
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } => return Ok(WriteBuffer::No),
                    KeyEvent {
                        code: KeyCode::Char('r'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } if matches!(
                        self.state(),
                        ShaiState::CommandGenerated | ShaiState::AuxExplanationGenerated
                    ) =>
                    {
                        return Ok(WriteBuffer::Raw)
                    }
                    KeyEvent {
                        code: KeyCode::Char('a'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } if matches!(
                        self.state(),
                        ShaiState::CommandGenerated | ShaiState::AuxExplanationGenerated
                    ) =>
                    {
                        return Ok(WriteBuffer::Yes)
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
                        if matches!(
                            self.send_request(RequestType::Normal).await?,
                            RequestExit::Exit
                        ) {
                            return Ok(WriteBuffer::No);
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char('e'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } if matches!(self.state(), ShaiState::CommandGenerated) => {
                        self.layout = Layout::InputResponseExplanation;
                        self.response_focus = Focus::AuxiliaryResponse;
                        if matches!(
                            self.send_request(RequestType::Auxiliary).await?,
                            RequestExit::Exit
                        ) {
                            return Ok(WriteBuffer::No);
                        }
                    }
                    // scroll explanation
                    KeyEvent {
                        code: dirchar @ KeyCode::Char('d' | 'u'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } if matches!(
                        self.state(),
                        ShaiState::ExplanationGenerated | ShaiState::AuxExplanationGenerated
                    ) =>
                    {
                        // NOTE: this doesnt take into account the width of the terminal.
                        let page = u16::try_from(
                            match self.response_focus {
                                Focus::MainResponse => &self.main_response.text,
                                Focus::AuxiliaryResponse => &self.auxiliary_response.text,
                            }
                            .lines()
                            .count(),
                        )?;
                        let half_page = (page / 2).max(1);
                        match self.response_focus {
                            Focus::MainResponse => {
                                if dirchar == KeyCode::Char('d') {
                                    self.main_response.scroll =
                                        (self.main_response.scroll + half_page).min(page);
                                } else {
                                    self.main_response.scroll =
                                        self.main_response.scroll.saturating_sub(half_page);
                                }
                            }
                            Focus::AuxiliaryResponse => {
                                if dirchar == KeyCode::Char('d') {
                                    self.auxiliary_response.scroll =
                                        (self.auxiliary_response.scroll + half_page).min(page);
                                } else {
                                    self.auxiliary_response.scroll =
                                        self.auxiliary_response.scroll.saturating_sub(half_page);
                                }
                            }
                        }
                    }
                    // resize
                    KeyEvent {
                        code: dirchar @ (KeyCode::Up | KeyCode::Down),
                        modifiers: KeyModifiers::SHIFT,
                        ..
                    } if matches!(self.state(), ShaiState::AuxExplanationGenerated) => {
                        if dirchar == KeyCode::Up {
                            self.main_response_size = self.main_response_size.saturating_sub(1).max(3);
                        } else {
                            self.main_response_size += 1; // NOTE: would be better to saturate at
                                                          // max height
                        }
                    }
                    // toggle focus
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } if matches!(self.layout, Layout::InputResponseExplanation) => {
                        self.response_focus = match self.response_focus {
                            Focus::MainResponse => Focus::AuxiliaryResponse,
                            Focus::AuxiliaryResponse => Focus::MainResponse,
                        }
                    }
                    _ => {
                        self.input.handle_event(&Event::Key(key));
                        self.input_text = self.input.value().to_string();
                    }
                }
            }
        }
    }

    fn draw(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.state();
        self.term.draw(|f| {
            let layout = self.layout.create(self.main_response_size);
            let chunks = layout.split(f.size());
            let width = chunks[0].width.max(3) - 3; // keep 2 for borders and 1 for cursor
            let scroll = self.input.visual_scroll(width as usize);
            f.render_widget(
                create_input_paragraph(self.input_text.clone(), Self::title(&self.args))
                    .scroll((0, u16::try_from(scroll).unwrap_or_default())),
                chunks[0],
            );
            f.set_cursor(
                chunks[0].x
                    + u16::try_from(self.input.visual_cursor().max(scroll) - scroll)
                        .unwrap_or_default()
                    + 1,
                chunks[0].y + 1,
            );
            f.render_widget(
                create_explanation_paragraph(
                    self.main_response.text.clone(),
                    self.main_response.request_state,
                    matches!(self.response_focus, Focus::MainResponse),
                )
                .scroll((self.main_response.scroll, 0)),
                chunks[1],
            );
            match &self.layout {
                Layout::InputResponse => {
                    f.render_widget(create_controls_paragraph(state), chunks[2]);
                }
                Layout::InputResponseExplanation => {
                    f.render_widget(
                        create_explanation_paragraph(
                            self.auxiliary_response.text.clone(),
                            self.auxiliary_response.request_state,
                            matches!(self.response_focus, Focus::AuxiliaryResponse),
                        )
                        .scroll((self.auxiliary_response.scroll, 0)),
                        chunks[2],
                    );
                    f.render_widget(create_controls_paragraph(state), chunks[3]);
                }
            }
        })?;
        Ok(())
    }

    fn update_request_state(&mut self, request_type: RequestType, finished: bool) {
        if finished {
            match request_type {
                RequestType::Normal => {
                    self.main_response.request_state = ShaiRequestProgress::None;
                }
                RequestType::Auxiliary => {
                    self.auxiliary_response.request_state = ShaiRequestProgress::None;
                }
            }
        } else {
            match request_type {
                RequestType::Normal => {
                    self.main_response.request_state = self.main_response.request_state.next_state()
                }
                RequestType::Auxiliary => {
                    self.auxiliary_response.request_state = self.auxiliary_response.request_state.next_state()
                }
            }
        }
    }

    // Source = {stdin, main_response}
    // Destination = {main_response, auxiliary_response}
    async fn send_request(
        &mut self,
        request_type: RequestType,
    ) -> Result<RequestExit, Box<dyn std::error::Error>> {
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
            RequestType::Normal => self.input.value().to_string(),
            RequestType::Auxiliary => self.main_response.text.clone(),
        };
        let request_task = tokio::spawn(model_stream_request(
            model.clone(),
            user_prompt,
            context.clone(),
            task,
        ));
        let mut reqstate = RequestState::WaitRequest;

        let ret = loop {
            self.draw()?;
            match reqstate {
                RequestState::WaitRequest => {
                    if crossterm::event::poll(Duration::from_millis(100))? {
                        if let Event::Key(key) = crossterm::event::read()? {
                            match key {
                                KeyEvent {
                                    code: KeyCode::Esc, ..
                                } => break Ok(RequestExit::Cancel),
                                KeyEvent {
                                    code: KeyCode::Char('c'),
                                    modifiers: KeyModifiers::CONTROL,
                                    ..
                                } => break Ok(RequestExit::Exit),
                                _ => (),
                            }
                        }
                    }
                    if request_task.is_finished() {
                        reqstate = RequestState::Streaming;
                        self.clear_response(request_type);
                    }
                }
                RequestState::Streaming => {
                    break self
                        .stream_response(
                            request_task
                                .await?
                                .map_err(|err| ModelError::Error(Box::new(err)))?
                                .map(|each| each.map_err(|err| ModelError::Error(Box::new(err)))),
                            request_type,
                        )
                        .await
                }
            }
            self.update_request_state(request_type, false);
        };
        self.update_request_state(request_type, true);
        ret
    }

    async fn stream_response(
        &mut self,
        mut response_stream: impl Stream<Item = Result<String, ModelError>> + Unpin,
        request_type: RequestType,
    ) -> Result<RequestExit, Box<dyn std::error::Error>> {
        while let Some(message) = response_stream.next().await {
            // TODO: dont block on await
            self.append_message_response(&message?, request_type);
            self.draw()?;
            if crossterm::event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = crossterm::event::read()? {
                    match key {
                        KeyEvent {
                            code: KeyCode::Char('c'),
                            modifiers: KeyModifiers::CONTROL,
                            ..
                        } => return Ok(RequestExit::Exit),
                        KeyEvent {
                            code: KeyCode::Esc, ..
                        } => return Ok(RequestExit::Cancel),
                        _ => (),
                    }
                }
            }
            self.update_request_state(request_type, false)
        }
        Ok(RequestExit::Finished)
    }

    fn clear_response(&mut self, request_type: RequestType) {
        match request_type {
            RequestType::Normal => {
                self.layout = Layout::InputResponse;
                self.response_focus = Focus::MainResponse;
                self.main_response = Default::default();
                self.auxiliary_response = Default::default();
            }
            RequestType::Auxiliary => {
                self.auxiliary_response = Default::default();
            }
        }
    }

    fn append_message_response(&mut self, response: &str, request_type: RequestType) {
        let old_text = match request_type {
            RequestType::Normal => &self.main_response.text,
            RequestType::Auxiliary => &self.auxiliary_response.text,
        };
        let new = format!("{old_text}{response}");
        match request_type {
            RequestType::Normal => self.main_response.text = new,
            RequestType::Auxiliary => self.auxiliary_response.text = new,
        }
    }

    fn title(args: &ShaiArgs) -> String {
        match args {
            ShaiArgs::Ask(_) => "What should shai's command do?",
            ShaiArgs::Explain(_) => "What command should shai explain?",
        }
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::extract_code_blocks;

    #[test]
    fn code_blocks_regex() {
        let code_rust = "fn main() {
    println!(\"Hello, World!\");
}";
        let code_no_tag = "
Hello my friend";

        let code_python = "
print('Hello, World!')



        ";
        let text = format!(
            "
Some text before the code block
```rust
{code_rust}
```



```
{code_no_tag}
```
Some text after the code block
```python
{code_python}
```
    "
        );
        let blocks = extract_code_blocks(&text);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0], code_rust);
        assert_eq!(blocks[1], code_no_tag);
        assert_eq!(blocks[2], code_python);
    }
}
