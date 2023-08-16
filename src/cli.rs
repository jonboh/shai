use std::fmt::Display;
use std::fs;
use std::io::{self, StdoutLock};
use std::time::Duration;

use clap::Parser;
use lazy_static::lazy_static;
use regex::Regex;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
};
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
#[command(name = "shai")]
pub enum ShaiCLIArgs {
    Ask(AskArgs),
    Explain(ExplainArgs),
    GenerateScript(IntegrationScriptArgs),
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

#[derive(clap::ValueEnum, Clone)]
enum Shell {
    Bash,
    Zsh,
    Fish,
    Nushell,
    PowerShell,
}

#[derive(clap::Args, Clone)]
#[command(author, version, about, long_about = None)]
pub struct IntegrationScriptArgs {
    #[arg(long, value_enum)]
    shell: Shell,
}

impl From<AskArgs> for AskConfig {
    fn from(value: AskArgs) -> Self {
        let pwd = if value.pwd { Some(()) } else { None };
        let model = value.model.into();
        Self {
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
        Self {
            pwd,
            depth: value.depth,
            environment: value.environment,
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
            Shell::PowerShell => println!("{}", include_str!("../scripts/powershell_assistant.ps1"))
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
    Waiting,
    S0,
    S1,
    S2,
    S3,
}

#[derive(Clone, Copy)]
enum ShaiControls {
    // TODO: this should set the actual controls available
    Started,
    Processing,
    ExplanationGenerated,
    CommandGenerated,
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
            Self::Waiting | Self::S3 => Self::S0,
            Self::S0 => Self::S1,
            Self::S1 => Self::S2,
            Self::S2 => Self::S3,
        }
    }
}

impl Display for ShaiRequestProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Waiting => write!(f, ""),
            Self::S0 => write!(f, "-"),
            Self::S1 => write!(f, "\\"),
            Self::S2 => write!(f, "|"),
            Self::S3 => write!(f, "/"),
        }
    }
}

struct ModelWindow<'t> {
    pub response: String,
    pub paragraph: Paragraph<'t>,
    fidget: ShaiRequestProgress,
}

impl ModelWindow<'_> {
    fn update(&mut self, new: String, fidget: ShaiRequestProgress) {
        self.response = new.clone();
        self.paragraph = create_explanation_paragraph(new, fidget);
    }

    fn spin_fidget(&mut self) {
        self.fidget = self.fidget.next_state();
        self.update(self.response.clone(), self.fidget);
    }
}

fn create_explanation_paragraph<'t>(text: String, thinking: ShaiRequestProgress) -> Paragraph<'t> {
    let title = format!("Shai {thinking}");
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

fn create_controls_paragraph<'t>(state: ShaiControls) -> Paragraph<'t> {
    let text = match state {
        ShaiControls::Started | ShaiControls::ExplanationGenerated =>  "<C-c>: Exit | Enter: Send Prompt".to_string(),
        ShaiControls::Processing => "<C-c>: Exit | Esc: Cancel ".to_string(),
        ShaiControls::CommandGenerated => "<C-c>: Exit | Enter: Send Prompt | <C-a>: Accept | <C-A>: Accept (raw) | <C-e>: Explain".to_string(),
    };
    Paragraph::new(text)
        .block(Block::default().borders(Borders::TOP))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
}

pub struct ShaiUI<'t> {
    args: ShaiArgs,
    term: Terminal<CrosstermBackend<StdoutLock<'t>>>,
    layout: Layout,
    input_text: Paragraph<'t>,
    input: Input,
    main_response: ModelWindow<'t>,
    auxiliary_response: ModelWindow<'t>,
    controls: Paragraph<'t>,
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

impl Layout {
    fn create(&self) -> ratatui::layout::Layout {
        match self {
            Self::InputResponse => ratatui::layout::Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(10),
                        Constraint::Length(2),
                    ]
                    .as_ref(),
                ),
            Self::InputResponseExplanation => ratatui::layout::Layout::default()
                .direction(Direction::Vertical)
                // .constraints([Constraint::Length(2), Constraint::Min(20), Constraint::Min(20)].as_ref())
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(10),
                    Constraint::Min(10),
                    Constraint::Length(2),
                ]),
        }
    }
}

impl<'t> ShaiUI<'t> {
    /// This function initializes Shai and eases disabling terminal raw mode in all circumstances
    fn initialization(args: ShaiArgs) -> Result<Self, Box<dyn std::error::Error>> {
        let mut stdout = io::stdout().lock();
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;

        let textarea = create_input_paragraph(String::new(), Self::title(&args));
        let input = Input::default();
        let main_response = ModelWindow {
            response: String::new(),
            paragraph: create_explanation_paragraph(String::new(), ShaiRequestProgress::Waiting),
            fidget: ShaiRequestProgress::Waiting,
        };
        let auxiliary_response = ModelWindow {
            response: String::new(),
            paragraph: create_explanation_paragraph(String::new(), ShaiRequestProgress::Waiting),
            fidget: ShaiRequestProgress::Waiting,
        };
        let controls = create_controls_paragraph(ShaiControls::Started);

        let layout = Layout::InputResponse;
        Ok(ShaiUI {
            args,
            term,
            layout,
            input_text: textarea,
            input,
            main_response,
            auxiliary_response,
            controls,
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
        let cli_text = self
            .args
            .edit_file()
            .as_ref()
            .and_then(|file| fs::read_to_string(file).ok())
            .unwrap_or_default();
        // self.textarea.insert_str(&cli_text);
        self.input_text = create_input_paragraph(cli_text, Self::title(&self.args)); // FIX
        let write_mode = self.mainloop().await;

        // restore terminal mode
        disable_raw_mode()?;
        crossterm::execute!(
            self.term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.term.show_cursor()?;

        if let ShaiArgs::Ask(_) = self.args {
            if let Some(file) = &self.args.edit_file() {
                match write_mode? {
                    WriteBuffer::Yes => {
                        let code_blocks = extract_code_blocks(&self.main_response.response);
                        if code_blocks.is_empty() {
                            // the model probably obeyed the instructions
                            fs::write(file, &self.main_response.response)?;
                        } else {
                            fs::write(file, code_blocks.join("\n"))?;
                        }
                    }
                    WriteBuffer::Raw => fs::write(file, &self.main_response.response)?,
                    WriteBuffer::No => (),
                }
            }
        }
        if self.args.write_stdout() {
            let response = &self.main_response.response;
            println!("{response}");
        }
        Ok(())
    }

    async fn mainloop(&mut self) -> Result<WriteBuffer, Box<dyn std::error::Error>> {
        loop {
            let controls = match self.args {
                ShaiArgs::Ask(_) => {
                    if self.main_response.response.is_empty() {
                        ShaiControls::Started
                    } else {
                        ShaiControls::CommandGenerated
                    }
                }
                ShaiArgs::Explain(_) => {
                    if self.main_response.response.is_empty() {
                        ShaiControls::Started
                    } else {
                        ShaiControls::ExplanationGenerated
                    }
                }
            };
            self.update_controls(controls);

            self.draw()?;

            if let Event::Key(key) = crossterm::event::read()? {
                match key {
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } => return Ok(WriteBuffer::No),
                    KeyEvent {
                        code: KeyCode::Char('a'),
                        modifiers: KeyModifiers::ALT, // FIX: CTRL+SHIFT
                        ..
                    } if matches!(controls, ShaiControls::CommandGenerated) => {
                        return Ok(WriteBuffer::Raw)
                    }
                    KeyEvent {
                        code: KeyCode::Char('a'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } if matches!(controls, ShaiControls::CommandGenerated) => {
                        return Ok(WriteBuffer::Yes)
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
                        if matches!(self.send_request(RequestType::Normal).await?, RequestExit::Exit) {
                            return Ok(WriteBuffer::No);
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Char('e'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    } if matches!(controls, ShaiControls::CommandGenerated) => {
                        self.layout = Layout::InputResponseExplanation;
                        if matches!(self.send_request(RequestType::Auxiliary).await?, RequestExit::Exit)
                        {
                            return Ok(WriteBuffer::No);
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } => (),
                    _ => {
                        // self.textarea.input(input);
                        self.input.handle_event(&Event::Key(key));
                        self.input_text = create_input_paragraph(
                            self.input.value().to_string(),
                            Self::title(&self.args),
                        );
                    }
                }
            }
        }
    }

    fn draw(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.term.draw(|f| {
            let layout = self.layout.create();
            let chunks = layout.split(f.size());
            let width = chunks[0].width.max(3) - 3; // keep 2 for borders and 1 for cursor
            let scroll = self.input.visual_scroll(width as usize);
            f.render_widget(
                self.input_text.clone().scroll((0, u16::try_from(scroll).unwrap_or_default())),
                chunks[0],
            );
            f.set_cursor(
                chunks[0].x + u16::try_from(self.input.visual_cursor().max(scroll) - scroll).unwrap_or_default() + 1,
                chunks[0].y + 1,
            );
            match &self.layout {
                Layout::InputResponse => {
                    f.render_widget(self.main_response.paragraph.clone(), chunks[1]);
                    f.render_widget(self.controls.clone(), chunks[2]);
                }
                Layout::InputResponseExplanation => {
                    f.render_widget(self.main_response.paragraph.clone(), chunks[1]);
                    f.render_widget(self.auxiliary_response.paragraph.clone(), chunks[2]);
                    f.render_widget(self.controls.clone(), chunks[3]);
                }
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
            self.update_controls(ShaiControls::Processing);
            self.draw()?;
            match state {
                RequestState::WaitRequest => {
                    if crossterm::event::poll(Duration::from_millis(100))? {
                        if let Event::Key(key) = crossterm::event::read()? {
                            match key {
                                KeyEvent {
                                    code: KeyCode::Esc, ..
                                } => return Ok(RequestExit::Cancel),
                                KeyEvent {
                                    code: KeyCode::Char('c'),
                                    modifiers: KeyModifiers::CONTROL,
                                    ..
                                } => return Ok(RequestExit::Exit),
                                _ => (),
                            }
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
                            request_task
                                .await?
                                .map_err(|err| ModelError::Error(Box::new(err)))?
                                .map(|each| each.map_err(|err| ModelError::Error(Box::new(err)))),
                            request_type,
                        )
                        .await
                }
            }
            match request_type {
                RequestType::Normal => self.main_response.spin_fidget(),
                RequestType::Auxiliary => self.auxiliary_response.spin_fidget(),
            }
        }
    }

    async fn stream_response(
        &mut self,
        mut response_stream: impl Stream<Item = Result<String, ModelError>> + Unpin,
        request_type: RequestType,
    ) -> Result<RequestExit, Box<dyn std::error::Error>> {
        self.update_controls(ShaiControls::Processing);
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
        }
        Ok(RequestExit::Finished)
    }

    fn clear_response(&mut self, request_type: RequestType) {
        match request_type {
            RequestType::Normal => {
                self.layout = Layout::InputResponse;
                self.main_response
                    .update(String::new(), ShaiRequestProgress::Waiting);
            }
            RequestType::Auxiliary => self
                .auxiliary_response
                .update(String::new(), ShaiRequestProgress::Waiting),
        }
    }

    fn update_controls(&mut self, controls: ShaiControls) {
        self.controls = create_controls_paragraph(controls);
    }

    fn append_message_response(&mut self, response: &str, request_type: RequestType) {
        let old_text = match request_type {
            RequestType::Normal => &self.main_response.response,
            RequestType::Auxiliary => &self.auxiliary_response.response,
        };
        let new = format!("{old_text}{response}");
        match request_type {
            RequestType::Normal => self.main_response.update(new, ShaiRequestProgress::Waiting),
            RequestType::Auxiliary => self
                .auxiliary_response
                .update(new, ShaiRequestProgress::Waiting),
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
