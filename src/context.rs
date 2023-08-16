use crate::ConfigKind;
use std::{io, process::Command};

#[derive(Clone)]
pub struct Context {
    pwd: Option<String>,
    tree: Option<String>,
    environment: Option<String>,
    programs: Option<String>,
}

impl From<ConfigKind> for Context {
    fn from(value: ConfigKind) -> Self {
        match value {
            ConfigKind::Ask(config) => Self {
                pwd: config.pwd.and_then(|_| std::env::var("PWD").ok()),
                tree: config
                    .depth
                    .and_then(|depth| get_directory_tree(depth).ok()),
                environment: config.environment.as_ref().map(|env| env.join(",")),
                programs: config.programs.as_ref().map(|programs| programs.join(",")),
            },
            ConfigKind::Explain(config) => Self {
                pwd: config.pwd.and_then(|_| std::env::var("PWD").ok()),
                tree: config
                    .depth
                    .and_then(|depth| get_directory_tree(depth).ok()),
                environment: config.environment.as_ref().map(|env| env.join(",")),
                programs: None,
            },
        }
    }
}

impl From<Context> for String {
    fn from(value: Context) -> Self {
        Self::new() + &value.pwd.map_or(Self::new(), |cwd| format!("You are currently in folder: {cwd}\n"))
            + &value.tree.map_or(Self::new(), |tree|format!("The tree command run in the current folder gave this output: {tree}\n"))
            + &value.environment.map_or(Self::new(), |env| format!("The following environment variables are defined: {env}\n"))
            + &value.programs.map_or(Self::new(), |bins| format!("You have the following programs installed in the system, you should only use these programs to accomplish the <task>: {bins}\n"))
    }
}

fn get_directory_tree(depth: u32) -> Result<String, io::Error> {
    let mut command = Command::new("tree");
    let command = command.arg("-L").arg(depth.to_string());

    let output = command.output()?;

    String::from_utf8(output.stdout).map_err(|_| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Only UTF8 is currently supported",
        )
    }) // TODO: handl with OsString?
}
