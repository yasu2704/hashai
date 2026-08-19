use std::path::PathBuf;

use crate::config::Shell;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentInfo {
    pub os: String,
    pub architecture: String,
    pub distribution_or_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptInput {
    pub request: String,
    pub shell: Shell,
    pub current_dir: PathBuf,
    pub environment: EnvironmentInfo,
    pub extra_instructions: Option<String>,
}

pub fn build_prompt(input: &PromptInput) -> String {
    let os_detail = input
        .environment
        .distribution_or_version
        .as_deref()
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    let fish_rule = if input.shell == Shell::Fish {
        "Do not use Bash syntax for Fish."
    } else {
        "Use syntax valid for the target shell."
    };
    let extra = input
        .extra_instructions
        .as_deref()
        .map(|value| format!("\nUser generation preference: {value}"))
        .unwrap_or_default();

    format!(
        "You generate one shell command from a single user request.\n\
Do not execute commands. Return only data that conforms to the supplied JSON schema.\n\
Target operating system: {}{}\n\
Target architecture: {}\n\
Target shell: {}\n\
Current working directory: {}\n\
Do not use sudo unless explicitly necessary.\n\
Do not invent ambiguous values; prefer an interactive command when needed.\n\
Use && only when operation order is required.\n\
{}{}\n\
User request: {}",
        input.environment.os,
        os_detail,
        input.environment.architecture,
        input.shell.as_str(),
        input.current_dir.display(),
        fish_rule,
        extra,
        input.request
    )
}
