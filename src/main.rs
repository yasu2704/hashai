use std::process::ExitCode as ProcessExitCode;

use clap::{Parser, error::ErrorKind};
use hashai::{
    HashaiError,
    cli::{Cli, Command},
    config::{ConfigOverrides, ConfigSources},
    platform,
    prompt::{EnvironmentInfo, PromptInput, build_prompt},
};

fn main() -> ProcessExitCode {
    match run() {
        Ok(()) => ProcessExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hashai: {error}");
            ProcessExitCode::from(error.exit_code() as u8)
        }
    }
}

fn run() -> Result<(), HashaiError> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return Ok(());
        }
        Err(error) => return Err(HashaiError::ArgumentOrConfig(error.to_string())),
    };
    let Command::Generate(args) = cli.command;
    if args.request.trim().is_empty() {
        return Err(HashaiError::ArgumentOrConfig(
            "request must not be empty".to_owned(),
        ));
    }

    let config = ConfigSources::from_system(ConfigOverrides {
        shell: args.shell,
        model: args.model,
        reasoning_effort: args.reasoning_effort,
        timeout_seconds: args.timeout_seconds,
        ..ConfigOverrides::default()
    })?;
    let shell = config
        .shell
        .resolve(std::env::var("SHELL").ok().as_deref())?;
    let environment = detected_environment();
    platform::validate(&environment.os, shell.as_str())?;
    let current_dir = std::env::current_dir()?;
    let _prompt = build_prompt(&PromptInput {
        request: args.request,
        shell,
        current_dir,
        environment,
        extra_instructions: config.prompt.extra_instructions,
    });

    // Codex process execution belongs to Phase 1B.  This phase establishes the
    // parse/config/prompt boundary without executing a command or printing one.
    Ok(())
}

fn detected_environment() -> EnvironmentInfo {
    EnvironmentInfo {
        os: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        distribution_or_version: detected_os_detail(),
    }
}

#[cfg(target_os = "linux")]
fn detected_os_detail() -> Option<String> {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_owned())
            })
        })
}

#[cfg(target_os = "macos")]
fn detected_os_detail() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detected_os_detail() -> Option<String> {
    None
}
