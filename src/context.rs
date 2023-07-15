use std::{io, process::Command};
use crate::Config;

pub (crate) struct Context {
    pwd: Option<String>,
    tree: Option<String>,
    environment: Option<String>,
    programs: Option<String>,
}

impl Context {
    pub(crate) fn new(config: &Config) -> Context {
        Context {
            pwd: config.pwd.and_then(|_| std::env::var("PWD").ok()),
            tree: config
                .depth
                .and_then(|depth| get_directory_tree(depth).ok()),
            environment: config.environment.as_ref().map(|env| env.join(",")),
            programs: config.programs.as_ref().map(|programs| programs.join(",")),
        }
    }
}

impl From<Context> for String {
    fn from(value: Context) -> Self {
        "".to_owned() + &value.pwd.map(|cwd| format!("You are currently in folder: {cwd}\n")).unwrap_or("".to_string())
            + &value.tree.map(|tree|format!("The tree command run in the current folder gave this output: {tree}\n")).unwrap_or("".to_string())
            + &value.environment.map(|env| format!("The following environment variables are defined: {env}\n")).unwrap_or("".to_string())
            + &value.programs.map(|bins| format!("You have the following programs installed in the system, you should only use these programs to accomplish the <task>: {bins}\n")).unwrap_or("".to_string())
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
